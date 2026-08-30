//! User administration: create, read, edit, delete, lock, unlock and force a password reset.
//!
//! This is the admin console's first page and the **audited write path every later
//! configuration change reuses**. Three things about that path are settled here rather than
//! per operation:
//!
//! - **The write and its audit entry commit together.** One transaction opened by the
//!   handler, both written through it, one commit ([ADR-0038]) — which is why Audit is not a
//!   module somebody could forget to call ([ADR-0060]).
//! - **Every write is audited with before and after plus the blast radius** (v1 §12). The
//!   radius is computed on the live side and handed in as a value ([ADR-0039]), so nothing
//!   here knows how it was worked out and nothing there knows it is being written down.
//! - **Refused writes are audited; refused reads are not** (v1 §3), and a refusal says *you
//!   may not* with the reason rather than pretending the operation is not there.
//!
//! Account lock and forced password reset live here, on the system-administration side of
//! ADR-0003's split. Forced *relinquish* is the operational one, it is conferred by the
//! `control` rung, and it is deliberately somewhere else.
//!
//! [ADR-0003]: ../../../docs/adr/0003-operational-authority-follows-the-role.md
//! [ADR-0038]: ../../../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md
//! [ADR-0039]: ../../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
//! [ADR-0060]: ../../../docs/adr/0060-a-seam-names-domain-operations.md

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};

use super::{Api, answers, name_as_it_stands};
use crate::authorisation::Caller;
use crate::configuration::{
    AdministrationRefused, AuditEntry, AuditEvent, AuditLog, BlastRadius, Change,
    ConfigurationWrite, Enrolment, NewUser, Outstanding, Snapshot, StoreError, Transaction, User,
    UserId, Users,
};
use crate::telemetry::module;

/// A user, as the console reads one.
#[derive(Serialize)]
struct Account {
    id: String,
    username: String,
    system_administration: bool,
    locked: bool,
    /// Whether they can sign in at all. A user created here has no password until an
    /// enrolment code sets one, and the console says so rather than leaving an administrator
    /// to wonder why nobody has turned up.
    enrolled: bool,
    /// When the code this user is holding stops being good, where one is outstanding.
    ///
    /// It is never the code. A code readable twice is one that was never single-use in the
    /// sense that matters, so the console is told a code is out there and no more — which is
    /// what stops an administrator issuing a second and leaving the first in somebody's
    /// hand.
    enrolment_expires_at: Option<i64>,
}

impl Account {
    fn of(user: &User, outstanding: Option<Outstanding>) -> Self {
        Self {
            id: user.id.as_str().to_owned(),
            username: user.username.clone(),
            system_administration: user.is_system_administrator,
            locked: user.is_locked,
            enrolled: user.has_password,
            enrolment_expires_at: outstanding.map(|code| code.expires_at),
        }
    }
}

impl From<&User> for Account {
    /// A user as they stand after a write to their record, which touches no enrolment code.
    ///
    /// Every write that goes through [`administer`] leaves whatever code was outstanding
    /// exactly as it was, so re-reading it would be a second query answering what the caller
    /// already knew. Issuing a code is the one act that changes it, and it answers with the
    /// code itself rather than with an account.
    fn from(user: &User) -> Self {
        Self::of(user, None)
    }
}

/// The code an administrator has just issued, shown once and never again.
#[derive(Serialize)]
struct IssuedCode {
    code: String,
    /// Milliseconds since the Unix epoch. Rendering one for a human is the console's job.
    expires_at: i64,
}

/// What the console sends to create a user.
///
/// No password: every record is created by system administration and the user sets their own
/// through an enrolment code, because VoxLoop has no mail path ([ADR-0025]).
///
/// [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
#[derive(Deserialize)]
pub(super) struct NewAccount {
    username: String,
    #[serde(default)]
    system_administration: bool,
}

/// What the console sends to edit one. Absent means *leave it alone*.
#[derive(Deserialize)]
pub(super) struct Edit {
    username: Option<String>,
    system_administration: Option<bool>,
}

/// Read every user. `SystemAdministration`. A read, so it is not audited.
pub(super) async fn list(State(api): State<Api>) -> Response {
    answers::or_unavailable(read_all(&api).await)
}

async fn read_all(api: &Api) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let read = async {
        let users = transaction.users().await?;
        let outstanding = transaction.outstanding_enrolments().await?;
        Ok::<_, StoreError>((users, outstanding))
    }
    .await;
    transaction.roll_back().await?;

    let (users, outstanding) = read?;
    let accounts: Vec<Account> = users
        .iter()
        .map(|user| Account::of(user, outstanding.get(&user.id).copied()))
        .collect();

    Ok(Json(accounts).into_response())
}

/// Read one user. `SystemAdministration`. A read, so it is not audited.
pub(super) async fn read(State(api): State<Api>, Path(id): Path<String>) -> Response {
    answers::or_unavailable(read_one(&api, &UserId::presented(id)).await)
}

async fn read_one(api: &Api, id: &UserId) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let read = async {
        let user = transaction.user(id).await?;
        let outstanding = transaction.outstanding_enrolments().await?;
        Ok::<_, StoreError>((user, outstanding))
    }
    .await;
    transaction.roll_back().await?;

    let (found, mut outstanding) = read?;

    Ok(match found {
        None => answers::no_such("user"),
        Some(user) => {
            let code = outstanding.remove(&user.id);
            Json(Account::of(&user, code)).into_response()
        }
    })
}

/// Create a user. `SystemAdministration`, audited — refused or not.
pub(super) async fn create(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Json(new): Json<NewAccount>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };

    answers::or_unavailable(creating(&api, acting, new).await)
}

/// Creating is the one write with no record before it, so it is the one that does not go
/// through [`administer`].
async fn creating(api: &Api, acting: &UserId, new: NewAccount) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let administrator = Administrator::of(&mut transaction, acting).await?;

    let created = transaction
        .create_user(NewUser {
            username: new.username.clone(),
            password_hash: None,
            is_system_administrator: new.system_administration,
        })
        .await;

    let id = match created {
        Ok(id) => id,
        Err(AdministrationRefused::Store(error)) => return Err(error),
        Err(refusal) => {
            transaction.roll_back().await?;
            return refuse(
                api,
                &administrator,
                AuditEvent::UserCreated,
                ConfigurationWrite {
                    // Nothing to name, nothing before it and nothing after it: the record
                    // never came into existence.
                    target: None,
                    target_name: new.username,
                    before: None,
                    after: None,
                    blast_radius: nothing_live(),
                    refusal: Some(reason(&refusal)),
                },
                answer_to(&refusal),
            )
            .await;
        }
    };

    let Some(created) = transaction.user(&id).await? else {
        // Unreachable: the record was written through this very transaction.
        transaction.roll_back().await?;
        return Ok(answers::no_such("user"));
    };

    transaction
        .record(administrator.wrote(
            AuditEvent::UserCreated,
            ConfigurationWrite {
                target: Some(id),
                target_name: created.username.clone(),
                before: None,
                after: Some(Snapshot::of(&created)),
                blast_radius: nothing_live(),
                refusal: None,
            },
        ))
        .await?;
    transaction.commit().await?;

    tracing::info!(target: module::CONFIGURATION, user = %created.id.as_str(), "user created");

    Ok((StatusCode::CREATED, Json(Account::from(&created))).into_response())
}

/// Edit a user: the name they type, the flag they hold, or both.
///
/// `SystemAdministration`, audited — refused or not.
pub(super) async fn edit(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    Json(edit): Json<Edit>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };
    if edit.username.is_none() && edit.system_administration.is_none() {
        return answers::cannot("That edit asks for no change.");
    }

    let target = UserId::presented(id);

    // Two writes, one transaction and one entry: an edit that renames and takes the flag away
    // is one act, so it lands whole or not at all and the log records where the record
    // started and where it ended.
    answers::or_unavailable(
        administer(
            api.clone(),
            acting,
            AuditEvent::UserEdited,
            &target,
            async |transaction| {
                let mut made = None;

                if let Some(username) = &edit.username {
                    made = transaction.rename_user(&target, username).await?;
                }

                if let Some(held) = edit.system_administration {
                    let then = transaction.set_system_administration(&target, held).await?;
                    made = match (made, then) {
                        (Some(first), Some(then)) => Some(first.then(then)),
                        (first, then) => then.or(first),
                    };
                }

                Ok(made)
            },
        )
        .await,
    )
}

/// Delete a user. `SystemAdministration`, audited — refused or not.
///
/// Their sign-ins go with them, and their audit entries stay: the log outlives the records it
/// references, so deleting a user leaves their entries readable and attributed ([ADR-0028]).
///
/// [ADR-0028]: ../../../docs/adr/0028-the-audit-log-records-decisions-not-traffic.md
pub(super) async fn delete(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };
    let target = UserId::presented(id);

    answers::or_unavailable(
        administer(
            api.clone(),
            acting,
            AuditEvent::UserDeleted,
            &target,
            async |transaction| transaction.delete_user(&target).await,
        )
        .await,
    )
}

/// Lock an account. `SystemAdministration`, audited — refused or not.
///
/// It ends the user's sign-in and their session immediately (v1 §2's lifetime table). It is
/// never a consequence of failed attempts: auto-lock is a denial of service aimed at whoever
/// is starting a shift, so locking is only ever somebody's decision ([ADR-0025]).
///
/// [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
pub(super) async fn lock(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    setting_the_lock(api, &caller, id, true).await
}

/// Unlock an account. `SystemAdministration`, audited.
pub(super) async fn unlock(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    setting_the_lock(api, &caller, id, false).await
}

async fn setting_the_lock(api: Api, caller: &Caller, id: String, locked: bool) -> Response {
    let Some(acting) = acting(caller) else {
        return unreachable_caller();
    };
    let target = UserId::presented(id);
    let event = if locked {
        AuditEvent::AccountLocked
    } else {
        AuditEvent::AccountUnlocked
    };

    answers::or_unavailable(
        administer(api, acting, event, &target, async |transaction| {
            transaction.set_account_lock(&target, locked).await
        })
        .await,
    )
}

/// Force a password reset. `SystemAdministration`, audited.
///
/// It takes the password away and **ends the user's sign-in and session immediately** (v1
/// §2's lifetime table), leaving an account that exists and cannot be signed into until an
/// enrolment code sets a new password (#32). There is no self-service reset and no link to
/// send, because there is no mail path ([ADR-0025]).
///
/// It is not one of the three acts the last system administrator is protected from (v1 §2):
/// the account keeps its flag and its record, and the on-box CLI is what resets a password
/// nobody left can reset (#32).
///
/// [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
pub(super) async fn force_password_reset(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };
    let target = UserId::presented(id);

    answers::or_unavailable(
        administer(
            api.clone(),
            acting,
            AuditEvent::PasswordResetForced,
            &target,
            async |transaction| Ok(transaction.clear_password(&target).await?),
        )
        .await,
    )
}

/// Issue an enrolment code against a user. `SystemAdministration`, audited.
///
/// **An enrolment code is a credential** ([ADR-0025]), so issuing one is an administration
/// write in its own right and not a step inside some other act: it is expiring, single-use,
/// and it is what sets a password on an account that has none — or replaces the one on an
/// account that has. Issuing a second invalidates the first, so an administrator who has
/// mislaid a code reissues rather than leaving two in circulation.
///
/// The code comes back **once**, in this answer, for the administrator to hand over out of
/// band. Nothing reads one back afterwards, here or in the audit log: a credential readable
/// twice is one that was never single-use in the sense that matters.
///
/// It does not take the existing password away. *Force a password reset* is the separate act
/// that does, and it is a separate row in `docs/spec/api-surface.md` because an administrator
/// enrolling somebody new and an administrator cutting off a compromised account are doing
/// two different things.
///
/// [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
pub(super) async fn issue_enrolment_code(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };

    answers::or_unavailable(issuing(&api, acting, &UserId::presented(id)).await)
}

async fn issuing(api: &Api, acting: &UserId, target: &UserId) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let administrator = Administrator::of(&mut transaction, acting).await?;

    let Some(user) = transaction.user(target).await? else {
        transaction.roll_back().await?;
        return Ok(answers::no_such("user"));
    };

    let issued = transaction.issue_enrolment_code(target).await?;

    transaction
        .record(
            administrator.wrote(
                AuditEvent::EnrolmentCodeIssued,
                ConfigurationWrite {
                    target: Some(user.id.clone()),
                    target_name: user.username.clone(),
                    // The before and after are the codes rather than the record, because the
                    // record is untouched: what this write changed is which credential enrols
                    // this user, and an entry showing the account unchanged would say nothing.
                    before: issued
                        .replaced
                        .map(|code| Snapshot::of_enrolment(code.expires_at)),
                    after: Some(Snapshot::of_enrolment(issued.outstanding.expires_at)),
                    blast_radius: nothing_live(),
                    refusal: None,
                },
            ),
        )
        .await?;
    transaction.commit().await?;

    tracing::info!(
        target: module::CONFIGURATION,
        user = %user.id.as_str(),
        "an enrolment code was issued"
    );

    Ok((
        StatusCode::CREATED,
        Json(IssuedCode {
            code: issued.code.as_str().to_owned(),
            expires_at: issued.outstanding.expires_at,
        }),
    )
        .into_response())
}

/// Make one write to a user record, audit it, and commit the two together.
///
/// This is the shape of every configuration write VoxLoop will ever make, which is why it is
/// one function rather than the same eight lines per operation: one transaction, the write
/// through it, the entry through it carrying before, after and the blast radius, one commit.
/// A write that refuses says so and is audited anyway; a write that named nobody is a
/// not-found rather than a refusal, and there is no record for an entry to be about.
async fn administer(
    api: Api,
    acting: &UserId,
    event: AuditEvent,
    target: &UserId,
    write: impl AsyncFnOnce(&mut Transaction) -> Result<Option<Change>, AdministrationRefused>,
) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let administrator = Administrator::of(&mut transaction, acting).await?;

    let change = match write(&mut transaction).await {
        Ok(Some(change)) => change,
        Ok(None) => {
            transaction.roll_back().await?;
            return Ok(answers::no_such("user"));
        }
        Err(AdministrationRefused::Store(error)) => return Err(error),
        Err(refusal) => {
            transaction.roll_back().await?;
            return refuse_about(&api, &administrator, event, target, &refusal).await;
        }
    };

    transaction
        .record(administrator.wrote(event, recorded(&change)))
        .await?;
    transaction.commit().await?;

    tracing::info!(
        target: module::CONFIGURATION,
        user = %target.as_str(),
        ?event,
        "user administered"
    );

    Ok(match &change.after {
        Some(after) => Json(Account::from(after)).into_response(),
        // A deletion has nothing to answer with, which is the whole of what it says.
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

/// The user whose behalf this request acts on.
///
/// Unreachable as anything else: every route in this module is registered under
/// `SystemAdministration`, which resolves a user before the handler runs. It is an `Option`
/// rather than an assumption so that the impossible case cannot be answered with a
/// mis-attributed audit entry, which is the one failure this module must not have.
fn acting(caller: &Caller) -> Option<&UserId> {
    match caller {
        Caller::User { id, .. } => Some(id),
        Caller::Nobody => None,
    }
}

fn unreachable_caller() -> Response {
    answers::refusal("That operation is for a system administrator.")
}

/// Whoever is administering, as the log will record them.
struct Administrator {
    id: UserId,
    /// The name as it stands, snapshotted into every entry this administrator writes.
    name: String,
}

impl Administrator {
    async fn of(transaction: &mut Transaction, id: &UserId) -> Result<Self, StoreError> {
        Ok(Self {
            id: id.clone(),
            name: name_as_it_stands(transaction, id).await?,
        })
    }

    fn wrote(&self, event: AuditEvent, write: ConfigurationWrite) -> AuditEntry {
        AuditEntry {
            event,
            actor: Some(self.id.clone()),
            actor_name: self.name.clone(),
            // An administration write is an act by somebody the store already recognises, so
            // where it came from adds nothing the actor does not already say.
            source: None,
            write: Some(write),
            operation: None,
        }
    }
}

/// What this write does to anything live at the moment it lands.
///
/// The state authority computes it from sessions, subscriptions and arms, and hands it over
/// as a value ([ADR-0039]) — this is the one line that changes when there are sessions for a
/// lock or a deletion to end (#37, #53). Until then there is nothing live for any write here
/// to touch, and an empty radius is the honest answer rather than a placeholder.
///
/// [ADR-0039]: ../../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
fn nothing_live() -> BlastRadius {
    BlastRadius::nothing_live()
}

/// A change to a user record, as the log holds it.
fn recorded(change: &Change) -> ConfigurationWrite {
    ConfigurationWrite {
        target: Some(change.before.id.clone()),
        target_name: change
            .after
            .as_ref()
            .unwrap_or(&change.before)
            .username
            .clone(),
        before: Some(Snapshot::of(&change.before)),
        after: change.after.as_ref().map(Snapshot::of),
        blast_radius: nothing_live(),
        refusal: None,
    }
}

/// Why a write did not happen, in one sentence, and how it is answered.
///
/// Configuration answers refused and never *why* in a form somebody can act on, and turning
/// that into something a human reads is Transport's job — the same division `routes.rs` makes
/// for a requirement nobody met. The log holds this sentence rather than the error's own
/// wording, so what the console shows later is what the administrator was told at the time.
///
/// A refusal says *you may not* with the reason rather than hiding the operation (v1 §3). A
/// name already taken is a different thing and answered as one: the caller may make users,
/// and this particular attempt at it will not do.
fn refusal_of(refusal: &AdministrationRefused) -> (String, StatusCode) {
    match refusal {
        AdministrationRefused::LastSystemAdministrator => (
            "This is the last system administrator this deployment can be administered by, and \
             the last one cannot be removed."
                .to_owned(),
            StatusCode::FORBIDDEN,
        ),
        AdministrationRefused::NameTaken { username } => (
            format!("The username {username:?} is already taken."),
            StatusCode::BAD_REQUEST,
        ),
        // Unreachable: a store fault is answered as a fault rather than as a refusal.
        AdministrationRefused::Store(_) => (
            "That write could not be made.".to_owned(),
            StatusCode::BAD_REQUEST,
        ),
    }
}

fn reason(refusal: &AdministrationRefused) -> String {
    refusal_of(refusal).0
}

fn answer_to(refusal: &AdministrationRefused) -> Response {
    let (reason, status) = refusal_of(refusal);

    match status {
        StatusCode::FORBIDDEN => answers::refusal(&reason),
        _ => answers::cannot(&reason),
    }
}

/// Record a write refused over the record it was about, and answer with why.
///
/// The record is read here rather than before every write, because a refusal is the rare
/// path and the entry needs a transaction of its own anyway: the attempt abandoned whatever
/// it had open.
async fn refuse_about(
    api: &Api,
    administrator: &Administrator,
    event: AuditEvent,
    target: &UserId,
    refusal: &AdministrationRefused,
) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let before = transaction.user(target).await?;
    transaction.roll_back().await?;

    refuse(
        api,
        administrator,
        event,
        ConfigurationWrite {
            target: before.as_ref().map(|user| user.id.clone()),
            target_name: before
                .as_ref()
                .map_or_else(String::new, |user| user.username.clone()),
            before: before.as_ref().map(Snapshot::of),
            // Nothing after it: the write did not happen.
            after: None,
            blast_radius: nothing_live(),
            refusal: Some(reason(refusal)),
        },
        answer_to(refusal),
    )
    .await
}

/// Record a refused write, and answer with why.
///
/// Refused administration writes are audited (v1 §3), and the entry needs a transaction of
/// its own because the refusal has abandoned whatever the attempt had open.
async fn refuse(
    api: &Api,
    administrator: &Administrator,
    event: AuditEvent,
    write: ConfigurationWrite,
    answer: Response,
) -> Result<Response, StoreError> {
    tracing::warn!(
        target: module::CONFIGURATION,
        actor = %administrator.id.as_str(),
        ?event,
        "an administration write was refused"
    );

    let mut transaction = api.store.begin().await?;
    transaction
        .record(administrator.wrote(event, write))
        .await?;
    transaction.commit().await?;

    Ok(answer)
}

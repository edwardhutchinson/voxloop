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
    AdministrationRefused, AuditEntry, AuditEvent, AuditLog, BlastRadius, ConfigurationWrite,
    NewUser, Snapshot, StoreError, Transaction, User, UserId, Users,
};
use crate::telemetry::module;

/// A user, as the console reads one.
#[derive(Serialize)]
struct Account {
    id: String,
    username: String,
    system_administration: bool,
    locked: bool,
}

impl From<User> for Account {
    fn from(user: User) -> Self {
        Self {
            id: user.id.as_str().to_owned(),
            username: user.username,
            system_administration: user.is_system_administrator,
            locked: user.is_locked,
        }
    }
}

/// What the console sends to create a user.
///
/// No password: every record is created by system administration and the user sets their own
/// through an enrolment code, because VoxLoop has no mail path ([ADR-0025]). Until #32 issues
/// one, a created user is a record nobody can yet sign in as, which is an ordinary state
/// rather than a fault.
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
    let found = transaction.users().await;
    transaction.roll_back().await?;

    let accounts: Vec<Account> = found?.into_iter().map(Account::from).collect();

    Ok(Json(accounts).into_response())
}

/// Read one user. `SystemAdministration`. A read, so it is not audited.
pub(super) async fn read(State(api): State<Api>, Path(id): Path<String>) -> Response {
    answers::or_unavailable(read_one(&api, &UserId::presented(id)).await)
}

async fn read_one(api: &Api, id: &UserId) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let found = transaction.user(id).await;
    transaction.roll_back().await?;

    Ok(match found? {
        None => answers::no_such("user"),
        Some(user) => Json(Account::from(user)).into_response(),
    })
}

/// Create a user. `SystemAdministration`, audited — refused or not.
pub(super) async fn create(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Json(new): Json<NewAccount>,
) -> Response {
    answers::or_unavailable(creating(&api, &caller, new).await)
}

async fn creating(api: &Api, caller: &Caller, new: NewAccount) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let administrator = Administrator::of(&mut transaction, caller).await?;

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
                    target: None,
                    target_name: new.username,
                    // Nothing before it and nothing after it: the record never existed.
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

    let created = read_back(&mut transaction, &id).await?;
    transaction
        .record(administrator.wrote(
            AuditEvent::UserCreated,
            ConfigurationWrite {
                target: Some(id),
                target_name: created.username.clone(),
                // Nothing before it: the record did not exist a moment ago.
                before: None,
                after: Some(Snapshot::of(&created)),
                blast_radius: nothing_live(),
                refusal: None,
            },
        ))
        .await?;
    transaction.commit().await?;

    tracing::info!(target: module::CONFIGURATION, user = %created.id.as_str(), "user created");

    Ok((StatusCode::CREATED, Json(Account::from(created))).into_response())
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
    if edit.username.is_none() && edit.system_administration.is_none() {
        return answers::cannot("That edit asks for no change.");
    }

    answers::or_unavailable(editing(&api, &caller, &UserId::presented(id), edit).await)
}

async fn editing(
    api: &Api,
    caller: &Caller,
    id: &UserId,
    edit: Edit,
) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let administrator = Administrator::of(&mut transaction, caller).await?;

    let Some(before) = transaction.user(id).await? else {
        transaction.roll_back().await?;
        return Ok(answers::no_such("user"));
    };

    let mut applied = Ok(());
    if let Some(username) = &edit.username {
        applied = transaction.rename_user(id, username).await;
    }
    if let (Ok(()), Some(held)) = (&applied, edit.system_administration) {
        applied = transaction.set_system_administration(id, held).await;
    }

    match applied {
        Ok(()) => {}
        Err(AdministrationRefused::Store(error)) => return Err(error),
        Err(refusal) => {
            // Both halves are abandoned, so an edit that renames and demotes in one call
            // either lands whole or does not land at all.
            transaction.roll_back().await?;
            return refuse(
                api,
                &administrator,
                AuditEvent::UserEdited,
                a_refused_write(&before, &refusal),
                answer_to(&refusal),
            )
            .await;
        }
    }

    let after = read_back(&mut transaction, id).await?;
    transaction
        .record(administrator.wrote(AuditEvent::UserEdited, changed(&before, &after)))
        .await?;
    transaction.commit().await?;

    tracing::info!(target: module::CONFIGURATION, user = %id.as_str(), "user edited");

    Ok(Json(Account::from(after)).into_response())
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
    answers::or_unavailable(deleting(&api, &caller, &UserId::presented(id)).await)
}

async fn deleting(api: &Api, caller: &Caller, id: &UserId) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let administrator = Administrator::of(&mut transaction, caller).await?;

    let Some(before) = transaction.user(id).await? else {
        transaction.roll_back().await?;
        return Ok(answers::no_such("user"));
    };

    match transaction.delete_user(id).await {
        Ok(()) => {}
        Err(AdministrationRefused::Store(error)) => return Err(error),
        Err(refusal) => {
            transaction.roll_back().await?;
            return refuse(
                api,
                &administrator,
                AuditEvent::UserDeleted,
                a_refused_write(&before, &refusal),
                answer_to(&refusal),
            )
            .await;
        }
    }

    transaction
        .record(administrator.wrote(
            AuditEvent::UserDeleted,
            ConfigurationWrite {
                target: Some(before.id.clone()),
                target_name: before.username.clone(),
                before: Some(Snapshot::of(&before)),
                // Nothing after it, which is the whole of what a deletion says.
                after: None,
                blast_radius: nothing_live(),
                refusal: None,
            },
        ))
        .await?;
    transaction.commit().await?;

    tracing::info!(target: module::CONFIGURATION, user = %id.as_str(), "user deleted");

    Ok(StatusCode::NO_CONTENT.into_response())
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
    answers::or_unavailable(setting_the_lock(&api, &caller, &UserId::presented(id), true).await)
}

/// Unlock an account. `SystemAdministration`, audited.
pub(super) async fn unlock(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    answers::or_unavailable(setting_the_lock(&api, &caller, &UserId::presented(id), false).await)
}

async fn setting_the_lock(
    api: &Api,
    caller: &Caller,
    id: &UserId,
    locked: bool,
) -> Result<Response, StoreError> {
    let event = if locked {
        AuditEvent::AccountLocked
    } else {
        AuditEvent::AccountUnlocked
    };

    let mut transaction = api.store.begin().await?;
    let administrator = Administrator::of(&mut transaction, caller).await?;

    let Some(before) = transaction.user(id).await? else {
        transaction.roll_back().await?;
        return Ok(answers::no_such("user"));
    };

    match transaction.set_account_lock(id, locked).await {
        Ok(()) => {}
        Err(AdministrationRefused::Store(error)) => return Err(error),
        Err(refusal) => {
            transaction.roll_back().await?;
            return refuse(
                api,
                &administrator,
                event,
                a_refused_write(&before, &refusal),
                answer_to(&refusal),
            )
            .await;
        }
    }

    let after = read_back(&mut transaction, id).await?;
    transaction
        .record(administrator.wrote(event, changed(&before, &after)))
        .await?;
    transaction.commit().await?;

    tracing::info!(target: module::CONFIGURATION, user = %id.as_str(), locked, "account lock set");

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Force a password reset. `SystemAdministration`, audited.
///
/// It takes the password away and **ends the user's sign-in and session immediately** (v1
/// §2's lifetime table), leaving an account that exists and cannot be signed into until an
/// enrolment code sets a new password (#32). There is no self-service reset and no link to
/// send, because there is no mail path ([ADR-0025]).
///
/// [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
pub(super) async fn force_password_reset(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    answers::or_unavailable(forcing_a_reset(&api, &caller, &UserId::presented(id)).await)
}

async fn forcing_a_reset(api: &Api, caller: &Caller, id: &UserId) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let administrator = Administrator::of(&mut transaction, caller).await?;

    let Some(before) = transaction.user(id).await? else {
        transaction.roll_back().await?;
        return Ok(answers::no_such("user"));
    };

    transaction.clear_password(id).await?;

    let after = read_back(&mut transaction, id).await?;
    transaction
        .record(administrator.wrote(AuditEvent::PasswordResetForced, changed(&before, &after)))
        .await?;
    transaction.commit().await?;

    tracing::info!(target: module::CONFIGURATION, user = %id.as_str(), "password reset forced");

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Whoever is administering, as the log will record them.
struct Administrator {
    id: UserId,
    /// The name as it stands, snapshotted into every entry this administrator writes.
    name: String,
}

impl Administrator {
    /// The caller the requirement resolved, with the name the store holds for them.
    async fn of(transaction: &mut Transaction, caller: &Caller) -> Result<Self, StoreError> {
        // Unreachable as `Nobody`: `SystemAdministration` resolved a user before the handler
        // ran, and there is no route here carrying anything else.
        let Caller::User { id, .. } = caller else {
            return Ok(Self {
                id: UserId::presented(String::new()),
                name: String::new(),
            });
        };

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

/// The record before and after a write that landed.
fn changed(before: &User, after: &User) -> ConfigurationWrite {
    ConfigurationWrite {
        target: Some(after.id.clone()),
        target_name: after.username.clone(),
        before: Some(Snapshot::of(before)),
        after: Some(Snapshot::of(after)),
        blast_radius: nothing_live(),
        refusal: None,
    }
}

/// The record as it stood, and why it was left that way.
fn a_refused_write(before: &User, refusal: &AdministrationRefused) -> ConfigurationWrite {
    ConfigurationWrite {
        target: Some(before.id.clone()),
        target_name: before.username.clone(),
        before: Some(Snapshot::of(before)),
        // Nothing after it: the write did not happen.
        after: None,
        blast_radius: nothing_live(),
        refusal: Some(reason(refusal)),
    }
}

/// Why a write did not happen, in one sentence.
///
/// Configuration answers refused and never *why* in a form somebody can act on, and turning
/// that into something a human reads is Transport's job — the same division routes.rs makes
/// for a requirement nobody met. The log holds this sentence rather than the error's own
/// wording, so what the console shows later is what the administrator was told at the time.
fn reason(refusal: &AdministrationRefused) -> String {
    match refusal {
        AdministrationRefused::LastSystemAdministrator => {
            "This is the last system administrator this deployment can be administered by, and \
             the last one cannot be removed."
                .to_owned()
        }
        AdministrationRefused::NameTaken { username } => {
            format!("The username {username:?} is already taken.")
        }
        // Unreachable: a store fault is answered as a fault rather than as a refusal.
        AdministrationRefused::Store(_) => "That write could not be made.".to_owned(),
    }
}

/// What a refusal says to whoever asked for it.
///
/// A refusal says *you may not* with the reason rather than hiding the operation (v1 §3). A
/// name already taken is a different thing and answered as one: the caller may make users,
/// and this particular attempt at it will not do.
fn answer_to(refusal: &AdministrationRefused) -> Response {
    match refusal {
        AdministrationRefused::LastSystemAdministrator => answers::refusal(&reason(refusal)),
        _ => answers::cannot(&reason(refusal)),
    }
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

/// The record a write just made or changed.
///
/// It is read back through the same transaction rather than assembled from what was asked
/// for, so the audit entry's *after* is what the store holds rather than what the handler
/// believes it wrote.
async fn read_back(transaction: &mut Transaction, id: &UserId) -> Result<User, StoreError> {
    transaction.user(id).await?.map_or_else(
        || {
            Err(crate::configuration::StoreError::Unavailable(
                "a user written in this transaction could not be read back".into(),
            ))
        },
        Ok,
    )
}

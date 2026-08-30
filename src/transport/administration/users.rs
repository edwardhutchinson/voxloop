//! User administration: create, read, edit, delete, lock, unlock, force a password reset and
//! issue an enrolment code.
//!
//! This is the admin console's first page, and it writes through the audited path in
//! [`super`] like every other configuration page.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};

use super::{Administrator, acting, administer, create, nothing_live, unreachable_caller};
use crate::authorisation::Caller;
use crate::configuration::{
    AdministrationRefused, AuditEvent, AuditLog, Change, Enrolment, NewUser, Outstanding,
    StoreError, Transaction, User, UserId, Users,
};
use crate::telemetry::module;
use crate::transport::{Api, answers};

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
    /// A user, and whatever enrolment code is outstanding against them.
    ///
    /// The code is a second read rather than something a write's answer may leave out.
    /// Displayed state is factual (v1's standing requirements), and answering *no code
    /// outstanding* because this particular write did not look is the console asserting
    /// something the server never checked.
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

    /// The user as they stand, read through the transaction that just wrote them.
    async fn read_through(
        transaction: &mut Transaction,
        user: &User,
    ) -> Result<Response, StoreError> {
        let outstanding = transaction.outstanding_enrolments().await?.remove(&user.id);

        Ok(Json(Self::of(user, outstanding)).into_response())
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
pub(in crate::transport) struct NewAccount {
    username: String,
    #[serde(default)]
    system_administration: bool,
}

/// What the console sends to edit one. Absent means *leave it alone*.
#[derive(Deserialize)]
pub(in crate::transport) struct Edit {
    username: Option<String>,
    system_administration: Option<bool>,
}

/// Read every user. `SystemAdministration`. A read, so it is not audited.
pub(in crate::transport) async fn list(State(api): State<Api>) -> Response {
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
pub(in crate::transport) async fn read(State(api): State<Api>, Path(id): Path<String>) -> Response {
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
pub(in crate::transport) async fn create_account(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Json(new): Json<NewAccount>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };
    let username = new.username.clone();

    answers::or_unavailable(
        create(
            &api,
            acting,
            AuditEvent::UserCreated,
            username.clone(),
            async |transaction: &mut Transaction| {
                let id = transaction
                    .create_user(NewUser {
                        username: username.clone(),
                        password_hash: None,
                        is_system_administrator: new.system_administration,
                    })
                    .await?;

                Ok(transaction.user(&id).await?)
            },
            async |_transaction: &mut Transaction, created: &User| {
                // A record that came into existence a moment ago holds no code, and that is a
                // fact about this write rather than an assumption: nothing can have issued
                // one against an id nobody had yet.
                Ok((StatusCode::CREATED, Json(Account::of(created, None))).into_response())
            },
        )
        .await,
    )
}

/// Edit a user: the name they type, the flag they hold, or both.
///
/// `SystemAdministration`, audited — refused or not.
pub(in crate::transport) async fn edit(
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
    administering(
        &api,
        acting,
        AuditEvent::UserEdited,
        &target,
        async |transaction: &mut Transaction| {
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
    .await
}

/// Delete a user. `SystemAdministration`, audited — refused or not.
///
/// Their sign-ins go with them, and their audit entries stay: the log outlives the records it
/// references, so deleting a user leaves their entries readable and attributed ([ADR-0028]).
///
/// [ADR-0028]: ../../../docs/adr/0028-the-audit-log-records-decisions-not-traffic.md
pub(in crate::transport) async fn delete(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };
    let target = UserId::presented(id);

    administering(
        &api,
        acting,
        AuditEvent::UserDeleted,
        &target,
        async |transaction: &mut Transaction| transaction.delete_user(&target).await,
    )
    .await
}

/// Lock an account. `SystemAdministration`, audited — refused or not.
///
/// It ends the user's sign-in and their session immediately (v1 §2's lifetime table). It is
/// never a consequence of failed attempts: auto-lock is a denial of service aimed at whoever
/// is starting a shift, so locking is only ever somebody's decision ([ADR-0025]).
///
/// [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
pub(in crate::transport) async fn lock(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    setting_the_lock(&api, &caller, id, true).await
}

/// Unlock an account. `SystemAdministration`, audited.
pub(in crate::transport) async fn unlock(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    setting_the_lock(&api, &caller, id, false).await
}

async fn setting_the_lock(api: &Api, caller: &Caller, id: String, locked: bool) -> Response {
    let Some(acting) = acting(caller) else {
        return unreachable_caller();
    };
    let target = UserId::presented(id);
    let event = if locked {
        AuditEvent::AccountLocked
    } else {
        AuditEvent::AccountUnlocked
    };

    administering(
        api,
        acting,
        event,
        &target,
        async |transaction: &mut Transaction| transaction.set_account_lock(&target, locked).await,
    )
    .await
}

/// Force a password reset. `SystemAdministration`, audited.
///
/// It takes the password away and **ends the user's sign-in and session immediately** (v1
/// §2's lifetime table), leaving an account that exists and cannot be signed into until an
/// enrolment code sets a new password. There is no self-service reset and no link to send,
/// because there is no mail path ([ADR-0025]).
///
/// It is not one of the three acts the last system administrator is protected from (v1 §2):
/// the account keeps its flag and its record, and the on-box CLI is what resets a password
/// nobody left can reset.
///
/// [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
pub(in crate::transport) async fn force_password_reset(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };
    let target = UserId::presented(id);

    administering(
        &api,
        acting,
        AuditEvent::PasswordResetForced,
        &target,
        async |transaction: &mut Transaction| Ok(transaction.clear_password(&target).await?),
    )
    .await
}

/// Every write to a user record, on the one audited path, answering with the account.
async fn administering(
    api: &Api,
    acting: &UserId,
    event: AuditEvent,
    target: &UserId,
    write: impl AsyncFnOnce(&mut Transaction) -> Result<Option<Change<User>>, AdministrationRefused>,
) -> Response {
    answers::or_unavailable(
        administer(
            api,
            acting,
            event,
            "user",
            async |transaction: &mut Transaction| transaction.user(target).await,
            write,
            Account::read_through,
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
pub(in crate::transport) async fn issue_enrolment_code(
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
        .record(administrator.wrote(
            AuditEvent::EnrolmentCodeIssued,
            issued.to_the_code(&user.id, &user.username, nothing_live()),
        ))
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

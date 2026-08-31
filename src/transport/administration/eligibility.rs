//! Eligibility: granting and revoking who may assume which role, from the two pages it is
//! administered from.
//!
//! **Eligibility is not a second matrix** ([ADR-0015]). Rendered as one, 190 users by 15
//! roles was the least legible object the console prototype produced, so there is no
//! whole-eligibility read here — no route, no shape, nothing to render a wall from. The grid
//! has one, as a reference view; this deliberately does not, because the grid's is fifteen
//! by twenty and this one would be fifteen by a hundred and ninety.
//!
//! What there is instead is two directions, and they are the two questions an administrator
//! actually arrives with:
//!
//! - **From the role**: *who may assume this*. The eligible and nobody else — a list of
//!   every user with a mark against some of them is the same wall one slice at a time.
//! - **From the user**: *which roles may this person assume*. The same rule, the other way.
//!
//! **Eligibility confers nothing.** It permits somebody to take up a role, and what that
//! role can hear, say or command is one cell on the grid. Nothing here widens reach, and a
//! user eligible for a role with an empty row may assume it and reach nothing.
//!
//! Revoking eligibility from somebody occupying the role **ends their occupancy
//! immediately, with the reason shown** (v1 §2's lifetime table). That half is live state
//! and arrives with sessions (#25); here it is a configuration write against a deployment
//! where nobody has assumed anything yet, and the blast radius says so honestly.
//!
//! [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Serialize;

use super::roles::RoleAsRead;
use super::{Administrator, acting, administer, nothing_live, unreachable_caller};
use crate::authorisation::Caller;
use crate::configuration::{
    AuditEvent, AuditLog, ConfigurationWrite, Eligibilities, Eligibility, Record, RoleId,
    StoreError, Transaction, User, UserId,
};
use crate::telemetry::module;
use crate::transport::{Api, answers};

/// A user, as an eligibility names one.
///
/// Enough to identify the person and nothing about their credential: whether an account is
/// locked or awaiting enrolment is the users page's question, and an eligibility list that
/// answered it would be a second users page kept in step by hand.
#[derive(Serialize)]
struct UserAsRead {
    id: String,
    username: String,
}

impl UserAsRead {
    fn of(user: &User) -> Self {
        Self {
            id: user.id.as_str().to_owned(),
            username: user.username.clone(),
        }
    }
}

/// One eligibility, as a write answers with one: the pair it names.
///
/// Both halves, because a grant is made from either page and the answer says what the write
/// landed on rather than what the caller happened to be looking at. There is no third field:
/// an eligibility has no rung, no condition and no expiry.
#[derive(Serialize)]
struct EligibilityAsRead {
    user: UserAsRead,
    role: RoleAsRead,
}

impl EligibilityAsRead {
    fn of(granted: &Eligibility) -> Self {
        Self {
            user: UserAsRead::of(&granted.user),
            role: RoleAsRead::of(&granted.for_role),
        }
    }
}

/// A role page's half: the role, and everyone who may assume it.
#[derive(Serialize)]
struct WhoMayAssume {
    role: RoleAsRead,
    users: Vec<UserAsRead>,
}

/// A user page's half: the user, and every role they may assume.
#[derive(Serialize)]
struct WhichRoles {
    user: UserAsRead,
    roles: Vec<RoleAsRead>,
}

/// Read who may assume a role. `SystemAdministration`. A read, so it is not audited.
pub(in crate::transport) async fn who_may_assume(
    State(api): State<Api>,
    Path(id): Path<String>,
) -> Response {
    answers::or_unavailable(read_who_may_assume(&api, &RoleId::presented(id)).await)
}

async fn read_who_may_assume(api: &Api, for_role: &RoleId) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let read = transaction.the_users_eligible_for(for_role).await;
    transaction.roll_back().await?;

    Ok(match read? {
        None => answers::no_such("role"),
        Some((role, users)) => Json(WhoMayAssume {
            role: RoleAsRead::of(&role),
            users: users.iter().map(UserAsRead::of).collect(),
        })
        .into_response(),
    })
}

/// Read which roles a user may assume. `SystemAdministration`. A read, so it is not audited.
pub(in crate::transport) async fn which_roles(
    State(api): State<Api>,
    Path(id): Path<String>,
) -> Response {
    answers::or_unavailable(read_which_roles(&api, &UserId::presented(id)).await)
}

async fn read_which_roles(api: &Api, user: &UserId) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let read = transaction.the_roles_open_to(user).await;
    transaction.roll_back().await?;

    Ok(match read? {
        None => answers::no_such("user"),
        Some((user, roles)) => Json(WhichRoles {
            user: UserAsRead::of(&user),
            roles: roles.iter().map(RoleAsRead::of).collect(),
        })
        .into_response(),
    })
}

/// Grant eligibility. `SystemAdministration`, audited with what it created.
///
/// It has its own audited path rather than going through [`super::create`] because a grant
/// names two records that already exist and creates the relation between them: the pair is
/// the whole address, so a pair naming nothing is a not-found rather than a creation that
/// failed. Nothing else can stop it — an eligibility is unconditional, so there is no
/// refusal for the log to record.
pub(in crate::transport) async fn grant(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path((user, for_role)): Path<(String, String)>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };

    answers::or_unavailable(
        granting(
            &api,
            acting,
            &UserId::presented(user),
            &RoleId::presented(for_role),
        )
        .await,
    )
}

async fn granting(
    api: &Api,
    acting: &UserId,
    user: &UserId,
    for_role: &RoleId,
) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let administrator = Administrator::of(&mut transaction, acting).await?;

    // Either half of the pair can be the one that is not there, and the answer says so
    // without guessing which: the console holds both ids and read both lists.
    let Some(granted) = transaction.grant_eligibility(user, for_role).await? else {
        transaction.roll_back().await?;
        return Ok(answers::no_such("user or role"));
    };

    transaction
        .record(administrator.wrote(
            AuditEvent::EligibilityGranted,
            ConfigurationWrite {
                target: Some(granted.recorded_id()),
                target_name: granted.recorded_name(),
                // Nothing before it. A grant that did not stand is not a lesser grant, it is
                // an absence, and the entry says so by having nothing to show.
                before: None,
                after: Some(granted.snapshot()),
                blast_radius: nothing_live(),
                refusal: None,
            },
        ))
        .await?;
    transaction.commit().await?;

    tracing::info!(
        target: module::CONFIGURATION,
        user = %granted.user.id.as_str(),
        role = %granted.for_role.id.as_str(),
        "eligibility was granted"
    );

    Ok((StatusCode::CREATED, Json(EligibilityAsRead::of(&granted))).into_response())
}

/// Revoke eligibility. `SystemAdministration`, audited with what it took away.
///
/// It goes through the ordinary audited path as a deletion, because that is what it is: the
/// grant is gone rather than reduced. Revoking one nobody holds is a not-found, so an
/// administrator working from a stale page is told their page is stale instead of being
/// shown a revocation that never happened.
///
/// **It ends the occupancy it revokes**, immediately and with the reason shown to whoever
/// was in the seat (v1 §2's lifetime table). That is live state and arrives with sessions
/// (#25); until then the blast radius is empty because there is genuinely nothing live.
pub(in crate::transport) async fn revoke(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path((user, for_role)): Path<(String, String)>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };
    let user = UserId::presented(user);
    let for_role = RoleId::presented(for_role);

    answers::or_unavailable(
        administer(
            &api,
            acting,
            AuditEvent::EligibilityRevoked,
            "eligibility",
            async |transaction: &mut Transaction| {
                transaction.an_eligibility(&user, &for_role).await
            },
            async |transaction: &mut Transaction| {
                Ok(transaction.revoke_eligibility(&user, &for_role).await?)
            },
            // Never reached: a revocation leaves nothing after it, and a deletion answers
            // with no content. It is here because the audited path answers with the record
            // wherever there is one, and eligibility is the one write where there never is.
            async |_transaction: &mut Transaction, granted: &Eligibility| {
                Ok(Json(EligibilityAsRead::of(granted)).into_response())
            },
        )
        .await,
    )
}

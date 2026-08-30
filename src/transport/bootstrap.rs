//! Redeeming the bootstrap code: a box with no system administrator becomes a box with one.
//!
//! This route is **registered only while no system administrator exists** and genuinely
//! stops existing afterwards — the one exception to VoxLoop's rule that refusals say *you
//! may not* rather than hiding an operation (v1 §3). Whether it is registered is decided at
//! startup, in [`super::start`]; nothing here has to check.
//!
//! There are no default credentials, ever ([ADR-0025]). The code was minted to the server's
//! own log, so whoever can read the box's console is who can perform this once.
//!
//! [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use super::{Api, answers};
use crate::configuration::{
    AdministrationRefused, AuditEntry, AuditEvent, AuditLog, NewUser, PasswordHash, StoreError,
    Users,
};
use crate::identity::{Bootstrap, PasswordRefused, Redemption};
use crate::telemetry::module;

/// What a browser presents to make the first system administrator.
#[derive(Deserialize)]
pub(super) struct FirstAdministrator {
    code: String,
    username: String,
    password: String,
}

/// Redeem the bootstrap code. `Public`, rate-limited on source, audited.
pub(super) async fn redeem(
    State(api): State<Api>,
    ConnectInfo(source): ConnectInfo<SocketAddr>,
    Json(presented): Json<FirstAdministrator>,
) -> Response {
    if !api.admits(&source) {
        return answers::too_many_attempts();
    }

    let Some(bootstrap) = api.bootstrap.as_deref() else {
        // Unreachable: with no code to redeem, this route was never registered.
        return answers::refusal("This deployment already has a system administrator.");
    };

    answers::or_unavailable(make_the_first_administrator(&api, bootstrap, presented, &source).await)
}

async fn make_the_first_administrator(
    api: &Api,
    bootstrap: &Bootstrap,
    presented: FirstAdministrator,
    source: &SocketAddr,
) -> Result<Response, StoreError> {
    // The code is checked before anything else happens, and checking does not spend it.
    // Everything after this point can tell the caller something about the deployment — that
    // a name is taken, that a password is too short — and none of it is anybody's to learn
    // without the code that was written to the server's own log.
    if !bootstrap.is_the_code(&presented.code) {
        return refuse(api, &presented.username, source).await;
    }

    let hashed = match api.identity.hash_password(&presented.password) {
        Ok(hashed) => hashed,
        Err(refusal @ PasswordRefused::TooShort) => {
            return Ok(answers::cannot(&refusal.to_string()));
        }
        Err(PasswordRefused::Unusable) => {
            tracing::error!(target: module::IDENTITY, "a password could not be hashed");
            return Ok(answers::cannot("That password could not be stored."));
        }
    };

    create(api, bootstrap, presented, hashed, source).await
}

async fn create(
    api: &Api,
    bootstrap: &Bootstrap,
    presented: FirstAdministrator,
    hashed: PasswordHash,
    source: &SocketAddr,
) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;

    if transaction.a_system_administrator_exists().await? {
        return Ok(answers::refusal(
            "This deployment already has a system administrator.",
        ));
    }

    // Everything that can refuse happens before the code is spent. A transaction abandoned
    // here rolls back, and the code is still good — so a name already taken costs an attempt
    // rather than the deployment. What is left is the store failing between the redemption
    // and the commit, which spends the code without making an administrator; the next start
    // mints another, which is why that is a restart rather than a dead box.
    let created = transaction
        .create_user(NewUser {
            username: presented.username.clone(),
            password_hash: Some(hashed),
            is_system_administrator: true,
        })
        .await;

    let user = match created {
        Ok(user) => user,
        Err(taken @ AdministrationRefused::NameTaken { .. }) => {
            return Ok(answers::cannot(&taken.to_string()));
        }
        Err(AdministrationRefused::Store(error)) => return Err(error),
        // Unreachable: creating a user takes no administrator away from the deployment, and
        // the other two refusals are about roles and loops, which this route never touches.
        Err(_) => {
            return Ok(answers::cannot("That user could not be created."));
        }
    };

    // Between the check at the top and here, another request may have spent the code. It is
    // the same refusal to whoever sent this one.
    //
    // The rollback is awaited rather than left to the handle being dropped, because the
    // refusal opens a transaction of its own to record itself, and the user this one just
    // wrote is holding the store's write lock until it lets go.
    if bootstrap.redeem(&presented.code) == Redemption::Refused {
        transaction.roll_back().await?;
        return refuse(api, &presented.username, source).await;
    }

    transaction
        .record(AuditEntry {
            event: AuditEvent::BootstrapRedeemed,
            actor: Some(user.clone()),
            actor_name: presented.username,
            source: Some(source.ip()),
            write: None,
            operation: None,
        })
        .await?;
    transaction.commit().await?;

    tracing::warn!(
        target: module::IDENTITY,
        user = %user.as_str(),
        "the bootstrap code was redeemed: this deployment now has a system administrator"
    );

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Refuse a code, and record that somebody tried.
///
/// Refused administration writes are audited ([ADR-0054]), and this is the write that makes
/// an administrator. It needs a transaction of its own because the refusal has abandoned
/// whatever the attempt had open.
///
/// [ADR-0054]: ../../../docs/adr/0054-every-operation-declares-its-authorisation.md
async fn refuse(api: &Api, submitted: &str, source: &SocketAddr) -> Result<Response, StoreError> {
    tracing::warn!(
        target: module::IDENTITY,
        source = %source.ip(),
        "a bootstrap code was presented and refused"
    );

    let mut transaction = api.store.begin().await?;
    transaction
        .record(AuditEntry {
            event: AuditEvent::BootstrapRefused,
            actor: None,
            actor_name: submitted.to_owned(),
            source: Some(source.ip()),
            write: None,
            operation: None,
        })
        .await?;
    transaction.commit().await?;

    Ok(answers::not_accepted(
        "That is not this server's bootstrap code.",
    ))
}

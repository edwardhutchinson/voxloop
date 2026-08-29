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

use super::rate_limit::Admission;
use super::{Api, answers};
use crate::configuration::{
    AuditEntry, AuditEvent, AuditLog, NewUser, PasswordHash, StoreError, Users,
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
    if api.limits.admit(source.ip()) == Admission::Throttled {
        return answers::too_many_attempts();
    }

    let Some(bootstrap) = api.bootstrap.as_deref() else {
        // Unreachable: with no code to redeem, this route was never registered.
        return answers::refusal("This deployment already has a system administrator.");
    };

    // Hashing before redeeming, so that a password under the floor does not spend the code
    // and leave the box unopenable until it is restarted.
    let hashed = match api.identity.hash_password(&presented.password) {
        Ok(hashed) => hashed,
        Err(refusal @ PasswordRefused::TooShort) => return answers::cannot(&refusal.to_string()),
        Err(PasswordRefused::Unusable) => {
            tracing::error!(target: module::IDENTITY, "a password could not be hashed");
            return answers::cannot("That password could not be stored.");
        }
    };

    match make_the_first_administrator(&api, bootstrap, presented, hashed, &source).await {
        Ok(answer) => answer,
        Err(error) => answers::unavailable(&error),
    }
}

async fn make_the_first_administrator(
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
    // here rolls back, and the code is still good.
    let created = transaction
        .create_user(NewUser {
            username: presented.username.clone(),
            password_hash: Some(hashed),
            is_system_administrator: true,
        })
        .await;

    let user = match created {
        Ok(user) => user,
        Err(taken @ StoreError::UsernameTaken { .. }) => {
            return Ok(answers::cannot(&taken.to_string()));
        }
        Err(error) => return Err(error),
    };

    if bootstrap.redeem(&presented.code) == Redemption::Refused {
        tracing::warn!(
            target: module::IDENTITY,
            source = %source.ip(),
            "a bootstrap code was presented and refused"
        );
        return Ok(answers::refusal(
            "That is not this server's bootstrap code.",
        ));
    }

    transaction
        .record(AuditEntry {
            event: AuditEvent::BootstrapRedeemed,
            actor: Some(user.clone()),
            actor_name: presented.username,
            source: Some(source.ip().to_string()),
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

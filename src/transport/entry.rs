//! Sign in and sign out: v1 §2's outermost act, and the end of it.
//!
//! **Sign in authenticates a principal to the application. It confers no role, no reach and
//! no audio** ([ADR-0023]). Assuming a role is a separate act over the signalling channel,
//! and it is not here.
//!
//! Both are audited, success and failure alike. A failed sign-in is where a brute-force
//! attempt becomes visible, which is the compensating control for rate-limiting on source
//! rather than locking accounts ([ADR-0025]).
//!
//! [ADR-0023]: ../../../docs/adr/0023-sign-in-is-to-the-application-and-a-role-is-assumed.md
//! [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use axum_extra::extract::CookieJar;
use serde::Deserialize;

use super::rate_limit::Admission;
use super::{Api, answers, cookies};
use crate::authorisation::Caller;
use crate::configuration::{
    AuditEntry, AuditEvent, AuditLog, SignIns, StoreError, Transaction, UserId, Users,
};
use crate::identity::Presented;
use crate::telemetry::module;

/// What a browser presents to sign in.
#[derive(Deserialize)]
pub(super) struct Credentials {
    username: String,
    password: String,
}

/// Sign in. `Public`, rate-limited on source, audited either way.
pub(super) async fn sign_in(
    State(api): State<Api>,
    ConnectInfo(source): ConnectInfo<SocketAddr>,
    jar: CookieJar,
    Json(presented): Json<Credentials>,
) -> Response {
    if api.limits.admit(source.ip()) == Admission::Throttled {
        return answers::too_many_attempts();
    }

    match attempt(&api, &source, jar, presented).await {
        Ok(answer) => answer,
        Err(error) => answers::unavailable(&error),
    }
}

async fn attempt(
    api: &Api,
    source: &SocketAddr,
    jar: CookieJar,
    presented: Credentials,
) -> Result<Response, StoreError> {
    let submitted_name = presented.username.clone();
    let credential = Presented::Password {
        username: presented.username,
        password: presented.password,
    };

    let mut transaction = api.store.begin().await?;
    let resolved = api.identity.resolve(&mut transaction, &credential).await?;

    // Identity answers a user id or nobody, and never why. A failure therefore has no actor
    // to attribute — only the name that was submitted, which is the whole of what is known,
    // and which is recorded as given because a name being sprayed at is the thing the log
    // exists to make visible. Reading the log is system administration, deliberately.
    let Some(user) = resolved else {
        transaction
            .record(AuditEntry {
                event: AuditEvent::SignInFailed,
                actor: None,
                actor_name: submitted_name,
                source: Some(source.ip().to_string()),
            })
            .await?;
        transaction.commit().await?;

        return Ok(answers::credentials_refused());
    };

    let token = transaction.open_sign_in(&user).await?;
    let name = name_as_it_stands(&mut transaction, &user).await?;
    transaction
        .record(AuditEntry {
            event: AuditEvent::SignInSucceeded,
            actor: Some(user.clone()),
            actor_name: name,
            source: Some(source.ip().to_string()),
        })
        .await?;
    transaction.commit().await?;

    tracing::info!(target: module::IDENTITY, user = %user.as_str(), "signed in");

    Ok((jar.add(cookies::holds(&token)), StatusCode::NO_CONTENT).into_response())
}

/// Sign out. `SignedIn`, audited.
///
/// Signing out ends everything ([ADR-0023]). There is no session to end yet; when there is,
/// it ends here too.
///
/// [ADR-0023]: ../../../docs/adr/0023-sign-in-is-to-the-application-and-a-role-is-assumed.md
pub(super) async fn sign_out(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    jar: CookieJar,
) -> Response {
    let Caller::User { id, sign_in } = caller else {
        // Unreachable: the requirement resolved a user before this handler ran.
        return answers::refusal("That operation is for a signed-in user.");
    };

    match end(&api, &id, &sign_in).await {
        Ok(()) => (jar.add(cookies::taken_back()), StatusCode::NO_CONTENT).into_response(),
        Err(error) => answers::unavailable(&error),
    }
}

async fn end(
    api: &Api,
    user: &UserId,
    sign_in: &crate::configuration::SignInToken,
) -> Result<(), StoreError> {
    let mut transaction = api.store.begin().await?;
    transaction.end_sign_in(sign_in).await?;
    let name = name_as_it_stands(&mut transaction, user).await?;
    transaction
        .record(AuditEntry {
            event: AuditEvent::SignedOut,
            actor: Some(user.clone()),
            actor_name: name,
            // A sign-out is an act by somebody the store already recognises, so where it came
            // from adds nothing the actor does not already say.
            source: None,
        })
        .await?;
    transaction.commit().await?;

    tracing::info!(target: module::IDENTITY, user = %user.as_str(), "signed out");

    Ok(())
}

/// The name to snapshot into an audit entry: the one the store holds, not the one submitted.
async fn name_as_it_stands(
    transaction: &mut Transaction,
    user: &UserId,
) -> Result<String, StoreError> {
    Ok(transaction
        .user(user)
        .await?
        .map_or_else(String::new, |user| user.username))
}

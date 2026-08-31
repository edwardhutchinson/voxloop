//! Changing one's own password: the one credential act a user performs for themselves.
//!
//! It is `SignedIn` and it takes the **current password re-presented**, which is what makes
//! it an act by the person rather than by whoever is sitting at their machine. It is
//! rate-limited on source like every route that accepts a credential, and audited whether
//! the current password was right or not.
//!
//! **It does not end the session** (v1 §2). An operator on the air who changes their
//! password should not lose audio for it, and there is nothing about a password the person
//! chose themselves that argues for cutting them off mid-word. Every *other* act on a
//! password — an administrator forcing a reset, a code being redeemed — ends the sign-ins
//! standing against the credential it replaced, and that difference is the whole distinction
//! between somebody changing their own and somebody having theirs changed.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;

use super::{Api, answers, name_as_it_stands, unstorable};
use crate::authorisation::Caller;
use crate::configuration::{AuditEntry, AuditEvent, AuditLog, StoreError, UserId, Users};
use crate::telemetry::module;

/// What a signed-in user presents to change their own password.
///
/// Named for what it holds rather than for the act, so that it cannot be mistaken for
/// Configuration's [`Change`], which is a record before and after a write.
///
/// [`Change`]: crate::configuration::Change
#[derive(Deserialize)]
pub(super) struct BothPasswords {
    current: String,
    new: String,
}

/// Both halves are live credentials, and one that turns up in a log is spent.
impl std::fmt::Debug for BothPasswords {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BothPasswords { current: withheld, new: withheld }")
    }
}

/// Change own password. `SignedIn`, rate-limited on source, audited. The session survives.
pub(super) async fn change(
    State(api): State<Api>,
    ConnectInfo(source): ConnectInfo<SocketAddr>,
    Extension(caller): Extension<Caller>,
    Json(presented): Json<BothPasswords>,
) -> Response {
    let Caller::User { id, .. } = caller else {
        // Unreachable: the requirement resolved a user before this handler ran.
        return answers::refusal("That operation is for a signed-in user.");
    };

    if !api.admits(&source) {
        return answers::too_many_attempts();
    }

    answers::or_unavailable(changing(&api, &id, presented, &source).await)
}

async fn changing(
    api: &Api,
    user: &UserId,
    presented: BothPasswords,
    source: &SocketAddr,
) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;

    if !api
        .identity
        .confirms(&mut transaction, user, &presented.current)
        .await?
    {
        let name = name_as_it_stands(&mut transaction, user).await?;
        transaction
            .record(AuditEntry {
                event: AuditEvent::PasswordChangeRefused,
                actor: Some(user.clone()),
                actor_name: name,
                // Where it came from is worth keeping here in a way it is not for an
                // ordinary act by a signed-in user: a run of these is the same brute-force
                // signal a run of failed sign-ins is, and against a known account.
                source: Some(source.ip()),
                write: None,
                operation: None,
                occupancy: None,
            })
            .await?;
        transaction.commit().await?;

        return Ok(answers::not_accepted("That is not your current password."));
    }

    let hashed = match api.identity.hash_password(&presented.new) {
        Ok(hashed) => hashed,
        Err(refusal) => {
            transaction.roll_back().await?;
            return Ok(unstorable(&refusal));
        }
    };

    let Some(_) = transaction.set_password(user, hashed).await? else {
        // Unreachable: the requirement read this record a moment ago.
        transaction.roll_back().await?;
        return Ok(answers::no_such("user"));
    };

    let name = name_as_it_stands(&mut transaction, user).await?;
    transaction
        .record(AuditEntry {
            event: AuditEvent::PasswordChanged,
            actor: Some(user.clone()),
            actor_name: name,
            source: Some(source.ip()),
            // An act by a principal on their own credential, not a configuration write on a
            // record: the log would otherwise hold two identical lines and say nothing.
            write: None,
            operation: None,
            occupancy: None,
        })
        .await?;
    transaction.commit().await?;

    tracing::info!(
        target: module::IDENTITY,
        user = %user.as_str(),
        "a user changed their own password"
    );

    // No cookie is taken back and none is handed out. The sign-in this request presented is
    // the sign-in the browser still holds afterwards.
    Ok(StatusCode::NO_CONTENT.into_response())
}

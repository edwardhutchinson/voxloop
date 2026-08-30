//! Who the caller is, as far as the console needs to know.
//!
//! This is what opens the admin console. It is gated on the **user's system-administration
//! flag and never on a role** (v1 §9), so an operator who is also a sysadmin reaches it from
//! the lobby and from within a session alike, without dropping off the air.
//!
//! The flag is here rather than in the cookie deliberately. The cookie carries no claims
//! (v1 §3), so the console asks and is told what the store says *now* — which is why taking
//! the flag away closes the console on the next request rather than at some expiry.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Serialize;

use super::{Api, answers};
use crate::authorisation::Caller;
use crate::configuration::{StoreError, UserId, Users};

/// The signed-in user, as the console frame reads them.
///
/// Eligible roles belong here too (`docs/spec/api-surface.md`) and arrive with eligibility
/// (#35). What is here is what the console frame needs: a name to show, and the one fact
/// that decides whether the admin console exists for this person.
#[derive(Serialize)]
struct Principal {
    id: String,
    username: String,
    system_administration: bool,
}

/// Read own principal. `SignedIn`. A read, so it is not audited.
pub(super) async fn own(State(api): State<Api>, Extension(caller): Extension<Caller>) -> Response {
    let Caller::User { id, .. } = caller else {
        // Unreachable: the requirement resolved a user before this handler ran.
        return answers::refusal("That operation is for a signed-in user.");
    };

    answers::or_unavailable(read(&api, &id).await)
}

async fn read(api: &Api, id: &UserId) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let found = transaction.user(id).await;
    transaction.roll_back().await?;

    Ok(match found? {
        // Unreachable in practice: the same read permitted this request a moment ago.
        None => answers::no_such("user"),
        Some(user) => Json(Principal {
            id: user.id.as_str().to_owned(),
            username: user.username,
            system_administration: user.is_system_administrator,
        })
        .into_response(),
    })
}

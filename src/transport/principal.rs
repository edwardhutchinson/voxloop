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
use crate::configuration::{Eligibilities, Role, StoreError, UserId};

/// The signed-in user, as the console frame reads them.
///
/// A name to show, the one fact that decides whether the admin console exists for this
/// person, and the roles they may assume — which is what the lobby is a list of.
#[derive(Serialize)]
struct Principal {
    id: String,
    username: String,
    system_administration: bool,
    /// The roles this user is eligible for, by name.
    ///
    /// Eligibility and nothing else. Who occupies each, and the staffing state of the loops
    /// those roles staff, is the lobby document the signalling channel pushes (#25) — this
    /// is the configuration half, read once when the frame opens.
    ///
    /// **Reach is not here and will not be.** A person's reach belongs to a (user, role)
    /// pair and is never composed across the roles they are eligible for ([ADR-0015]): a
    /// session is bound to one role, so a union would display authority nobody can hold.
    ///
    /// [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md
    eligible_for: Vec<EligibleRole>,
}

/// A role this user may assume.
#[derive(Serialize)]
struct EligibleRole {
    id: String,
    name: String,
}

impl EligibleRole {
    fn of(role: &Role) -> Self {
        Self {
            id: role.id.as_str().to_owned(),
            name: role.name.clone(),
        }
    }
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
    // The user and the roles they may assume, read together: the console frame renders them
    // as one thing, and two reads a moment apart would let it render a name from one moment
    // beside a lobby from another.
    let found = transaction.the_roles_open_to(id).await;
    transaction.roll_back().await?;

    Ok(match found? {
        // Unreachable in practice: the same read permitted this request a moment ago.
        None => answers::no_such("user"),
        Some((user, eligible_for)) => Json(Principal {
            id: user.id.as_str().to_owned(),
            username: user.username,
            system_administration: user.is_system_administrator,
            eligible_for: eligible_for.iter().map(EligibleRole::of).collect(),
        })
        .into_response(),
    })
}

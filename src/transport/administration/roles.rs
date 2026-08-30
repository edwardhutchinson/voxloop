//! Role administration: create, read, edit and delete roles, including `max_occupants`.
//!
//! A **role is a staffable position**, not a group of users (v1 §1). Nothing here says who
//! may assume one — that is eligibility (#35) — and nothing here says what one may hear or
//! say, which is the grid (#34). This page administers the position itself.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};

use super::{acting, administer, create, unreachable_caller};
use crate::authorisation::Caller;
use crate::configuration::{
    AdministrationRefused, AuditEvent, Change, NewRole, Role, RoleId, Roles, StoreError,
    Transaction,
};
use crate::transport::{Api, answers};

/// A role, as the console reads one.
#[derive(Serialize)]
struct Position {
    id: String,
    name: String,
    /// Absent is *no limit*, which is how the console renders it: the same concept with the
    /// limit unset rather than a second kind of role.
    max_occupants: Option<u32>,
}

impl Position {
    fn of(role: &Role) -> Self {
        Self {
            id: role.id.as_str().to_owned(),
            name: role.name.clone(),
            max_occupants: role.max_occupants,
        }
    }

    async fn read_through(
        _transaction: &mut Transaction,
        role: &Role,
    ) -> Result<Response, StoreError> {
        Ok(Json(Self::of(role)).into_response())
    }
}

/// What the console sends to create a role.
#[derive(Deserialize)]
pub(in crate::transport) struct NewPosition {
    name: String,
    /// Absent is no limit, and that is a decision an administrator makes rather than a field
    /// they forgot: `Observer` is seeded that way, and a site's own listen-only role wants
    /// the same.
    #[serde(default)]
    max_occupants: Option<u32>,
}

/// What the console sends to edit one. Absent means *leave it alone*.
///
/// `max_occupants` is therefore doubly optional: absent leaves the limit as it is, and
/// `null` takes it away.
#[derive(Deserialize)]
pub(in crate::transport) struct Edit {
    name: Option<String>,
    #[serde(default, deserialize_with = "crate::transport::present_or_absent")]
    max_occupants: Option<Option<u32>>,
}

/// Read every role. `SystemAdministration`. A read, so it is not audited.
pub(in crate::transport) async fn list(State(api): State<Api>) -> Response {
    answers::or_unavailable(read_all(&api).await)
}

async fn read_all(api: &Api) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let read = transaction.roles().await;
    transaction.roll_back().await?;

    let positions: Vec<Position> = read?.iter().map(Position::of).collect();

    Ok(Json(positions).into_response())
}

/// Read one role. `SystemAdministration`. A read, so it is not audited.
pub(in crate::transport) async fn read(State(api): State<Api>, Path(id): Path<String>) -> Response {
    answers::or_unavailable(read_one(&api, &RoleId::presented(id)).await)
}

async fn read_one(api: &Api, id: &RoleId) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let read = transaction.role(id).await;
    transaction.roll_back().await?;

    Ok(match read? {
        None => answers::no_such("role"),
        Some(role) => Json(Position::of(&role)).into_response(),
    })
}

/// Create a role. `SystemAdministration`, audited — refused or not.
pub(in crate::transport) async fn create_role(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Json(new): Json<NewPosition>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };

    answers::or_unavailable(
        create(
            &api,
            acting,
            AuditEvent::RoleCreated,
            new.name.clone(),
            async |transaction: &mut Transaction| {
                let id = transaction
                    .create_role(NewRole {
                        name: new.name.clone(),
                        max_occupants: new.max_occupants,
                    })
                    .await?;

                Ok(transaction.role(&id).await?)
            },
            async |_transaction: &mut Transaction, made: &Role| {
                Ok((axum::http::StatusCode::CREATED, Json(Position::of(made))).into_response())
            },
        )
        .await,
    )
}

/// Edit a role: its name, how many may occupy it, or both.
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
    if edit.name.is_none() && edit.max_occupants.is_none() {
        return answers::cannot("That edit asks for no change.");
    }

    let target = RoleId::presented(id);

    // Two writes, one transaction and one entry: renaming a role and widening its occupancy
    // is one act, so it lands whole or not at all.
    administering(
        &api,
        acting,
        AuditEvent::RoleEdited,
        &target,
        async |transaction: &mut Transaction| {
            let mut made = None;

            if let Some(name) = &edit.name {
                made = transaction.rename_role(&target, name).await?;
            }

            if let Some(max_occupants) = edit.max_occupants {
                let then = transaction
                    .set_max_occupants(&target, max_occupants)
                    .await?;
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

/// Delete a role. `SystemAdministration`, audited — refused or not.
///
/// Its audit entries stay, readable and attributed by the name as it stood.
pub(in crate::transport) async fn delete(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };
    let target = RoleId::presented(id);

    administering(
        &api,
        acting,
        AuditEvent::RoleDeleted,
        &target,
        async |transaction: &mut Transaction| Ok(transaction.delete_role(&target).await?),
    )
    .await
}

/// Every write to a role record, on the one audited path, answering with the role.
async fn administering(
    api: &Api,
    acting: &crate::configuration::UserId,
    event: AuditEvent,
    target: &RoleId,
    write: impl AsyncFnOnce(&mut Transaction) -> Result<Option<Change<Role>>, AdministrationRefused>,
) -> Response {
    answers::or_unavailable(
        administer(
            api,
            acting,
            event,
            "role",
            async |transaction: &mut Transaction| transaction.role(target).await,
            write,
            Position::read_through,
        )
        .await,
    )
}

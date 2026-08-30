//! Loop administration: create, read, edit and delete loops, and set the deployment-wide
//! base loop order.
//!
//! There is no loop kind, type, category or naming convention here, and there will not be
//! one ([ADR-0055]). A loop arrives `unreviewed` and says so until an administrator has
//! ruled on its column, which is the grid's act and arrives with it (#34).
//!
//! The base order is **administered, not derived** ([ADR-0053]): it is set as a complete
//! ordering, and a new loop lands at the end of it because appending is the only honest
//! placement for something VoxLoop has been told nothing about.
//!
//! [ADR-0053]: ../../../docs/adr/0053-the-loop-order-is-complete-and-a-new-loop-lands-at-the-end.md
//! [ADR-0055]: ../../../docs/adr/0055-there-is-no-conference-loop.md

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};

use super::{
    Administrator, acting, administer, answer_to, create, nothing_live, reason, refuse,
    unreachable_caller,
};
use crate::authorisation::Caller;
use crate::configuration::{
    AdministrationRefused, AuditEvent, AuditLog, Change, ConfigurationWrite, Loop, LoopId, Loops,
    Snapshot, StoreError, Transaction, UserId,
};
use crate::telemetry::module;
use crate::transport::{Api, answers};

/// A loop, as the console reads one. The list is in the base order, which is the order.
#[derive(Serialize)]
struct LoopAsRead {
    id: String,
    name: String,
    /// Whether anybody has ruled on this loop's column yet ([ADR-0015]). The console says so
    /// plainly, because absent-because-denied and absent-because-nobody-ruled render
    /// identically everywhere else.
    ///
    /// [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md
    unreviewed: bool,
}

impl LoopAsRead {
    fn of(held: &Loop) -> Self {
        Self {
            id: held.id.as_str().to_owned(),
            name: held.name.clone(),
            unreviewed: held.is_unreviewed,
        }
    }

    async fn read_through(
        _transaction: &mut Transaction,
        held: &Loop,
    ) -> Result<Response, StoreError> {
        Ok(Json(Self::of(held)).into_response())
    }
}

/// What the console sends to create a loop. A name, and nothing else there could be.
#[derive(Deserialize)]
pub(in crate::transport) struct Creating {
    name: String,
}

/// What the console sends to rename one.
#[derive(Deserialize)]
pub(in crate::transport) struct Edit {
    name: Option<String>,
}

/// What the console sends to set the base order: every loop, once, in the order it wants.
#[derive(Deserialize)]
pub(in crate::transport) struct Order {
    order: Vec<String>,
}

/// Read every loop, in the base order. `SystemAdministration`. A read, so it is not audited.
pub(in crate::transport) async fn list(State(api): State<Api>) -> Response {
    answers::or_unavailable(read_all(&api).await)
}

async fn read_all(api: &Api) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let read = transaction.loops().await;
    transaction.roll_back().await?;

    let held: Vec<LoopAsRead> = read?.iter().map(LoopAsRead::of).collect();

    Ok(Json(held).into_response())
}

/// Read one loop. `SystemAdministration`. A read, so it is not audited.
pub(in crate::transport) async fn read(State(api): State<Api>, Path(id): Path<String>) -> Response {
    answers::or_unavailable(read_one(&api, &LoopId::presented(id)).await)
}

async fn read_one(api: &Api, id: &LoopId) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let read = transaction.a_loop(id).await;
    transaction.roll_back().await?;

    Ok(match read? {
        None => answers::no_such("loop"),
        Some(held) => Json(LoopAsRead::of(&held)).into_response(),
    })
}

/// Create a loop. `SystemAdministration`, audited — refused or not.
///
/// It arrives `unreviewed` and at the end of the base order, neither of which the caller
/// chooses.
pub(in crate::transport) async fn create_loop(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Json(new): Json<Creating>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };

    answers::or_unavailable(
        create(
            &api,
            acting,
            AuditEvent::LoopCreated,
            new.name.clone(),
            async |transaction: &mut Transaction| {
                let id = transaction.create_loop(&new.name).await?;

                Ok(transaction.a_loop(&id).await?)
            },
            async |_transaction: &mut Transaction, made: &Loop| {
                Ok((StatusCode::CREATED, Json(LoopAsRead::of(made))).into_response())
            },
        )
        .await,
    )
}

/// Rename a loop. `SystemAdministration`, audited — refused or not.
pub(in crate::transport) async fn edit(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    Json(edit): Json<Edit>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };
    let Some(name) = edit.name else {
        return answers::cannot("That edit asks for no change.");
    };

    let target = LoopId::presented(id);

    administering(
        &api,
        acting,
        AuditEvent::LoopEdited,
        &target,
        async |transaction: &mut Transaction| transaction.rename_loop(&target, &name).await,
    )
    .await
}

/// Delete a loop. `SystemAdministration`, audited — refused or not.
///
/// The loops around it keep their places in the base order.
pub(in crate::transport) async fn delete(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };
    let target = LoopId::presented(id);

    administering(
        &api,
        acting,
        AuditEvent::LoopDeleted,
        &target,
        async |transaction: &mut Transaction| Ok(transaction.delete_loop(&target).await?),
    )
    .await
}

/// Every write to a loop record, on the one audited path, answering with the loop.
async fn administering(
    api: &Api,
    acting: &UserId,
    event: AuditEvent,
    target: &LoopId,
    write: impl AsyncFnOnce(&mut Transaction) -> Result<Option<Change<Loop>>, AdministrationRefused>,
) -> Response {
    answers::or_unavailable(
        administer(
            api,
            acting,
            event,
            "loop",
            async |transaction: &mut Transaction| transaction.a_loop(target).await,
            write,
            LoopAsRead::read_through,
        )
        .await,
    )
}

/// Set the deployment-wide base loop order. `SystemAdministration`, audited — refused or not.
///
/// It is the one configuration write about no single record, so it is the one that does not
/// go through [`administer`]: the order is a fact about every loop at once, and the entry
/// names none of them as its target. What it records is the order before and the order
/// after, by name, because *did they mean to put `THERMAL` first* is the question a reader
/// of that entry has.
pub(in crate::transport) async fn set_order(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Json(asked): Json<Order>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };

    answers::or_unavailable(ordering(&api, acting, asked.order).await)
}

async fn ordering(api: &Api, acting: &UserId, order: Vec<String>) -> Result<Response, StoreError> {
    let asked: Vec<LoopId> = order.into_iter().map(LoopId::presented).collect();

    let mut transaction = api.store.begin().await?;
    let administrator = Administrator::of(&mut transaction, acting).await?;
    let before = transaction.loops().await?;

    let after = match transaction.set_the_loop_order(&asked).await {
        Ok(after) => after,
        Err(AdministrationRefused::Store(error)) => return Err(error),
        Err(refusal) => {
            transaction.roll_back().await?;

            return refuse(
                api,
                &administrator,
                AuditEvent::LoopOrderEdited,
                the_order_wrote(&before, None, Some(reason(&refusal))),
                answer_to(&refusal),
            )
            .await;
        }
    };

    transaction
        .record(administrator.wrote(
            AuditEvent::LoopOrderEdited,
            the_order_wrote(&before, Some(&after), None),
        ))
        .await?;
    transaction.commit().await?;

    tracing::info!(target: module::CONFIGURATION, "the base loop order was set");

    let ordered: Vec<LoopAsRead> = after.iter().map(LoopAsRead::of).collect();

    Ok(Json(ordered).into_response())
}

/// What setting the order did, as the log holds it.
fn the_order_wrote(
    before: &[Loop],
    after: Option<&[Loop]>,
    refusal: Option<String>,
) -> ConfigurationWrite {
    ConfigurationWrite {
        // No target: the order is about every loop rather than any one of them, and naming
        // one of them would be the entry claiming the write was about that loop.
        target: None,
        target_name: "the deployment loop order".to_owned(),
        before: Some(Snapshot::of_the_loop_order(before)),
        after: after.map(Snapshot::of_the_loop_order),
        blast_radius: nothing_live(),
        refusal,
    }
}

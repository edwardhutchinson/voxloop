//! The grid: reading a role's row, reading a loop's column, setting one cell, and ruling on
//! a loop.
//!
//! **The console reads one row at a time** ([ADR-0015]). A role page is the row — *what can
//! this role reach* — and a loop page is the column — *who may hear this loop*; both are the
//! primary surface, and both are a list at full size rather than a wall of small squares.
//! The matrix is here too, as a **secondary reference view**: reading a whole configuration
//! at once is a reviewing act, so it is one read and no writes.
//!
//! A cell is one value from an ordered four and there is no second layer ([ADR-0011]), which
//! is why there is one write here and no *clear*, no *grant*, no *deny* and no per-user
//! anything. Setting `none` is how a permission is taken away.
//!
//! **Dismissing a loop's `unreviewed` mark lives here** rather than with the loop record,
//! because what it writes is cells: a deliberate `none` against every role nobody ruled on.
//! It is per loop, never per cell (v1 §9).
//!
//! [ADR-0011]: ../../../docs/adr/0011-a-permission-is-one-cell-on-the-grid.md
//! [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};

use super::loops::LoopAsRead;
use super::roles::RoleAsRead;
use super::{acting, administer, unreachable_caller};
use crate::authorisation::Caller;
use crate::configuration::{
    AuditEvent, Cell, Grid, LoopId, Loops, Permission, RoleId, Roles, StoreError, Transaction,
};
use crate::transport::{Api, answers};

/// One cell, as a write answers with one: the pair, and the one value it holds.
///
/// Both halves, because a cell is set from either page and the answer says what the write
/// landed on rather than what the caller happened to be looking at.
#[derive(Serialize)]
struct CellAsRead {
    role: RoleAsRead,
    #[serde(rename = "loop")]
    held_on: LoopAsRead,
    /// `none`, `monitor`, `emit` or `control` — the same four words the store holds and the
    /// audit log reads back, so nothing has to be translated to be talked about.
    permission: &'static str,
}

/// One cell of a role's row: the loop, and what the role holds on it.
///
/// The role is on the page already, and repeating it against every loop would make the row
/// say its own subject twenty times.
#[derive(Serialize)]
struct OnALoop {
    #[serde(rename = "loop")]
    held_on: LoopAsRead,
    permission: &'static str,
}

/// One cell of a loop's column: the role, and what it holds on the loop.
#[derive(Serialize)]
struct ByARole {
    role: RoleAsRead,
    permission: &'static str,
}

impl CellAsRead {
    fn of(cell: &Cell) -> Self {
        Self {
            role: RoleAsRead::of(&cell.role),
            held_on: LoopAsRead::of(&cell.held_on),
            permission: cell.permission.as_str(),
        }
    }

    async fn read_through(
        _transaction: &mut Transaction,
        cell: &Cell,
    ) -> Result<Response, StoreError> {
        Ok(Json(Self::of(cell)).into_response())
    }
}

/// A role's row: the role, and every loop in the base order with what it holds on each.
#[derive(Serialize)]
struct RowAsRead {
    role: RoleAsRead,
    cells: Vec<OnALoop>,
}

/// A loop's column: the loop, and every role by name with what it holds on it.
#[derive(Serialize)]
struct ColumnAsRead {
    #[serde(rename = "loop")]
    held_on: LoopAsRead,
    cells: Vec<ByARole>,
}

/// The whole grid, for the reference view.
///
/// The axes come back beside the cells rather than being inferred from them, because a
/// deployment with no loops has no cells and a matrix that vanished at that moment would be
/// the console guessing at what it was reading.
#[derive(Serialize)]
struct MatrixAsRead {
    roles: Vec<RoleAsRead>,
    loops: Vec<LoopAsRead>,
    cells: Vec<HeldAsRead>,
}

/// One cell of the matrix, by the ids of its pair.
///
/// The names are on the axes already, and a whole-grid read repeats every cell across both
/// of them.
#[derive(Serialize)]
struct HeldAsRead {
    role: String,
    #[serde(rename = "loop")]
    held_on: String,
    permission: &'static str,
}

/// What the console sends to set a cell: the one value it is to hold.
#[derive(Deserialize)]
pub(in crate::transport) struct Setting {
    permission: String,
}

/// Read a role's row. `SystemAdministration`. A read, so it is not audited.
pub(in crate::transport) async fn row(State(api): State<Api>, Path(id): Path<String>) -> Response {
    answers::or_unavailable(read_the_row(&api, &RoleId::presented(id)).await)
}

async fn read_the_row(api: &Api, role: &RoleId) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let read = async {
        let Some(cells) = transaction.the_row_of(role).await? else {
            return Ok(None);
        };

        Ok(transaction.role(role).await?.map(|role| RowAsRead {
            role: RoleAsRead::of(&role),
            cells: cells
                .iter()
                .map(|cell| OnALoop {
                    held_on: LoopAsRead::of(&cell.held_on),
                    permission: cell.permission.as_str(),
                })
                .collect(),
        }))
    }
    .await;
    transaction.roll_back().await?;

    Ok(match read? {
        None => answers::no_such("role"),
        Some(row) => Json(row).into_response(),
    })
}

/// Read a loop's column. `SystemAdministration`. A read, so it is not audited.
pub(in crate::transport) async fn column(
    State(api): State<Api>,
    Path(id): Path<String>,
) -> Response {
    answers::or_unavailable(read_the_column(&api, &LoopId::presented(id)).await)
}

async fn read_the_column(api: &Api, held_on: &LoopId) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let read = async {
        let Some(cells) = transaction.the_column_of(held_on).await? else {
            return Ok(None);
        };

        Ok(transaction
            .a_loop(held_on)
            .await?
            .map(|held_on| ColumnAsRead {
                held_on: LoopAsRead::of(&held_on),
                cells: cells
                    .iter()
                    .map(|cell| ByARole {
                        role: RoleAsRead::of(&cell.role),
                        permission: cell.permission.as_str(),
                    })
                    .collect(),
            }))
    }
    .await;
    transaction.roll_back().await?;

    Ok(match read? {
        None => answers::no_such("loop"),
        Some(column) => Json(column).into_response(),
    })
}

/// Read the whole grid. `SystemAdministration`. A read, so it is not audited.
///
/// The reference view, and the only place a whole-configuration read is possible ([ADR-0015]).
///
/// [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md
pub(in crate::transport) async fn matrix(State(api): State<Api>) -> Response {
    answers::or_unavailable(read_the_matrix(&api).await)
}

async fn read_the_matrix(api: &Api) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let read = async {
        Ok::<_, StoreError>(MatrixAsRead {
            roles: transaction
                .roles()
                .await?
                .iter()
                .map(RoleAsRead::of)
                .collect(),
            loops: transaction
                .loops()
                .await?
                .iter()
                .map(LoopAsRead::of)
                .collect(),
            cells: transaction
                .the_whole_grid()
                .await?
                .iter()
                .map(|cell| HeldAsRead {
                    role: cell.role.id.as_str().to_owned(),
                    held_on: cell.held_on.id.as_str().to_owned(),
                    permission: cell.permission.as_str(),
                })
                .collect(),
        })
    }
    .await;
    transaction.roll_back().await?;

    Ok(Json(read?).into_response())
}

/// Set one cell. `SystemAdministration`, audited with before and after.
///
/// The pair is the whole address of a cell, so it is named in the path and the body carries
/// the one value. Granting and revoking are this operation twice with different words in it.
pub(in crate::transport) async fn set(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path((role, held_on)): Path<(String, String)>,
    Json(asked): Json<Setting>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };
    let Some(permission) = Permission::named(&asked.permission) else {
        return answers::cannot(
            "A permission is one of none, monitor, emit or control, each carrying those below it.",
        );
    };

    let role = RoleId::presented(role);
    let held_on = LoopId::presented(held_on);

    answers::or_unavailable(
        administer(
            &api,
            acting,
            AuditEvent::GridCellEdited,
            // Either half of the pair can be the one that is not there, and the answer says
            // so without guessing which: the console holds both ids and read both lists.
            "role or loop",
            async |transaction: &mut Transaction| transaction.a_cell(&role, &held_on).await,
            async |transaction: &mut Transaction| {
                Ok(transaction.set_cell(&role, &held_on, permission).await?)
            },
            CellAsRead::read_through,
        )
        .await,
    )
}

/// Dismiss a loop's `unreviewed` mark. `SystemAdministration`, audited with before and after.
///
/// It is the administrator saying *I have ruled on this column*, and what it records is what
/// they ruled: a deliberate `none` against every role they left alone. The mark is a display
/// state throughout — the evaluator enforced `none` on every one of these cells already, and
/// goes on doing exactly what it did.
pub(in crate::transport) async fn dismiss_unreviewed(
    State(api): State<Api>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> Response {
    let Some(acting) = acting(&caller) else {
        return unreachable_caller();
    };
    let target = LoopId::presented(id);

    answers::or_unavailable(
        administer(
            &api,
            acting,
            AuditEvent::LoopReviewed,
            "loop",
            async |transaction: &mut Transaction| transaction.a_loop(&target).await,
            async |transaction: &mut Transaction| {
                Ok(transaction.dismiss_unreviewed(&target).await?)
            },
            LoopAsRead::read_through,
        )
        .await,
    )
}

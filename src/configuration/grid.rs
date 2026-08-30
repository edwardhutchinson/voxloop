//! The grid: one permission per (role, loop) pair, and the only place voice authority is
//! configured ([ADR-0011]).
//!
//! Everything about the model is in one sentence: a cell holds a single value from an
//! ordered four — `none`, `monitor`, `emit`, `control` — each rung carrying those below it,
//! and an absent cell is `none`. There is **no second layer**: no per-user grant, no
//! per-user deny, no explicit deny beating a grant, no exception and no precedence rule.
//! Anything of the kind would make evaluation two lookups that can disagree, and would mean
//! a loop's column is never the whole answer to *who may hear this*.
//!
//! Two absences are worth naming, because both look like omissions and neither is:
//!
//! - **A deliberate `none` and an absent cell are the same permission.** The store can tell
//!   them apart — one is a row — and nothing that decides anything is allowed to. The
//!   difference exists so that an administrator can be prompted about a loop nobody has
//!   ruled on ([ADR-0015]), and a prompt is not an input to a permission decision.
//! - **An unreviewed loop is enforced as `none` on every rung**, whatever its cells say
//!   (v1 §3). [`Grid::cell`] — the evaluator's lookup — applies that; the reads the console
//!   works from do not, because an administrator ruling on a column has to see what they
//!   have set so far.
//!
//! The console reads this one row or one column at a time ([ADR-0015]): a role page is the
//! row and a loop page is the column. Both are the same list of cells in a different order,
//! which is why they are one type here and not two.
//!
//! [ADR-0011]: ../../../docs/adr/0011-a-permission-is-one-cell-on-the-grid.md
//! [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md

use async_trait::async_trait;
use sqlx::Row;

use super::loops::{Loop, LoopId, Loops, a_loop};
use super::records::Change;
use super::roles::{Role, RoleId, Roles, a_role};
use super::store::{StoreError, Transaction, now, unavailable};

/// The single value a (role, loop) pair holds.
///
/// The four are **ordered**, and the ordering is the whole of the model's expressiveness:
/// each rung carries everything below it, so a role holding `control` may also emit and
/// monitor. `Ord` is derived from the order they are written in, and that order is the
/// ladder — reordering these lines silently changes what every deployment permits.
///
/// Listen and emit are deliberately not independent axes ([ADR-0011]): emit-without-monitor
/// is a hazard, because an operator armed on a loop they cannot hear cannot tell they are
/// talking over somebody and cannot hear the reply.
///
/// [ADR-0011]: ../../../docs/adr/0011-a-permission-is-one-cell-on-the-grid.md
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Permission {
    /// Nothing. What an absent cell means, and what a deliberate `none` means.
    #[default]
    None,
    /// May subscribe and hear.
    Monitor,
    /// May arm, key, key priority, hail and use presets.
    Emit,
    /// Operational authority on that loop ([ADR-0012]).
    ///
    /// [ADR-0012]: ../../../docs/adr/0012-operational-authority-is-the-control-rung.md
    Control,
}

impl Permission {
    /// Whether this permission carries `rung`, which is the whole of what the order is for.
    pub(crate) fn carries(self, rung: Self) -> bool {
        self >= rung
    }

    /// The word this permission is known by, on the wire and in the store alike.
    ///
    /// One set of words rather than two: `emit` is what the console sends, what the store
    /// holds and what the audit log reads back, so a deployment's file and its API say the
    /// same thing to whoever is reading either by hand.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Monitor => "monitor",
            Self::Emit => "emit",
            Self::Control => "control",
        }
    }

    /// The permission that word names, where it names one.
    pub(crate) fn named(word: &str) -> Option<Self> {
        match word {
            "none" => Some(Self::None),
            "monitor" => Some(Self::Monitor),
            "emit" => Some(Self::Emit),
            "control" => Some(Self::Control),
            _ => None,
        }
    }
}

/// A value the grid holds that this binary cannot read back.
///
/// The schema's `CHECK` refuses anything else, so only a hand-edited store produces one. It
/// is a fault to report rather than a value to guess at: a permission read that fell back to
/// `none` would turn a corrupt store into a silent outage, and one that fell back to
/// anything else does not bear thinking about.
#[derive(Debug, thiserror::Error)]
#[error("the grid holds a permission this binary does not know: {0:?}")]
struct Unreadable(String);

/// One cell: what a role holds on a loop.
///
/// It carries both records rather than their ids, because every reader of a cell wants the
/// names — the console renders them and the audit log snapshots them — and a cell read
/// without them is a pair of opaque strings nobody can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Cell {
    pub(crate) role: Role,
    pub(crate) held_on: Loop,
    pub(crate) permission: Permission,
}

/// The grid, as domain operations rather than queries ([ADR-0038]).
///
/// [ADR-0038]: ../../../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md
#[async_trait]
pub(crate) trait Grid {
    /// What this role holds on this loop, as the evaluator asks it: **one lookup**.
    ///
    /// This is the read every permission decision in VoxLoop is made from, so it is the one
    /// that enforces the rules rather than reporting them. An absent cell is `none`, a loop
    /// nobody holds is `none`, and an unreviewed loop is `none` whatever its cells say —
    /// and none of the three is distinguishable from the others in the answer.
    ///
    /// [`Grid::a_cell`] is the other read of the same cell, and the difference between them
    /// is the whole of what `unreviewed` is: this one decides, that one reports.
    async fn held_by(&mut self, role: &RoleId, held_on: &LoopId) -> Result<Permission, StoreError>;

    /// One cell as an administrator set it, or nothing where the pair names a record that is
    /// not there.
    ///
    /// It reports rather than decides, so it does **not** enforce an unreviewed loop as
    /// `none`: the console has to show what has been set so far for ruling on a column to be
    /// possible, and an audit entry saying `none` about a cell somebody just set to `control`
    /// would be a lie about what they did.
    async fn a_cell(&mut self, role: &RoleId, held_on: &LoopId)
    -> Result<Option<Cell>, StoreError>;

    /// Set a cell, answering with what it held and what it holds now.
    ///
    /// There is no *clear a cell*: setting `none` is how a permission is taken away, and it
    /// is deliberately the same write as granting one — a cell always holds exactly one of
    /// the four, and the row's presence carries no meaning of its own.
    ///
    /// A pair naming a role or a loop that is not there is no change rather than a refusal,
    /// exactly as a write against any other id nobody holds is.
    async fn set_cell(
        &mut self,
        role: &RoleId,
        held_on: &LoopId,
        permission: Permission,
    ) -> Result<Option<Change<Cell>>, StoreError>;

    /// A role's row: every loop in the base order, with what this role holds on it.
    ///
    /// This is a role page ([ADR-0015]), and it answers *what can this role reach*. It is
    /// every loop rather than the ones with cells, because a row read as a list has to show
    /// the loops this role cannot reach for the list to mean anything.
    ///
    /// [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md
    async fn the_row_of(&mut self, role: &RoleId) -> Result<Option<Vec<Cell>>, StoreError>;

    /// A loop's column: every role by name, with what it holds on this loop.
    ///
    /// This is a loop page, and it answers *who may hear this loop*.
    async fn the_column_of(&mut self, held_on: &LoopId) -> Result<Option<Vec<Cell>>, StoreError>;

    /// Every cell on the deployment, by role and then by the base loop order.
    ///
    /// The matrix is a **secondary reference view** ([ADR-0015]): checking the shape of a
    /// configuration — a role with no reach, a loop nobody can hear — is a reviewing act,
    /// and administering is done a row at a time.
    async fn the_whole_grid(&mut self) -> Result<Vec<Cell>, StoreError>;

    /// Dismiss a loop's unreviewed mark, recording a deliberate `none` for every role
    /// nobody has ruled on.
    ///
    /// It is **per loop, not per cell** (v1 §9): the act is an administrator saying *I have
    /// ruled on this column*, and what it writes is what they ruled — a `none` against every
    /// role they left alone, which is a decision rather than an absence from that moment on.
    async fn dismiss_unreviewed(
        &mut self,
        held_on: &LoopId,
    ) -> Result<Option<Change<Loop>>, StoreError>;
}

#[async_trait]
impl Grid for Transaction {
    async fn held_by(&mut self, role: &RoleId, held_on: &LoopId) -> Result<Permission, StoreError> {
        let found = sqlx::query(
            // One statement, and every rule in it. An unreviewed loop is answered as `none`
            // here rather than by a caller remembering to ask: a caller that forgets is a
            // caller that grants reach on a loop nobody has ruled on.
            "SELECT CASE WHEN loops.is_unreviewed <> 0 THEN 'none' \
                         ELSE COALESCE(grid_cells.permission, 'none') END AS permission \
             FROM loops \
             LEFT JOIN grid_cells \
               ON grid_cells.loop_id = loops.id AND grid_cells.role_id = ? \
             WHERE loops.id = ?",
        )
        .bind(role.as_str())
        .bind(held_on.as_str())
        .fetch_optional(self.connection())
        .await
        .map_err(unavailable)?;

        // No loop, no reach. A loop that has been deleted answers exactly as one that never
        // existed, which is what stops a stale id in a client's hands meaning anything.
        let Some(row) = found else {
            return Ok(Permission::None);
        };

        a_permission(&row, "permission")
    }

    async fn set_cell(
        &mut self,
        role: &RoleId,
        held_on: &LoopId,
        permission: Permission,
    ) -> Result<Option<Change<Cell>>, StoreError> {
        let Some(before) = self.a_cell(role, held_on).await? else {
            return Ok(None);
        };

        sqlx::query(
            "INSERT INTO grid_cells (role_id, loop_id, permission, set_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT (role_id, loop_id) \
             DO UPDATE SET permission = excluded.permission, set_at = excluded.set_at",
        )
        .bind(role.as_str())
        .bind(held_on.as_str())
        .bind(permission.as_str())
        .bind(now())
        .execute(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(Some(Change {
            before,
            // Read back through the same transaction, so the entry records what the store
            // holds rather than what the caller asked for.
            after: self.a_cell(role, held_on).await?,
        }))
    }

    async fn the_row_of(&mut self, role: &RoleId) -> Result<Option<Vec<Cell>>, StoreError> {
        let Some(role) = self.role(role).await? else {
            return Ok(None);
        };

        let rows = sqlx::query(
            "SELECT loops.id AS id, loops.name AS name, loops.is_unreviewed AS is_unreviewed, \
                    COALESCE(grid_cells.permission, 'none') AS permission \
             FROM loops \
             LEFT JOIN grid_cells \
               ON grid_cells.loop_id = loops.id AND grid_cells.role_id = ? \
             ORDER BY loops.position, loops.created_at, loops.id",
        )
        .bind(role.id.as_str())
        .fetch_all(self.connection())
        .await
        .map_err(unavailable)?;

        rows.iter()
            .map(|row| {
                Ok(Cell {
                    role: role.clone(),
                    held_on: a_loop(row),
                    permission: a_permission(row, "permission")?,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    async fn the_column_of(&mut self, held_on: &LoopId) -> Result<Option<Vec<Cell>>, StoreError> {
        let Some(held_on) = self.a_loop(held_on).await? else {
            return Ok(None);
        };

        let rows = sqlx::query(
            "SELECT roles.id AS id, roles.name AS name, roles.max_occupants AS max_occupants, \
                    COALESCE(grid_cells.permission, 'none') AS permission \
             FROM roles \
             LEFT JOIN grid_cells \
               ON grid_cells.role_id = roles.id AND grid_cells.loop_id = ? \
             ORDER BY roles.name",
        )
        .bind(held_on.id.as_str())
        .fetch_all(self.connection())
        .await
        .map_err(unavailable)?;

        rows.iter()
            .map(|row| {
                Ok(Cell {
                    role: a_role(row),
                    held_on: held_on.clone(),
                    permission: a_permission(row, "permission")?,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    async fn the_whole_grid(&mut self) -> Result<Vec<Cell>, StoreError> {
        let rows = sqlx::query(
            "SELECT roles.id AS role_id, roles.name AS role_name, \
                    roles.max_occupants AS max_occupants, \
                    loops.id AS loop_id, loops.name AS loop_name, \
                    loops.is_unreviewed AS is_unreviewed, \
                    COALESCE(grid_cells.permission, 'none') AS permission \
             FROM roles \
             CROSS JOIN loops \
             LEFT JOIN grid_cells \
               ON grid_cells.role_id = roles.id AND grid_cells.loop_id = loops.id \
             ORDER BY roles.name, loops.position, loops.created_at, loops.id",
        )
        .fetch_all(self.connection())
        .await
        .map_err(unavailable)?;

        rows.iter()
            .map(|row| {
                Ok(Cell {
                    role: Role {
                        id: RoleId::known(row.get("role_id")),
                        name: row.get("role_name"),
                        max_occupants: super::roles::a_limit(row),
                    },
                    held_on: Loop {
                        id: LoopId::known(row.get("loop_id")),
                        name: row.get("loop_name"),
                        is_unreviewed: row.get::<i64, _>("is_unreviewed") != 0,
                    },
                    permission: a_permission(row, "permission")?,
                })
            })
            .collect()
    }

    async fn dismiss_unreviewed(
        &mut self,
        held_on: &LoopId,
    ) -> Result<Option<Change<Loop>>, StoreError> {
        let Some(before) = self.a_loop(held_on).await? else {
            return Ok(None);
        };

        // Every role nobody has ruled on, ruled on now. A `none` written here is deliberate
        // in the only sense the word can have: somebody was shown the column and left it.
        sqlx::query(
            "INSERT INTO grid_cells (role_id, loop_id, permission, set_at) \
             SELECT roles.id, ?, 'none', ? FROM roles \
             WHERE NOT EXISTS ( \
                 SELECT 1 FROM grid_cells \
                 WHERE grid_cells.role_id = roles.id AND grid_cells.loop_id = ? \
             )",
        )
        .bind(held_on.as_str())
        .bind(now())
        .bind(held_on.as_str())
        .execute(self.connection())
        .await
        .map_err(unavailable)?;

        sqlx::query("UPDATE loops SET is_unreviewed = 0 WHERE id = ?")
            .bind(held_on.as_str())
            .execute(self.connection())
            .await
            .map_err(unavailable)?;

        Ok(Some(Change {
            before,
            after: self.a_loop(held_on).await?,
        }))
    }

    async fn a_cell(
        &mut self,
        role: &RoleId,
        held_on: &LoopId,
    ) -> Result<Option<Cell>, StoreError> {
        let (Some(role), Some(held_on)) = (self.role(role).await?, self.a_loop(held_on).await?)
        else {
            return Ok(None);
        };

        let held: Option<String> = sqlx::query_scalar(
            "SELECT permission FROM grid_cells WHERE role_id = ? AND loop_id = ?",
        )
        .bind(role.id.as_str())
        .bind(held_on.id.as_str())
        .fetch_optional(self.connection())
        .await
        .map_err(unavailable)?;

        let permission = match held {
            None => Permission::None,
            Some(word) => Permission::named(&word).ok_or_else(|| unavailable(Unreadable(word)))?,
        };

        Ok(Some(Cell {
            role,
            held_on,
            permission,
        }))
    }
}

/// The permission a column of a row holds.
fn a_permission(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<Permission, StoreError> {
    let word: String = row.get(column);

    Permission::named(&word).ok_or_else(|| unavailable(Unreadable(word)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::roles::NewRole;
    use crate::configuration::store::a_temporary_store;

    /// A deployment with a role and a loop on it, and the loop already ruled on.
    ///
    /// Most of what is tested here is about a cell rather than about the review mark, and a
    /// loop nobody has ruled on answers `none` on every rung whatever its cells say — which
    /// would make every one of those tests pass for the wrong reason.
    async fn a_role_and_a_reviewed_loop(
        transaction: &mut Transaction,
        role: &str,
        held_on: &str,
    ) -> (RoleId, LoopId) {
        let role = transaction
            .create_role(NewRole {
                name: role.to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the role to be created");
        let held_on = transaction
            .create_loop(held_on)
            .await
            .expect("the loop to be created");
        transaction
            .dismiss_unreviewed(&held_on)
            .await
            .expect("the loop to be ruled on");

        (role, held_on)
    }

    #[tokio::test]
    async fn the_rungs_are_ordered_and_each_carries_everything_below_it() {
        let ladder = [
            Permission::None,
            Permission::Monitor,
            Permission::Emit,
            Permission::Control,
        ];

        for (above, held) in ladder.iter().enumerate() {
            for (below, rung) in ladder.iter().enumerate() {
                assert_eq!(
                    held.carries(*rung),
                    above >= below,
                    "{held:?} answered the wrong thing about carrying {rung:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_cell_nobody_has_set_is_none() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;

        assert_eq!(
            transaction
                .held_by(&role, &held_on)
                .await
                .expect("the lookup to answer"),
            Permission::None
        );
    }

    #[tokio::test]
    async fn sets_a_cell_and_answers_with_what_it_held_and_what_it_holds_now() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;

        let change = transaction
            .set_cell(&role, &held_on, Permission::Control)
            .await
            .expect("the cell to be set")
            .expect("a change");

        assert_eq!(change.before.permission, Permission::None);
        let after = change.after.expect("the cell after");
        assert_eq!(after.permission, Permission::Control);
        assert_eq!(after.role.name, "Flight Director");
        assert_eq!(after.held_on.name, "FLIGHT");
        assert_eq!(
            transaction
                .held_by(&role, &held_on)
                .await
                .expect("the lookup to answer"),
            Permission::Control
        );
    }

    /// There is no *clear a cell*: taking a permission away is setting `none`, which is the
    /// same write as granting one and lands in the same one value.
    #[tokio::test]
    async fn taking_a_permission_away_is_setting_none() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        transaction
            .set_cell(&role, &held_on, Permission::Emit)
            .await
            .expect("the cell to be set");

        let change = transaction
            .set_cell(&role, &held_on, Permission::None)
            .await
            .expect("the cell to be set")
            .expect("a change");

        assert_eq!(change.before.permission, Permission::Emit);
        assert_eq!(
            change.after.expect("the cell after").permission,
            Permission::None
        );
    }

    /// The lookup every permission decision is made from cannot tell a deliberate `none`
    /// from a cell nobody ever set, and must not be able to (v1 §3).
    #[tokio::test]
    async fn a_deliberate_none_reads_exactly_as_a_cell_nobody_set() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (ruled_on, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        let untouched = transaction
            .create_role(NewRole {
                name: "Support Engineer".to_owned(),
                max_occupants: None,
            })
            .await
            .expect("the second role");
        transaction
            .set_cell(&ruled_on, &held_on, Permission::None)
            .await
            .expect("the deliberate none to be recorded");

        assert_eq!(
            transaction
                .held_by(&ruled_on, &held_on)
                .await
                .expect("the lookup to answer"),
            transaction
                .held_by(&untouched, &held_on)
                .await
                .expect("the lookup to answer"),
        );
    }

    /// An unreviewed loop is `none` on every rung, whatever its cells say (v1 §3).
    #[tokio::test]
    async fn an_unreviewed_loop_is_enforced_as_none_whatever_its_cells_hold() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let role = transaction
            .create_role(NewRole {
                name: "Flight Director".to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the role");
        let held_on = transaction.create_loop("FLIGHT").await.expect("the loop");
        transaction
            .set_cell(&role, &held_on, Permission::Control)
            .await
            .expect("the cell to be set");

        assert_eq!(
            transaction
                .held_by(&role, &held_on)
                .await
                .expect("the lookup to answer"),
            Permission::None,
            "an unreviewed loop conferred the reach its cells were set to"
        );

        transaction
            .dismiss_unreviewed(&held_on)
            .await
            .expect("the mark to be dismissed");

        assert_eq!(
            transaction
                .held_by(&role, &held_on)
                .await
                .expect("the lookup to answer"),
            Permission::Control,
            "ruling on the loop did not release what was already set"
        );
    }

    /// Dismissing is **per loop**: it clears that loop's mark and records a deliberate
    /// `none` for every role nobody ruled on, leaving the cells that were set alone.
    #[tokio::test]
    async fn dismissing_unreviewed_is_per_loop_and_records_a_deliberate_none() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let ruled_on = transaction
            .create_role(NewRole {
                name: "Flight Director".to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the role");
        let flight = transaction.create_loop("FLIGHT").await.expect("the loop");
        let gnc = transaction.create_loop("GNC").await.expect("a second loop");
        transaction
            .set_cell(&ruled_on, &flight, Permission::Emit)
            .await
            .expect("the cell to be set");

        let change = transaction
            .dismiss_unreviewed(&flight)
            .await
            .expect("the mark to be dismissed")
            .expect("a change");

        assert!(change.before.is_unreviewed);
        assert!(!change.after.expect("the loop after").is_unreviewed);
        assert!(
            transaction
                .a_loop(&gnc)
                .await
                .expect("a read")
                .expect("the loop")
                .is_unreviewed,
            "dismissing one loop's mark cleared another's"
        );

        let column = transaction
            .the_column_of(&flight)
            .await
            .expect("the column to be read")
            .expect("a column");
        let held: Vec<(&str, Permission)> = column
            .iter()
            .map(|cell| (cell.role.name.as_str(), cell.permission))
            .collect();
        assert_eq!(
            held,
            [
                ("Flight Director", Permission::Emit),
                ("Observer", Permission::None)
            ],
            "the roles nobody ruled on were not recorded as deliberate nones"
        );
    }

    /// A role page is the row: every loop, in the base order, with what this role holds.
    #[tokio::test]
    async fn a_role_row_is_every_loop_in_the_base_order() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, gnc) = a_role_and_a_reviewed_loop(&mut transaction, "GNC Officer", "GNC").await;
        let flight = transaction.create_loop("FLIGHT").await.expect("a loop");
        transaction
            .set_the_loop_order(&[flight.clone(), gnc.clone()])
            .await
            .expect("the order to be set");
        transaction
            .set_cell(&role, &gnc, Permission::Control)
            .await
            .expect("the cell to be set");

        let row = transaction
            .the_row_of(&role)
            .await
            .expect("the row to be read")
            .expect("a row");

        let held: Vec<(&str, Permission)> = row
            .iter()
            .map(|cell| (cell.held_on.name.as_str(), cell.permission))
            .collect();
        assert_eq!(
            held,
            [("FLIGHT", Permission::None), ("GNC", Permission::Control)],
            "a role's row was not every loop in the base order"
        );
        assert!(row.iter().all(|cell| cell.role.name == "GNC Officer"));
    }

    /// A loop page is the column: every role, by name, with what it holds on this loop.
    #[tokio::test]
    async fn a_loop_column_is_every_role_by_name() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        transaction
            .set_cell(&role, &held_on, Permission::Monitor)
            .await
            .expect("the cell to be set");

        let column = transaction
            .the_column_of(&held_on)
            .await
            .expect("the column to be read")
            .expect("a column");

        let held: Vec<(&str, Permission)> = column
            .iter()
            .map(|cell| (cell.role.name.as_str(), cell.permission))
            .collect();
        assert_eq!(
            held,
            [
                ("Flight Director", Permission::Monitor),
                ("Observer", Permission::None)
            ]
        );
    }

    /// The matrix is the same cells read whole, which is what makes it a reference view of
    /// the pages rather than a second source of truth.
    #[tokio::test]
    async fn the_whole_grid_is_every_role_against_every_loop() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        transaction.create_loop("GNC").await.expect("a second loop");
        transaction
            .set_cell(&role, &held_on, Permission::Emit)
            .await
            .expect("the cell to be set");

        let grid = transaction
            .the_whole_grid()
            .await
            .expect("the grid to be read");

        let held: Vec<(&str, &str, Permission)> = grid
            .iter()
            .map(|cell| {
                (
                    cell.role.name.as_str(),
                    cell.held_on.name.as_str(),
                    cell.permission,
                )
            })
            .collect();
        assert_eq!(
            held,
            [
                ("Flight Director", "FLIGHT", Permission::Emit),
                ("Flight Director", "GNC", Permission::None),
                ("Observer", "FLIGHT", Permission::None),
                ("Observer", "GNC", Permission::None),
            ]
        );
    }

    /// A cell is only about its two records, so it goes when either of them does.
    #[tokio::test]
    async fn deleting_a_role_or_a_loop_takes_its_cells_with_it() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        transaction
            .set_cell(&role, &held_on, Permission::Control)
            .await
            .expect("the cell to be set");

        transaction
            .delete_role(&role)
            .await
            .expect("the role to be deleted");

        assert_eq!(
            transaction
                .the_column_of(&held_on)
                .await
                .expect("the column to be read")
                .expect("a column")
                .len(),
            1,
            "a deleted role left its cells behind"
        );
    }

    #[tokio::test]
    async fn a_write_naming_a_role_or_a_loop_nobody_holds_is_no_change() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        let nobody = RoleId::presented("no-such-role".to_owned());
        let nowhere = LoopId::presented("no-such-loop".to_owned());

        assert!(
            transaction
                .set_cell(&nobody, &held_on, Permission::Emit)
                .await
                .expect("the write to answer")
                .is_none()
        );
        assert!(
            transaction
                .set_cell(&role, &nowhere, Permission::Emit)
                .await
                .expect("the write to answer")
                .is_none()
        );
        assert!(
            transaction
                .dismiss_unreviewed(&nowhere)
                .await
                .expect("the write to answer")
                .is_none()
        );
        assert!(
            transaction
                .the_row_of(&nobody)
                .await
                .expect("the read to answer")
                .is_none()
        );
        assert!(
            transaction
                .the_column_of(&nowhere)
                .await
                .expect("the read to answer")
                .is_none()
        );
        assert_eq!(
            transaction
                .held_by(&role, &nowhere)
                .await
                .expect("the lookup to answer"),
            Permission::None,
            "a loop nobody holds conferred reach"
        );
    }
}

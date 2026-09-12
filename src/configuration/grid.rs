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
//!   (v1 §3). [`Grid::held_by`] — the evaluator's lookup — applies that; the reads the
//!   console works from do not, because an administrator ruling on a column has to see what
//!   they have set so far.
//!
//! **The staffing flag is the one other thing a cell holds**, and it is a column on the cell
//! rather than a table beside it: it is set per (role, loop), which is what a cell is. It
//! says one thing — *this role counts toward this loop's staffing state* — and it **confers
//! nothing** ([ADR-0065]): marking a role as staffing a loop subscribes nobody and changes
//! no console. Its one rule is that the pair must hold at least `emit`, because a role that
//! cannot answer cannot staff (v1 §1), and it is held by the schema as well as by
//! [`Grid::set_staffing`] — lowering a cell below `emit` clears the flag with it.
//!
//! **A staffing role on an unreviewed loop staffs nothing**, and that is the same split the
//! two permission reads are: [`Grid::a_cell`] reports the flag as it was set, so an
//! administrator sees what they did, and [`Grid::the_staffing_roles`] — the read staffing
//! state is computed from — leaves it out, because every rung on that loop is enforced as
//! `none` and an occupant of the role could not subscribe if they tried. A loop nobody has
//! ruled on reading `away — 3 not subscribed` for months is the misrepresentation
//! [ADR-0056] refuses, arriving from the other side.
//!
//! The console reads this one row or one column at a time ([ADR-0015]): a role page is the
//! row and a loop page is the column. Both are the same list of cells in a different order,
//! which is why they are one type here and not two.
//!
//! [ADR-0011]: ../../../docs/adr/0011-a-permission-is-one-cell-on-the-grid.md
//! [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md
//! [ADR-0056]: ../../../docs/adr/0056-a-loop-with-no-staffing-roles-has-no-staffing-state.md
//! [ADR-0065]: ../../../docs/adr/0065-the-staffing-flag-reports-it-never-subscribes.md

use async_trait::async_trait;
use sqlx::Row;

use super::loops::{Loop, LoopId, Loops, a_loop};
use super::records::{AdministrationRefused, Change};
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
    /// Whether this role counts toward this loop's staffing state (v1 §1).
    ///
    /// It is on the cell because it is set per (role, loop) and a cell is that pair, and it
    /// is **beside** the permission rather than a fifth rung above `control`, because it is
    /// not authority: it confers nothing at all ([ADR-0065]), and a ladder that ended in it
    /// would make *counts as cover* something an administrator grants by raising somebody's
    /// reach.
    ///
    /// Only ever true where the permission carries `emit`. It is reported as it was set,
    /// unreviewed loop or not — [`Grid::the_staffing_roles`] is the read that decides.
    ///
    /// [ADR-0065]: ../../../docs/adr/0065-the-staffing-flag-reports-it-never-subscribes.md
    pub(crate) staffs: bool,
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
    /// Where it leaves no role unruled on that loop, the loop's `unreviewed` mark is cleared
    /// with it: an administrator who has ruled on every cell of a column has ruled on the
    /// column (v1 §9), and a mark left standing there would enforce `none` on permissions
    /// somebody had deliberately set.
    ///
    /// A pair naming a role or a loop that is not there is no change rather than a refusal,
    /// exactly as a write against any other id nobody holds is.
    async fn set_cell(
        &mut self,
        role: &RoleId,
        held_on: &LoopId,
        permission: Permission,
    ) -> Result<Option<Change<Cell>>, StoreError>;

    /// Mark this role as staffing this loop, or stop marking it.
    ///
    /// **It confers nothing** ([ADR-0065]). Nobody is subscribed by it, no console changes,
    /// and the one thing it does is put this role's occupants into the answer to *is a human
    /// behind this loop*.
    ///
    /// **Marking** is refused where the pair holds less than `emit`: a role that cannot
    /// answer cannot staff (v1 §1). **Unmarking never is** — the rule constrains what may
    /// count as cover, and saying that a role does not is always a thing an administrator
    /// may say. A pair naming a role or a loop that is not there is no change rather than a
    /// refusal, exactly as [`Grid::set_cell`] is.
    ///
    /// [ADR-0065]: ../../../docs/adr/0065-the-staffing-flag-reports-it-never-subscribes.md
    async fn set_staffing(
        &mut self,
        role: &RoleId,
        held_on: &LoopId,
        staffs: bool,
    ) -> Result<Option<Change<Cell>>, AdministrationRefused>;

    /// Every staffing role on the deployment: the loop, and the role that staffs it.
    ///
    /// **This is the read staffing state is computed from**, and it is the whole deployment
    /// in one answer rather than a question asked per loop, because that is how it is
    /// consumed — a presence document works out the state of every loop in reach at once,
    /// and a lobby works out the state of every loop every eligible role staffs. It is
    /// roughly fifteen roles against twenty loops, and only the marked pairs are in it.
    ///
    /// It **decides** rather than reports, so an unreviewed loop contributes nothing: every
    /// rung on it is enforced as `none`, so no occupant of any role could subscribe to it,
    /// and a loop nobody has ruled on would otherwise read `away — not subscribed`
    /// permanently ([ADR-0056]). A loop with no staffing roles is simply absent, which is
    /// how the absence of a staffing state is arrived at rather than configured.
    ///
    /// It answers **by loop** rather than a row per marked pair, because that is the
    /// question: staffing state is a property of the loop, computed across every occupant
    /// of every role that staffs it. A caller handed the pairs would regroup them before it
    /// could ask anything.
    ///
    /// In the base loop order, like every other read that answers with loops ([ADR-0053]).
    ///
    /// [ADR-0053]: ../../../docs/adr/0053-the-loop-order-is-complete-and-a-new-loop-lands-at-the-end.md
    /// [ADR-0056]: ../../../docs/adr/0056-a-loop-with-no-staffing-roles-has-no-staffing-state.md
    async fn the_staffing_roles(&mut self) -> Result<Vec<(Loop, Vec<RoleId>)>, StoreError>;

    /// A role's row: the role, and every loop in the base order with what it holds on each.
    ///
    /// This is a role page ([ADR-0015]), and it answers *what can this role reach*. It is
    /// every loop rather than the ones with cells, because a row read as a list has to show
    /// the loops this role cannot reach for the list to mean anything — and the role comes
    /// back with them, because a deployment with no loops still has a row to render.
    ///
    /// [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md
    async fn the_row_of(&mut self, role: &RoleId) -> Result<Option<(Role, Vec<Cell>)>, StoreError>;

    /// A role's **reach**: every loop it holds at least `rung` on, in the base loop order.
    ///
    /// It is the row read that **decides** rather than reports, so it is [`Grid::held_by`]
    /// over a whole row rather than [`Grid::the_row_of`] filtered — an unreviewed loop is
    /// `none` here, exactly as it is to the evaluator. Reading the reporting row and
    /// filtering it would put a loop nobody has ruled on into a session's presence document
    /// while the evaluator refused every message about it.
    ///
    /// This is what scopes the presence document ([ADR-0019]): a session receives presence
    /// only for loops its role holds at least `monitor` on.
    ///
    /// [ADR-0019]: ../../../docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md
    async fn the_reach_of(
        &mut self,
        role: &RoleId,
        rung: Permission,
    ) -> Result<Vec<(Loop, Permission)>, StoreError>;

    /// A loop's column: the loop, and every role by name with what it holds on it.
    ///
    /// This is a loop page, and it answers *who may hear this loop*.
    async fn the_column_of(
        &mut self,
        held_on: &LoopId,
    ) -> Result<Option<(Loop, Vec<Cell>)>, StoreError>;

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
            // **Lowering a cell below `emit` clears its staffing flag**, in the statement
            // that lowers it. A role that cannot answer cannot staff (v1 §1), so the
            // alternative is a refusal — and refusing to revoke a permission because of a
            // report it feeds would put the grid under the staffing model rather than the
            // other way round. The administrator is told what the write did by the entry it
            // is audited with, which carries the cell either side.
            "INSERT INTO grid_cells (role_id, loop_id, permission, staffs, set_at) \
             VALUES (?, ?, ?, 0, ?) \
             ON CONFLICT (role_id, loop_id) \
             DO UPDATE SET permission = excluded.permission, \
                           staffs = CASE \
                               WHEN excluded.permission IN ('emit', 'control') \
                               THEN grid_cells.staffs ELSE 0 END, \
                           set_at = excluded.set_at",
        )
        .bind(role.as_str())
        .bind(held_on.as_str())
        .bind(permission.as_str())
        .bind(now())
        .execute(self.connection())
        .await
        .map_err(unavailable)?;

        self.rule_on_a_complete_column(held_on).await?;

        Ok(Some(Change {
            before,
            // Read back through the same transaction, so the entry records what the store
            // holds rather than what the caller asked for.
            after: self.a_cell(role, held_on).await?,
        }))
    }

    async fn the_row_of(&mut self, role: &RoleId) -> Result<Option<(Role, Vec<Cell>)>, StoreError> {
        let Some(role) = self.role(role).await? else {
            return Ok(None);
        };

        let rows = sqlx::query(
            "SELECT loops.id AS id, loops.name AS name, loops.is_unreviewed AS is_unreviewed, \
                    COALESCE(grid_cells.permission, 'none') AS permission, \
                    COALESCE(grid_cells.staffs, 0) AS staffs \
             FROM loops \
             LEFT JOIN grid_cells \
               ON grid_cells.loop_id = loops.id AND grid_cells.role_id = ? \
             ORDER BY loops.position, loops.created_at, loops.id",
        )
        .bind(role.id.as_str())
        .fetch_all(self.connection())
        .await
        .map_err(unavailable)?;

        let cells: Vec<Cell> = rows
            .iter()
            .map(|row| {
                Ok(Cell {
                    role: role.clone(),
                    held_on: a_loop(row),
                    permission: a_permission(row, "permission")?,
                    staffs: staffs(row),
                })
            })
            .collect::<Result<_, StoreError>>()?;

        Ok(Some((role, cells)))
    }

    async fn the_reach_of(
        &mut self,
        role: &RoleId,
        rung: Permission,
    ) -> Result<Vec<(Loop, Permission)>, StoreError> {
        let rows = sqlx::query(
            // The same `CASE` `held_by` carries, for the same reason: a loop nobody has
            // ruled on is `none` whatever its cells hold, and it is answered that way here
            // rather than by a caller remembering to ask.
            "SELECT loops.id AS id, loops.name AS name, loops.is_unreviewed AS is_unreviewed, \
                    CASE WHEN loops.is_unreviewed <> 0 THEN 'none' \
                         ELSE COALESCE(grid_cells.permission, 'none') END AS permission \
             FROM loops \
             LEFT JOIN grid_cells \
               ON grid_cells.loop_id = loops.id AND grid_cells.role_id = ? \
             ORDER BY loops.position, loops.created_at, loops.id",
        )
        .bind(role.as_str())
        .fetch_all(self.connection())
        .await
        .map_err(unavailable)?;

        rows.iter()
            .map(|row| Ok((a_loop(row), a_permission(row, "permission")?)))
            .collect::<Result<Vec<_>, StoreError>>()
            .map(|reach| {
                reach
                    .into_iter()
                    .filter(|(_held_on, permission)| permission.carries(rung))
                    .collect()
            })
    }

    async fn the_column_of(
        &mut self,
        held_on: &LoopId,
    ) -> Result<Option<(Loop, Vec<Cell>)>, StoreError> {
        let Some(held_on) = self.a_loop(held_on).await? else {
            return Ok(None);
        };

        let rows = sqlx::query(
            "SELECT roles.id AS id, roles.name AS name, roles.max_occupants AS max_occupants, \
                    COALESCE(grid_cells.permission, 'none') AS permission, \
                    COALESCE(grid_cells.staffs, 0) AS staffs \
             FROM roles \
             LEFT JOIN grid_cells \
               ON grid_cells.role_id = roles.id AND grid_cells.loop_id = ? \
             ORDER BY roles.name",
        )
        .bind(held_on.id.as_str())
        .fetch_all(self.connection())
        .await
        .map_err(unavailable)?;

        let cells: Vec<Cell> = rows
            .iter()
            .map(|row| {
                Ok(Cell {
                    role: a_role(row),
                    held_on: held_on.clone(),
                    permission: a_permission(row, "permission")?,
                    staffs: staffs(row),
                })
            })
            .collect::<Result<_, StoreError>>()?;

        Ok(Some((held_on, cells)))
    }

    /// Composed from the two record reads and the cells that are set, rather than from a
    /// join of its own.
    ///
    /// A join would have to name both records' columns in one row and so could not use the
    /// one function that turns a row into a loop or into a role — and a second place that
    /// builds a record is the one a later column is forgotten in. The cross product is small
    /// by construction: the grid is roughly fifteen roles against twenty loops.
    async fn the_whole_grid(&mut self) -> Result<Vec<Cell>, StoreError> {
        let roles = self.roles().await?;
        let loops = self.loops().await?;
        let set = self.the_cells_that_are_set().await?;

        let mut grid = Vec::with_capacity(roles.len() * loops.len());
        for role in &roles {
            for held_on in &loops {
                grid.push(Cell {
                    role: role.clone(),
                    held_on: held_on.clone(),
                    // An absent cell is `none`, here as everywhere — and staffs nothing,
                    // because the flag is only ever set where the pair may emit.
                    permission: set
                        .get(&(role.id.clone(), held_on.id.clone()))
                        .map_or_else(Permission::default, |(permission, _staffs)| *permission),
                    staffs: set
                        .get(&(role.id.clone(), held_on.id.clone()))
                        .is_some_and(|(_permission, staffs)| *staffs),
                });
            }
        }

        Ok(grid)
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

        // Nothing is left unruled now, so this clears the mark — the same statement a cell
        // write ends with, because the two acts leave the column in the same state.
        self.rule_on_a_complete_column(held_on).await?;

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

        let held = sqlx::query(
            "SELECT permission, staffs FROM grid_cells \
                                WHERE role_id = ? AND loop_id = ?",
        )
        .bind(role.id.as_str())
        .bind(held_on.id.as_str())
        .fetch_optional(self.connection())
        .await
        .map_err(unavailable)?;

        // No row is `none`, and `none` staffs nothing: there is no cell to have marked.
        let (permission, marked) = match &held {
            None => (Permission::None, false),
            Some(row) => (a_permission(row, "permission")?, staffs(row)),
        };

        Ok(Some(Cell {
            role,
            held_on,
            permission,
            staffs: marked,
        }))
    }

    async fn set_staffing(
        &mut self,
        role: &RoleId,
        held_on: &LoopId,
        staffs: bool,
    ) -> Result<Option<Change<Cell>>, AdministrationRefused> {
        let Some(before) = self.a_cell(role, held_on).await? else {
            return Ok(None);
        };
        if staffs && !before.permission.carries(Permission::Emit) {
            return Err(AdministrationRefused::CannotStaff);
        }

        // An `UPDATE` and never an upsert: a pair with no row holds `none`, so marking it
        // has been refused above and unmarking it has nothing to clear — and writing a row
        // here would record a deliberate `none` against a cell nobody has ruled on, which
        // is a decision, and not the one being made.
        sqlx::query(
            "UPDATE grid_cells SET staffs = ?, set_at = ? WHERE role_id = ? AND loop_id = ?",
        )
        .bind(i64::from(staffs))
        .bind(now())
        .bind(role.as_str())
        .bind(held_on.as_str())
        .execute(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(Some(Change {
            before,
            after: self.a_cell(role, held_on).await?,
        }))
    }

    async fn the_staffing_roles(&mut self) -> Result<Vec<(Loop, Vec<RoleId>)>, StoreError> {
        let rows = sqlx::query(
            // `is_unreviewed` is both read and tested: the loop comes back as it stands, and
            // a loop nobody has ruled on is not in the answer at all.
            "SELECT loops.id AS id, loops.name AS name, loops.is_unreviewed AS is_unreviewed, \
                    grid_cells.role_id AS role_id \
             FROM grid_cells \
             JOIN loops ON loops.id = grid_cells.loop_id \
             WHERE grid_cells.staffs <> 0 AND loops.is_unreviewed = 0 \
             ORDER BY loops.position, loops.created_at, loops.id",
            // Ordered by the loop, which is what lets the rows be gathered as they arrive.
        )
        .fetch_all(self.connection())
        .await
        .map_err(unavailable)?;

        // Gathered here rather than by the caller, and in the order the rows arrive, which
        // is the base loop order the query already answers in.
        let mut staffed: Vec<(Loop, Vec<RoleId>)> = Vec::new();
        for row in &rows {
            let held_on = a_loop(row);
            let role = RoleId::known(row.get("role_id"));
            match staffed.last_mut() {
                Some((already, roles)) if already.id == held_on.id => roles.push(role),
                _ => staffed.push((held_on, vec![role])),
            }
        }

        Ok(staffed)
    }
}

impl Transaction {
    /// Clear a loop's `unreviewed` mark, where no role is left unruled on it.
    ///
    /// A loop is unreviewed until an administrator has **set or explicitly dismissed each
    /// role's cell** (v1 §9), so this is the first half of that sentence: an administrator
    /// who ruled on every role has ruled on the column, and the mark is an answered prompt
    /// rather than a standing one.
    ///
    /// It is still cleared **per loop, never per cell** ([ADR-0015]): setting one cell does
    /// nothing to it while any other role is unruled, and what clears it is the state of the
    /// whole column rather than the write that happened to complete it.
    ///
    /// [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md
    async fn rule_on_a_complete_column(&mut self, held_on: &LoopId) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE loops SET is_unreviewed = 0 WHERE id = ? AND NOT EXISTS ( \
                 SELECT 1 FROM roles WHERE NOT EXISTS ( \
                     SELECT 1 FROM grid_cells \
                     WHERE grid_cells.role_id = roles.id AND grid_cells.loop_id = ? \
                 ) \
             )",
        )
        .bind(held_on.as_str())
        .bind(held_on.as_str())
        .execute(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(())
    }

    /// Every cell somebody has ruled on, by the pair that identifies it.
    ///
    /// Only the cells that are *set*: what an absent one means is a rule rather than a row,
    /// and it is applied where the grid is assembled.
    async fn the_cells_that_are_set(
        &mut self,
    ) -> Result<std::collections::HashMap<(RoleId, LoopId), (Permission, bool)>, StoreError> {
        let rows = sqlx::query("SELECT role_id, loop_id, permission, staffs FROM grid_cells")
            .fetch_all(self.connection())
            .await
            .map_err(unavailable)?;

        rows.iter()
            .map(|row| {
                Ok((
                    (
                        RoleId::known(row.get("role_id")),
                        LoopId::known(row.get("loop_id")),
                    ),
                    (a_permission(row, "permission")?, staffs(row)),
                ))
            })
            .collect()
    }
}

/// Whether the cell in this row is marked as staffing its loop.
///
/// The schema holds `0` or `1` and the `CHECK` refuses anything else, so unlike a permission
/// there is no third value to fail to read: anything that is not zero is the flag set.
fn staffs(row: &sqlx::sqlite::SqliteRow) -> bool {
    row.get::<i64, _>("staffs") != 0
}

/// The permission a row holds, from the column it is held in.
fn a_permission(row: &sqlx::sqlite::SqliteRow, held_as: &str) -> Result<Permission, StoreError> {
    let word: String = row.get(held_as);

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

    /// A loop is unreviewed until an administrator has **set or explicitly dismissed each
    /// role's cell** (v1 §9), so ruling on every role rules on the column. It is still per
    /// loop: nothing happens to the mark while any role is left unruled.
    #[tokio::test]
    async fn setting_the_last_unruled_cell_rules_on_the_column() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let flight_director = transaction
            .create_role(NewRole {
                name: "Flight Director".to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the role");
        let observer = transaction
            .roles()
            .await
            .expect("a read")
            .into_iter()
            .find(|role| role.name == "Observer")
            .expect("the seeded role")
            .id;
        let held_on = transaction.create_loop("FLIGHT").await.expect("the loop");

        transaction
            .set_cell(&flight_director, &held_on, Permission::Control)
            .await
            .expect("the cell to be set");

        assert!(
            transaction
                .a_loop(&held_on)
                .await
                .expect("a read")
                .expect("the loop")
                .is_unreviewed,
            "one cell ruled on a column another role is still unruled on"
        );

        transaction
            .set_cell(&observer, &held_on, Permission::Monitor)
            .await
            .expect("the cell to be set");

        assert!(
            !transaction
                .a_loop(&held_on)
                .await
                .expect("a read")
                .expect("the loop")
                .is_unreviewed,
            "a column with every role ruled on was still unreviewed"
        );
        assert_eq!(
            transaction
                .held_by(&flight_director, &held_on)
                .await
                .expect("the lookup to answer"),
            Permission::Control,
            "permissions somebody set were still being enforced as none"
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

        let (_, column) = transaction
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

        let (read, row) = transaction
            .the_row_of(&role)
            .await
            .expect("the row to be read")
            .expect("a row");

        assert_eq!(read.name, "GNC Officer");
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

        let (read, column) = transaction
            .the_column_of(&held_on)
            .await
            .expect("the column to be read")
            .expect("a column");

        assert_eq!(read.name, "FLIGHT");
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

        let (_, column) = transaction
            .the_column_of(&held_on)
            .await
            .expect("the column to be read")
            .expect("a column");
        assert_eq!(column.len(), 1, "a deleted role left its cells behind");
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

    /// A role's reach is the loops it holds at least the named rung on, in the base order —
    /// and it is the **deciding** read, so a rung short of the one asked for is not in it.
    #[tokio::test]
    async fn the_reach_of_a_role_is_the_loops_it_holds_the_rung_on() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, air_to_ground) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "Air-to-ground").await;
        let flight = transaction
            .create_loop("Flight Director")
            .await
            .expect("the loop to be created");
        let surgeon = transaction
            .create_loop("Surgeon")
            .await
            .expect("the loop to be created");
        for held_on in [&flight, &surgeon] {
            transaction
                .dismiss_unreviewed(held_on)
                .await
                .expect("the loop to be ruled on");
        }
        transaction
            .set_cell(&role, &air_to_ground, Permission::Emit)
            .await
            .expect("the cell to be set");
        transaction
            .set_cell(&role, &flight, Permission::Monitor)
            .await
            .expect("the cell to be set");
        transaction
            .set_cell(&role, &surgeon, Permission::None)
            .await
            .expect("the cell to be set");

        let reach = transaction
            .the_reach_of(&role, Permission::Monitor)
            .await
            .expect("the reach to be readable");

        assert_eq!(
            reach
                .iter()
                .map(|(held_on, permission)| (held_on.name.as_str(), *permission))
                .collect::<Vec<_>>(),
            [
                ("Air-to-ground", Permission::Emit),
                ("Flight Director", Permission::Monitor)
            ]
        );

        let may_emit = transaction
            .the_reach_of(&role, Permission::Emit)
            .await
            .expect("the reach to be readable");
        assert_eq!(
            may_emit
                .iter()
                .map(|(held_on, _permission)| held_on.name.as_str())
                .collect::<Vec<_>>(),
            ["Air-to-ground"]
        );

        transaction.roll_back().await.expect("the read to close");
    }

    /// **A loop nobody has ruled on is out of reach**, whatever its cells hold — the same
    /// answer `held_by` gives, because this is the same read over a whole row rather than a
    /// second one that could disagree with it.
    #[tokio::test]
    async fn an_unreviewed_loop_is_out_of_reach_whatever_its_cells_hold() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let role = transaction
            .create_role(NewRole {
                name: "Flight Director".to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the role to be created");
        let unreviewed = transaction
            .create_loop("Air-to-ground")
            .await
            .expect("the loop to be created");
        transaction
            .set_cell(&role, &unreviewed, Permission::Control)
            .await
            .expect("the cell to be set");

        let reach = transaction
            .the_reach_of(&role, Permission::Monitor)
            .await
            .expect("the reach to be readable");

        assert!(reach.is_empty());
        assert_eq!(
            transaction
                .held_by(&role, &unreviewed)
                .await
                .expect("the lookup to answer"),
            Permission::None,
            "the deciding lookup and the reach disagreed about the same cell"
        );

        transaction.roll_back().await.expect("the read to close");
    }

    /// A role nobody has given anything reaches nothing, and a role that is not there
    /// reaches nothing either — an id nobody holds is not a special case.
    #[tokio::test]
    async fn a_role_with_no_cells_reaches_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, _held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "Air-to-ground").await;

        assert!(
            transaction
                .the_reach_of(&role, Permission::Monitor)
                .await
                .expect("the reach to be readable")
                .is_empty()
        );
        assert!(
            transaction
                .the_reach_of(
                    &RoleId::presented("no-such-role".to_owned()),
                    Permission::Monitor
                )
                .await
                .expect("the reach to be readable")
                .is_empty()
        );

        transaction.roll_back().await.expect("the read to close");
    }

    /// The flag is set per (role, loop) and only where the role may answer on that loop
    /// (v1 §1). A role that may hear a loop and not speak on it is not cover.
    #[tokio::test]
    async fn a_role_that_may_not_emit_on_a_loop_cannot_staff_it() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        transaction
            .set_cell(&role, &held_on, Permission::Monitor)
            .await
            .expect("the cell to be set");

        let refused = transaction.set_staffing(&role, &held_on, true).await;

        assert!(matches!(refused, Err(AdministrationRefused::CannotStaff)));
        assert!(
            !transaction
                .a_cell(&role, &held_on)
                .await
                .expect("the cell to be readable")
                .expect("the cell")
                .staffs
        );
    }

    #[tokio::test]
    async fn marks_a_role_as_staffing_a_loop_it_may_emit_on_and_unmarks_it_again() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        transaction
            .set_cell(&role, &held_on, Permission::Emit)
            .await
            .expect("the cell to be set");

        let marked = transaction
            .set_staffing(&role, &held_on, true)
            .await
            .expect("the flag to be set")
            .expect("a change");

        assert!(!marked.before.staffs);
        assert!(marked.after.expect("the cell after").staffs);

        let unmarked = transaction
            .set_staffing(&role, &held_on, false)
            .await
            .expect("the flag to be cleared")
            .expect("a change");

        assert!(unmarked.before.staffs);
        assert!(!unmarked.after.expect("the cell after").staffs);
    }

    /// **The flag confers nothing** ([ADR-0065]): the permission it is set beside is
    /// untouched by it, in either direction.
    ///
    /// [ADR-0065]: ../../../docs/adr/0065-the-staffing-flag-reports-it-never-subscribes.md
    #[tokio::test]
    async fn marking_a_staffing_role_changes_no_permission() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        transaction
            .set_cell(&role, &held_on, Permission::Emit)
            .await
            .expect("the cell to be set");

        transaction
            .set_staffing(&role, &held_on, true)
            .await
            .expect("the flag to be set");

        assert_eq!(
            transaction
                .held_by(&role, &held_on)
                .await
                .expect("the lookup to answer"),
            Permission::Emit
        );
    }

    /// Lowering a cell below `emit` takes its staffing flag with it, in the write that
    /// lowers it: a role that cannot answer cannot staff, and the grid is not held under the
    /// staffing model.
    #[tokio::test]
    async fn lowering_a_cell_below_emit_clears_its_staffing_flag() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        transaction
            .set_cell(&role, &held_on, Permission::Control)
            .await
            .expect("the cell to be set");
        transaction
            .set_staffing(&role, &held_on, true)
            .await
            .expect("the flag to be set");

        let lowered = transaction
            .set_cell(&role, &held_on, Permission::Monitor)
            .await
            .expect("the cell to be set")
            .expect("a change");

        assert!(lowered.before.staffs);
        assert!(!lowered.after.expect("the cell after").staffs);
        assert!(
            transaction
                .the_staffing_roles()
                .await
                .expect("the staffing roles to be readable")
                .is_empty()
        );
    }

    /// Raising it back is not the flag coming back with it. A permission restored restores a
    /// permission; a role staffs a loop because somebody said so.
    #[tokio::test]
    async fn raising_a_cell_back_to_emit_does_not_bring_the_flag_back() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        transaction
            .set_cell(&role, &held_on, Permission::Emit)
            .await
            .expect("the cell to be set");
        transaction
            .set_staffing(&role, &held_on, true)
            .await
            .expect("the flag to be set");
        transaction
            .set_cell(&role, &held_on, Permission::None)
            .await
            .expect("the cell to be taken away");

        let back = transaction
            .set_cell(&role, &held_on, Permission::Emit)
            .await
            .expect("the cell to be set")
            .expect("a change");

        assert!(!back.after.expect("the cell after").staffs);
    }

    /// A loop with no staffing roles is absent from the read staffing state is computed
    /// from, which is how the absence of a state is arrived at rather than configured
    /// ([ADR-0056]).
    ///
    /// [ADR-0056]: ../../../docs/adr/0056-a-loop-with-no-staffing-roles-has-no-staffing-state.md
    #[tokio::test]
    async fn the_staffing_roles_name_the_loop_and_the_role_and_leave_out_the_rest() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        let unstaffed = transaction
            .create_loop("Air-to-ground")
            .await
            .expect("a loop");
        transaction
            .dismiss_unreviewed(&unstaffed)
            .await
            .expect("the second loop to be ruled on");
        transaction
            .set_cell(&role, &held_on, Permission::Emit)
            .await
            .expect("the cell to be set");
        transaction
            .set_cell(&role, &unstaffed, Permission::Emit)
            .await
            .expect("the second cell to be set");
        transaction
            .set_staffing(&role, &held_on, true)
            .await
            .expect("the flag to be set");

        let staffing = transaction
            .the_staffing_roles()
            .await
            .expect("the staffing roles to be readable");

        assert_eq!(staffing.len(), 1);
        assert_eq!(staffing[0].0.id, held_on);
        assert_eq!(staffing[0].0.name, "FLIGHT");
        assert_eq!(staffing[0].1, vec![role]);
    }

    /// An unreviewed loop is `none` on every rung, so nobody could answer on it and nothing
    /// staffs it — the flag is reported as it was set and left out of what decides.
    #[tokio::test]
    async fn an_unreviewed_loop_has_no_staffing_roles_whatever_its_cells_are_marked() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        transaction
            .set_cell(&role, &held_on, Permission::Emit)
            .await
            .expect("the cell to be set");
        transaction
            .set_staffing(&role, &held_on, true)
            .await
            .expect("the flag to be set");
        let unreviewed = transaction
            .create_loop("Air-to-ground")
            .await
            .expect("a loop");
        transaction
            .set_cell(&role, &unreviewed, Permission::Emit)
            .await
            .expect("the cell to be set");
        transaction
            .set_staffing(&role, &unreviewed, true)
            .await
            .expect("the flag to be set");

        let staffing = transaction
            .the_staffing_roles()
            .await
            .expect("the staffing roles to be readable");

        assert_eq!(staffing.len(), 1);
        assert_eq!(staffing[0].0.id, held_on);
        assert!(
            transaction
                .a_cell(&role, &unreviewed)
                .await
                .expect("the cell to be readable")
                .expect("the cell")
                .staffs,
            "the cell reports the flag as it was set, whatever the review mark does to it"
        );
    }

    /// A pair naming a record that is not there is no change rather than a refusal, exactly
    /// as setting a cell is.
    #[tokio::test]
    async fn staffing_a_pair_nobody_holds_is_no_change() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, _held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;

        assert!(
            transaction
                .set_staffing(&role, &LoopId::presented("no-such-loop".to_owned()), true)
                .await
                .expect("the write to answer")
                .is_none()
        );
    }

    /// **Unmarking is never refused.** The rule says what may count as cover; saying that a
    /// role does not is always a thing an administrator may say, and a flag that could only
    /// be cleared while the permission it stood on was still there would be unclearable
    /// exactly where somebody wanted it gone.
    #[tokio::test]
    async fn a_role_that_may_not_emit_can_still_be_unmarked() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (role, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        transaction
            .set_cell(&role, &held_on, Permission::Monitor)
            .await
            .expect("the cell to be set");

        let unmarked = transaction
            .set_staffing(&role, &held_on, false)
            .await
            .expect("the flag to be clearable")
            .expect("a change");

        assert!(!unmarked.after.expect("the cell after").staffs);
    }

    /// The roles that staff one loop come back **together**, because staffing state is a
    /// property of the loop: a caller handed a row per pair would have to gather them
    /// before it could ask anything.
    #[tokio::test]
    async fn the_staffing_roles_of_one_loop_come_back_together() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let (flight_director, held_on) =
            a_role_and_a_reviewed_loop(&mut transaction, "Flight Director", "FLIGHT").await;
        let capcom = transaction
            .create_role(NewRole {
                name: "CAPCOM".to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the second role");
        for role in [&flight_director, &capcom] {
            transaction
                .set_cell(role, &held_on, Permission::Emit)
                .await
                .expect("the cell to be set");
            transaction
                .set_staffing(role, &held_on, true)
                .await
                .expect("the flag to be set");
        }

        let staffing = transaction
            .the_staffing_roles()
            .await
            .expect("the staffing roles to be readable");

        assert_eq!(staffing.len(), 1, "one loop came back as two");
        assert_eq!(staffing[0].1.len(), 2);
        assert!(staffing[0].1.contains(&flight_director));
        assert!(staffing[0].1.contains(&capcom));
    }
}

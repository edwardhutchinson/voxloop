//! What every configuration record has in common: how a write to one is answered, and what
//! can stop one.
//!
//! Users, roles and loops are three records administered the same way — created, read,
//! edited, deleted, and audited with before and after (v1 §12). The two types here are that
//! sameness, so the audited write path is one path rather than three that drift.

use super::store::StoreError;

/// A configuration record before and after a write to it.
///
/// Every configuration change is audited with **before and after** (v1 §12), so a write
/// answers with both rather than leaving the caller to read around it — which would be two
/// more reads and a window in which the answer is assembled from three different moments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change<T> {
    pub(crate) before: T,
    /// Absent on a deletion, which is the whole of what a deletion says.
    pub(crate) after: Option<T>,
}

impl<T> Change<T> {
    /// Two writes read as the one change they were asked for.
    ///
    /// An edit that renames a role and widens its occupancy is one act by one administrator,
    /// and the log records where the record started and where it ended rather than the step
    /// between.
    pub(crate) fn then(self, next: Self) -> Self {
        Self {
            before: self.before,
            after: next.after,
        }
    }
}

/// What can stop an administration write.
///
/// The refusals and the fault are different in kind, and the type says so rather than
/// leaving it to whoever writes the next `match`: the first four are refusals a human acts
/// on — by choosing another name, by promoting somebody before demoting themselves, by
/// admitting at least one occupant, or by ordering the loops that are actually there — and
/// the last is a fault. Folding a refusal into [`StoreError`] would let a caller who forgot
/// the arm answer "that name is taken" with "VoxLoop could not answer that just now".
#[derive(Debug, thiserror::Error)]
pub(crate) enum AdministrationRefused {
    /// `what` names the field a human typed into — a username, a role name, a loop name —
    /// because *the name is taken* is only actionable if it says which one.
    #[error("the {what} {name:?} is already taken")]
    NameTaken { what: &'static str, name: String },

    /// The last system administrator cannot be removed (v1 §2). Clearing the flag on,
    /// locking or deleting the final one is refused, because each of the three leaves a
    /// deployment nobody can administer and only shell access to the box can recover it.
    ///
    /// *Final* counts flag holders and nothing else. Narrowing it to the ones who could
    /// sign in today reads as an improvement and is a hole: an administrator who stops
    /// counting is one the next call may delete, and a box can be emptied of them one act
    /// at a time.
    #[error("that is the last system administrator this deployment can be administered by")]
    LastSystemAdministrator,

    /// A role is a staffable position, so one nobody may occupy is not a role at all (v1
    /// §1). It is refused rather than stored, because a role that exists and turns everybody
    /// away is indistinguishable from a permission problem to whoever hits it.
    #[error("a role must admit at least one occupant")]
    NobodyMayOccupy,

    /// The base loop order is a **complete** ordering of the deployment's loops (ADR-0053),
    /// so an order naming anything other than exactly the loops that exist is refused rather
    /// than half-applied. It is also how a console that was arranging an order while
    /// somebody else created a loop is told to read again instead of quietly dropping it.
    #[error("that order does not name every loop exactly once")]
    IncompleteOrder,

    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Tell a name that is already taken apart from a store that could not answer.
///
/// Every record with a name a human types is unique on it, case-insensitively, so this is
/// one function rather than three: the unique violation is the store saying the name is
/// taken, and anything else is the store not answering.
pub(super) fn taken_or_unavailable(
    error: sqlx::Error,
    what: &'static str,
    name: &str,
) -> AdministrationRefused {
    let taken = error
        .as_database_error()
        .is_some_and(sqlx::error::DatabaseError::is_unique_violation);

    if taken {
        return AdministrationRefused::NameTaken {
            what,
            name: name.to_owned(),
        };
    }

    AdministrationRefused::Store(super::store::unavailable(error))
}

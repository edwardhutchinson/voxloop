//! Configuration — users, roles, loops, the (role, loop) grid, eligibility, service
//! principals, the pronunciation dictionary, personalisation and the audit log, plus the
//! file-held configuration needed to reach any of it.
//!
//! Persistence is not a module of its own ([ADR-0038]); it is this module's seam, and
//! nothing else in the system reaches a database. Audit is not a module either — an entry
//! and the write it records commit in one transaction, so the entry is written by whatever
//! owns that write.
//!
//! Two rules hold everything here, and both are cheaper to keep than to restore:
//!
//! - **The seam is domain-shaped.** Repositories name domain operations over domain types —
//!   `set_cell`, `grant_eligibility`, `record_authority_act` — never `query` and `execute`.
//!   No `sqlx` type crosses out of this module, and `sqlx::Any` is ruled out by name.
//! - **The transaction is part of the seam.** A [`Transaction`] is opened by the caller and
//!   passed into repository methods rather than hidden behind them, because a grid edit and
//!   its audit entry have to land together or not at all.
//!
//! There is no in-memory repository fake, now or later ([ADR-0064]). Tests run against the
//! real store: a temporary file, migrated and thrown away.
//!
//! [ADR-0038]: ../../../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md
//! [ADR-0064]: ../../../docs/adr/0064-tests-run-against-the-real-store.md

mod audit;
mod deployment;
mod sign_ins;
mod store;
mod users;

pub(crate) use audit::{AuditEntry, AuditEvent, AuditLog};
pub(crate) use deployment::{Deployment, DeploymentError};
pub(crate) use sign_ins::{SignInToken, SignIns};
#[cfg(test)]
pub(crate) use store::a_temporary_store;
pub(crate) use store::{Store, StoreError, Transaction};
pub(crate) use users::{NameRefused, NewUser, PasswordHash, UserId, Users};

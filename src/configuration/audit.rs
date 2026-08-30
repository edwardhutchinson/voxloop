//! The audit log: decisions about the system, never the traffic through it ([ADR-0028]).
//!
//! Audit is not a module of its own. An entry and the write it records commit in one
//! transaction, so the entry is written by whoever owns that write — a separate module would
//! be one somebody could forget to call ([ADR-0060]).
//!
//! The log **outlives the records it references**, so an entry holds the internal user id
//! *and* the name as it stood. The id keeps the entry correct across a rename; the snapshot
//! keeps it readable after the user is deleted. That is also why `actor_id` is not a foreign
//! key: a deleted user's entries must stay, attributed.
//!
//! Append-only is an application discipline rather than a database guarantee ([ADR-0038]).
//! Nothing here updates or deletes an entry, and the schema's triggers refuse it, so the
//! promise is testable rather than merely intended.
//!
//! [ADR-0028]: ../../../docs/adr/0028-the-audit-log-records-decisions-not-traffic.md
//! [ADR-0038]: ../../../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md
//! [ADR-0060]: ../../../docs/adr/0060-a-seam-names-domain-operations.md

use std::net::IpAddr;

use async_trait::async_trait;
use sqlx::Row;

use super::store::{StoreError, Transaction, now, unavailable};
use super::users::UserId;

/// A decision worth recording.
///
/// The classes are fixed by [ADR-0028] and grow one ticket at a time: these four are the
/// authentication events #30 can produce. Configuration changes and authority acts join them
/// with the writes that make them.
///
/// [ADR-0028]: ../../../docs/adr/0028-the-audit-log-records-decisions-not-traffic.md
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuditEvent {
    SignInSucceeded,
    SignInFailed,
    SignedOut,
    /// The first system administrator, made by whoever could read the server's own log.
    BootstrapRedeemed,
    /// A bootstrap code presented and not accepted. Refused administration writes are
    /// audited ([ADR-0054]), and this is the one write that makes an administrator.
    ///
    /// [ADR-0054]: ../../../docs/adr/0054-every-operation-declares-its-authorisation.md
    BootstrapRefused,
}

impl AuditEvent {
    /// The name the log holds. These strings are on disk in customer deployments, so they
    /// are renamed only by a migration.
    fn stored(self) -> &'static str {
        match self {
            Self::SignInSucceeded => "sign_in_succeeded",
            Self::SignInFailed => "sign_in_failed",
            Self::SignedOut => "signed_out",
            Self::BootstrapRedeemed => "bootstrap_redeemed",
            Self::BootstrapRefused => "bootstrap_refused",
        }
    }

    #[allow(dead_code)] // Half of a pair: the other half is on every write.
    fn from_stored(stored: &str) -> Option<Self> {
        match stored {
            "sign_in_succeeded" => Some(Self::SignInSucceeded),
            "sign_in_failed" => Some(Self::SignInFailed),
            "signed_out" => Some(Self::SignedOut),
            "bootstrap_redeemed" => Some(Self::BootstrapRedeemed),
            "bootstrap_refused" => Some(Self::BootstrapRefused),
            _ => None,
        }
    }
}

/// A row the log holds that this binary cannot read back.
///
/// Only a newer VoxLoop or a hand-edited file can produce one, and the binary refuses to
/// start against a newer schema — so this is a fault to report rather than a value to guess
/// at. An audit read that quietly dropped what it could not parse would be the worst of the
/// available behaviours.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)] // Constructed on the read path, which #31's console is the first to use.
enum Unreadable {
    #[error("the audit log holds an event this binary does not know: {0:?}")]
    Event(String),

    #[error("the audit log holds a source that is not an address: {0:?}")]
    Source(String),
}

/// A decision about to be recorded.
pub(crate) struct AuditEntry {
    pub(crate) event: AuditEvent,
    /// Absent where the attempt named nobody the store recognises — a sign-in failure
    /// against a username that does not exist has no actor to attribute it to.
    pub(crate) actor: Option<UserId>,
    /// The name as it stood, which is what keeps the entry readable once the record it
    /// refers to is gone. For a failed sign-in it is the name that was submitted.
    pub(crate) actor_name: String,
    /// Where the attempt came from. A failed sign-in with no source cannot show a
    /// brute-force attempt, which is the compensating control for rate-limiting rather than
    /// auto-locking ([ADR-0025]).
    ///
    /// [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
    pub(crate) source: Option<IpAddr>,
}

/// A decision as the log holds it.
#[allow(dead_code)] // Read by the tests that hold the log to its promises, and by #31.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RecordedEntry {
    pub(crate) event: AuditEvent,
    pub(crate) actor: Option<UserId>,
    pub(crate) actor_name: String,
    pub(crate) source: Option<IpAddr>,
    /// Milliseconds since the Unix epoch.
    pub(crate) recorded_at: i64,
}

/// The audit log, as domain operations rather than queries.
#[async_trait]
pub(crate) trait AuditLog {
    /// Record one decision, in the same transaction as the write it records.
    async fn record(&mut self, entry: AuditEntry) -> Result<(), StoreError>;

    /// The most recent entries, newest first.
    ///
    /// The console's filtered query — by actor and by target — is system administration and
    /// arrives with the console (#31). Until it does, this is what the log is written
    /// against: an append-only log nothing can read is a promise rather than a record.
    #[allow(dead_code)]
    async fn recent_entries(&mut self, at_most: u32) -> Result<Vec<RecordedEntry>, StoreError>;
}

#[async_trait]
impl AuditLog for Transaction {
    async fn record(&mut self, entry: AuditEntry) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO audit_entries (recorded_at, event, actor_id, actor_name, source) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(now())
        .bind(entry.event.stored())
        .bind(entry.actor.as_ref().map(UserId::as_str))
        .bind(&entry.actor_name)
        .bind(entry.source.map(|source| source.to_string()))
        .execute(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(())
    }

    async fn recent_entries(&mut self, at_most: u32) -> Result<Vec<RecordedEntry>, StoreError> {
        let rows = sqlx::query(
            "SELECT recorded_at, event, actor_id, actor_name, source FROM audit_entries \
             ORDER BY id DESC LIMIT ?",
        )
        .bind(at_most)
        .fetch_all(self.connection())
        .await
        .map_err(unavailable)?;

        rows.into_iter()
            .map(|row| {
                let stored: String = row.get("event");
                let event = AuditEvent::from_stored(&stored)
                    .ok_or_else(|| unavailable(Unreadable::Event(stored)))?;

                let source = match row.get::<Option<String>, _>("source") {
                    None => None,
                    Some(stored) => Some(
                        stored
                            .parse()
                            .map_err(|_| unavailable(Unreadable::Source(stored)))?,
                    ),
                };

                Ok(RecordedEntry {
                    event,
                    actor: row.get::<Option<String>, _>("actor_id").map(UserId::known),
                    actor_name: row.get("actor_name"),
                    source,
                    recorded_at: row.get("recorded_at"),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::store::a_temporary_store;
    use crate::configuration::users::{NewUser, Users};

    fn a_sign_in_by(actor: Option<&UserId>, name: &str) -> AuditEntry {
        AuditEntry {
            event: AuditEvent::SignInSucceeded,
            actor: actor.cloned(),
            actor_name: name.to_owned(),
            source: Some(IpAddr::from([192, 0, 2, 7])),
        }
    }

    async fn a_user(transaction: &mut Transaction, username: &str) -> UserId {
        transaction
            .create_user(NewUser {
                username: username.to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("the user to be created")
    }

    #[tokio::test]
    async fn records_a_decision_and_reads_it_back() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;

        transaction
            .record(a_sign_in_by(Some(&user), "flight"))
            .await
            .expect("the entry to be recorded");

        let entries = transaction
            .recent_entries(10)
            .await
            .expect("the log to be readable");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event, AuditEvent::SignInSucceeded);
        assert_eq!(entries[0].actor.as_ref(), Some(&user));
        assert_eq!(entries[0].actor_name, "flight");
        assert_eq!(entries[0].source, Some(IpAddr::from([192, 0, 2, 7])));
        assert!(entries[0].recorded_at > 0);
    }

    #[tokio::test]
    async fn reads_the_newest_first() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        for name in ["first", "second", "third"] {
            transaction
                .record(a_sign_in_by(None, name))
                .await
                .expect("the entry to be recorded");
        }

        let entries = transaction
            .recent_entries(2)
            .await
            .expect("the log to be readable");
        let names: Vec<&str> = entries
            .iter()
            .map(|entry| entry.actor_name.as_str())
            .collect();
        assert_eq!(names, ["third", "second"]);
    }

    #[tokio::test]
    async fn a_failed_sign_in_records_the_name_that_was_submitted_and_no_actor() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        transaction
            .record(AuditEntry {
                event: AuditEvent::SignInFailed,
                actor: None,
                actor_name: "nobody-by-that-name".to_owned(),
                source: Some(IpAddr::from([192, 0, 2, 7])),
            })
            .await
            .expect("the entry to be recorded");

        let entries = transaction
            .recent_entries(1)
            .await
            .expect("the log to be readable");
        assert_eq!(entries[0].actor, None);
        assert_eq!(entries[0].actor_name, "nobody-by-that-name");
    }

    /// The log outlives the records it references ([ADR-0028]).
    ///
    /// [ADR-0028]: ../../../docs/adr/0028-the-audit-log-records-decisions-not-traffic.md
    #[tokio::test]
    async fn deleting_a_user_leaves_their_entries_readable_and_attributed() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;
        transaction
            .record(a_sign_in_by(Some(&user), "flight"))
            .await
            .expect("the entry to be recorded");

        sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(user.as_str())
            .execute(transaction.connection())
            .await
            .expect("the user to be deleted");

        let entries = transaction
            .recent_entries(10)
            .await
            .expect("the log to be readable");
        assert_eq!(entries.len(), 1, "the entry went with the user");
        assert_eq!(entries[0].actor.as_ref(), Some(&user));
        assert_eq!(entries[0].actor_name, "flight");
    }

    #[tokio::test]
    async fn renaming_a_user_leaves_the_name_the_entry_recorded_alone() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let user = a_user(&mut transaction, "flight").await;
        transaction
            .record(a_sign_in_by(Some(&user), "flight"))
            .await
            .expect("the entry to be recorded");

        transaction
            .rename_user(&user, "flight-director")
            .await
            .expect("the rename to land");

        let entries = transaction
            .recent_entries(10)
            .await
            .expect("the log to be readable");
        assert_eq!(entries[0].actor_name, "flight");
        assert_eq!(entries[0].actor.as_ref(), Some(&user));
    }

    /// ADR-0038 requires the append-only property to be *tested*, not merely intended.
    #[tokio::test]
    async fn an_entry_can_be_neither_amended_nor_removed() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        transaction
            .record(a_sign_in_by(None, "flight"))
            .await
            .expect("the entry to be recorded");

        let amended = sqlx::query("UPDATE audit_entries SET actor_name = 'somebody else'")
            .execute(transaction.connection())
            .await;
        assert!(amended.is_err(), "an audit entry was amended");

        let removed = sqlx::query("DELETE FROM audit_entries")
            .execute(transaction.connection())
            .await;
        assert!(removed.is_err(), "an audit entry was removed");

        assert_eq!(
            transaction
                .recent_entries(10)
                .await
                .expect("the log to be readable")
                .len(),
            1
        );
    }
}

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

use super::loops::Loop;
use super::records::Change;
use super::roles::Role;
use super::store::{StoreError, Transaction, now, unavailable};
use super::users::{User, UserId};

/// A decision worth recording.
///
/// The classes are fixed by [ADR-0028] and grow one ticket at a time: five authentication
/// events, the user administration writes from #31, and the role, loop and base-order writes
/// from #33. The grid joins them with the writes that make it.
///
/// An event says what was attempted, not whether it succeeded: a refused write is the same
/// event carrying [`ConfigurationWrite::refusal`], so a log filtered to *deletions of this
/// user* shows the one that was refused alongside the one that was not.
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
    UserCreated,
    /// A rename, or the system-administration flag given or taken away.
    UserEdited,
    UserDeleted,
    RoleCreated,
    /// A rename, or a change to how many may occupy the role at once.
    RoleEdited,
    RoleDeleted,
    LoopCreated,
    LoopEdited,
    LoopDeleted,
    /// The deployment-wide base loop order, set. It is the one configuration write that is
    /// about no single record: the order is a fact about all of them at once ([ADR-0053]).
    ///
    /// [ADR-0053]: ../../../docs/adr/0053-the-loop-order-is-complete-and-a-new-loop-lands-at-the-end.md
    LoopOrderEdited,
    AccountLocked,
    AccountUnlocked,
    /// The password taken away, ending the sign-in and the session immediately (v1 §2).
    PasswordResetForced,
    /// An enrolment code issued against a user. It is a credential, so issuing one is an
    /// administration write in its own right rather than a step in some other act
    /// ([ADR-0025]).
    ///
    /// [ADR-0025]: ../../../docs/adr/0025-credentials-are-administered-because-there-is-no-email.md
    EnrolmentCodeIssued,
    /// A code spent, and the password it set. The actor is the user the code named: they
    /// proved possession of it, which is the whole of what a redemption establishes.
    EnrolmentRedeemed,
    /// A code presented and not accepted — never issued, already spent, or expired. It names
    /// nobody, because a code that enrols nobody says nothing about who presented it, and
    /// where it came from is the whole of what is known.
    EnrolmentRefused,
    /// A signed-in user changing their own password by re-presenting the current one. It
    /// does not end their session (v1 §2).
    PasswordChanged,
    /// A change of one's own password where the current one presented was not it. It is the
    /// same brute-force signal a failed sign-in is, arriving on a route that already has a
    /// user attached to it.
    PasswordChangeRefused,
    /// An administration write somebody was turned away from before it reached the record
    /// it was about. Refused administration writes are audited; refused reads are not
    /// (v1 §3), and an unauthorised attempt to make an administrator is the case this is
    /// here for.
    AdministrationRefused,
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
            Self::UserCreated => "user_created",
            Self::UserEdited => "user_edited",
            Self::UserDeleted => "user_deleted",
            Self::RoleCreated => "role_created",
            Self::RoleEdited => "role_edited",
            Self::RoleDeleted => "role_deleted",
            Self::LoopCreated => "loop_created",
            Self::LoopEdited => "loop_edited",
            Self::LoopDeleted => "loop_deleted",
            Self::LoopOrderEdited => "loop_order_edited",
            Self::AccountLocked => "account_locked",
            Self::AccountUnlocked => "account_unlocked",
            Self::PasswordResetForced => "password_reset_forced",
            Self::EnrolmentCodeIssued => "enrolment_code_issued",
            Self::EnrolmentRedeemed => "enrolment_redeemed",
            Self::EnrolmentRefused => "enrolment_refused",
            Self::PasswordChanged => "password_changed",
            Self::PasswordChangeRefused => "password_change_refused",
            Self::AdministrationRefused => "administration_refused",
        }
    }

    fn from_stored(stored: &str) -> Option<Self> {
        match stored {
            "sign_in_succeeded" => Some(Self::SignInSucceeded),
            "sign_in_failed" => Some(Self::SignInFailed),
            "signed_out" => Some(Self::SignedOut),
            "bootstrap_redeemed" => Some(Self::BootstrapRedeemed),
            "bootstrap_refused" => Some(Self::BootstrapRefused),
            "user_created" => Some(Self::UserCreated),
            "user_edited" => Some(Self::UserEdited),
            "user_deleted" => Some(Self::UserDeleted),
            "role_created" => Some(Self::RoleCreated),
            "role_edited" => Some(Self::RoleEdited),
            "role_deleted" => Some(Self::RoleDeleted),
            "loop_created" => Some(Self::LoopCreated),
            "loop_edited" => Some(Self::LoopEdited),
            "loop_deleted" => Some(Self::LoopDeleted),
            "loop_order_edited" => Some(Self::LoopOrderEdited),
            "account_locked" => Some(Self::AccountLocked),
            "account_unlocked" => Some(Self::AccountUnlocked),
            "password_reset_forced" => Some(Self::PasswordResetForced),
            "enrolment_code_issued" => Some(Self::EnrolmentCodeIssued),
            "enrolment_redeemed" => Some(Self::EnrolmentRedeemed),
            "enrolment_refused" => Some(Self::EnrolmentRefused),
            "password_changed" => Some(Self::PasswordChanged),
            "password_change_refused" => Some(Self::PasswordChangeRefused),
            "administration_refused" => Some(Self::AdministrationRefused),
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
    /// What the write did to configuration, where the decision was one.
    ///
    /// Absent on an authentication event, which changes no record — and the type is what
    /// makes that the only way to record one without a blast radius.
    pub(crate) write: Option<ConfigurationWrite>,
    /// The operation somebody was turned away from, where the entry is about an attempt
    /// rather than about a record.
    ///
    /// A write refused before it reached the record it was about touched nothing, so it has
    /// no before, no after and no radius to record — and which operation it was is the whole
    /// of what makes the entry worth keeping.
    pub(crate) operation: Option<String>,
}

/// What a configuration change did: to which record, from what, to what, and to anything
/// live at the time.
///
/// The four travel together because v1 §12 asks for all four of every configuration change,
/// and a struct that can be built without one is a struct somebody builds without one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigurationWrite {
    /// The record the write was about. Absent where there is none to name — a creation
    /// refused because the name was taken never got an id, and the base loop order is about
    /// every loop rather than any one of them.
    pub(crate) target: Option<RecordId>,
    /// The target's name as it stood, which is what keeps the entry readable once the
    /// record it refers to is gone.
    pub(crate) target_name: String,
    /// The record before the write. Absent on a creation, which had nothing before it.
    pub(crate) before: Option<Snapshot>,
    /// The record after it. Absent on a deletion, and on any write that was refused.
    pub(crate) after: Option<Snapshot>,
    pub(crate) blast_radius: BlastRadius,
    /// Why the write did not happen, where it did not. Refused administration writes are
    /// audited; refused reads are not (v1 §3).
    pub(crate) refusal: Option<String>,
}

impl ConfigurationWrite {
    /// What a write to a record did, as the log holds it.
    ///
    /// It is here rather than with any caller because it is built from a [`Change`], and
    /// [`Change`] is Configuration's. It is one function over users, roles and loops alike:
    /// the admin console and the on-box CLI make the same writes by different entitlements,
    /// and an entry that differed between them — or between two kinds of record — would say
    /// the write differed.
    pub(crate) fn about<T: Record>(change: &Change<T>, blast_radius: BlastRadius) -> Self {
        let ended_as = change.after.as_ref().unwrap_or(&change.before);

        Self {
            target: Some(change.before.recorded_id()),
            // The name as it ended, which is what a rename's entry has to be read by.
            target_name: ended_as.recorded_name().to_owned(),
            before: Some(change.before.snapshot()),
            after: change.after.as_ref().map(Record::snapshot),
            blast_radius,
            refusal: None,
        }
    }
}

/// The internal id of whatever record an entry is about, whichever kind it is.
///
/// Which kind that was is the event's job to say — `role_deleted` names a role — so this is
/// the id and nothing else. It is opaque here exactly as it is everywhere else: the log
/// holds it so that an entry stays correct across a rename, and holds the name beside it so
/// that the entry stays readable after the record is gone ([ADR-0028]).
///
/// [ADR-0028]: ../../../docs/adr/0028-the-audit-log-records-decisions-not-traffic.md
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecordId(String);

impl RecordId {
    /// The record an opaque internal id names.
    pub(super) fn of(id: &str) -> Self {
        Self(id.to_owned())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A configuration record the log can be about: a user, a role or a loop.
///
/// Three writes, one audited path. What each record renders into a [`Snapshot`] is the only
/// thing that differs between them, and it is here rather than with the record so that the
/// strings customer deployments hold on disk are changed in one place, deliberately.
pub(crate) trait Record {
    fn recorded_id(&self) -> RecordId;

    /// The name as it stands, which keeps the entry readable once the record is gone.
    fn recorded_name(&self) -> &str;

    fn snapshot(&self) -> Snapshot;
}

impl Record for User {
    fn recorded_id(&self) -> RecordId {
        RecordId::of(self.id.as_str())
    }

    fn recorded_name(&self) -> &str {
        &self.username
    }

    /// A user as they stood.
    ///
    /// Whether a password is set is in here and what it is never could be: without it, a
    /// forced password reset — a write whose entire effect is on the credential — would
    /// record two identical lines and say nothing.
    fn snapshot(&self) -> Snapshot {
        Snapshot(format!(
            "username={} system_administration={} locked={} password={}",
            self.username,
            yes_or_no(self.is_system_administrator),
            yes_or_no(self.is_locked),
            if self.has_password { "set" } else { "none" },
        ))
    }
}

impl Record for Role {
    fn recorded_id(&self) -> RecordId {
        RecordId::of(self.id.as_str())
    }

    fn recorded_name(&self) -> &str {
        &self.name
    }

    /// A role as it stood, with the limit rendered as the absence it is where there is none.
    fn snapshot(&self) -> Snapshot {
        Snapshot(format!(
            "role={} max_occupants={}",
            self.name,
            self.max_occupants
                .map_or_else(|| "no limit".to_owned(), |limit| limit.to_string()),
        ))
    }
}

impl Record for Loop {
    fn recorded_id(&self) -> RecordId {
        RecordId::of(self.id.as_str())
    }

    fn recorded_name(&self) -> &str {
        &self.name
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot(format!(
            "loop={} reviewed={}",
            self.name,
            yes_or_no(!self.is_unreviewed),
        ))
    }
}

/// A configuration record as it stood, in the form the log holds it.
///
/// One line of `field=value`, so that a before and an after can be read side by side by
/// somebody who was not there. These strings are on disk in customer deployments, so their
/// shape is changed only deliberately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot(String);

impl Snapshot {
    /// The base loop order as it stood, which is the one snapshot about no single record.
    ///
    /// It holds the names rather than the ids, because an order is read by somebody
    /// answering *did they mean to put `THERMAL` first* and a line of opaque ids answers
    /// nothing. The write it belongs to names no target, so nothing is joined on these.
    pub(crate) fn of_the_loop_order(order: &[Loop]) -> Self {
        Self(format!(
            "loop_order={}",
            order
                .iter()
                .map(|held| held.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    /// An enrolment code as it stands, which is never the code itself.
    ///
    /// A credential readable out of the audit log would be a credential anybody who may read
    /// the log holds, so what is recorded is when it stops being good. That is enough for
    /// the entry to say what issuing did: this code replaced that one, and it dies then.
    pub(crate) fn of_enrolment(expires_at: i64) -> Self {
        Self(format!(
            "enrolment_code=outstanding expires_at={expires_at}"
        ))
    }

    /// Take back a snapshot the log already holds.
    fn known(rendered: String) -> Self {
        Self(rendered)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

fn yes_or_no(held: bool) -> &'static str {
    if held { "yes" } else { "no" }
}

/// What a configuration write does to anything live at the moment it lands.
///
/// It is **computed on the live side and handed to this transaction as a value**, which is
/// the only place the two state seams meet: the state authority works it out from sessions,
/// subscriptions and arms, and Configuration writes down what it was told ([ADR-0039]).
/// Neither knows about the other.
///
/// An empty radius is a real answer — *nothing live was touched* — and is distinct from an
/// entry that carries none, which is an authentication event that changed no record at all.
///
/// [ADR-0039]: ../../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BlastRadius {
    /// One consequence per line, in the words the administrator was shown before committing
    /// ([ADR-0015]) — who is cut mid-word, whose subscriptions drop, which loop goes vacant.
    ///
    /// [ADR-0015]: ../../../docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md
    consequences: Vec<String>,
}

impl BlastRadius {
    /// Nothing live is touched by this write.
    pub(crate) fn nothing_live() -> Self {
        Self::default()
    }

    /// The consequences the state authority computed.
    pub(crate) fn of(consequences: Vec<String>) -> Self {
        Self { consequences }
    }

    fn stored(&self) -> String {
        self.consequences.join("\n")
    }

    fn known(stored: &str) -> Self {
        if stored.is_empty() {
            return Self::nothing_live();
        }

        Self::of(stored.split('\n').map(str::to_owned).collect())
    }
}

/// A decision as the log holds it.
#[allow(dead_code)] // Read by the tests that hold the log to its promises, and by #59.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RecordedEntry {
    pub(crate) event: AuditEvent,
    pub(crate) actor: Option<UserId>,
    pub(crate) actor_name: String,
    pub(crate) source: Option<IpAddr>,
    pub(crate) write: Option<ConfigurationWrite>,
    pub(crate) operation: Option<String>,
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
    /// The console's filtered query — by actor and by target — is its own read surface and
    /// arrives with #59. Until it does, this is what the log is written against: an
    /// append-only log nothing can read is a promise rather than a record.
    #[allow(dead_code)]
    async fn recent_entries(&mut self, at_most: u32) -> Result<Vec<RecordedEntry>, StoreError>;
}

#[async_trait]
impl AuditLog for Transaction {
    async fn record(&mut self, entry: AuditEntry) -> Result<(), StoreError> {
        let write = entry.write.as_ref();

        sqlx::query(
            "INSERT INTO audit_entries \
             (recorded_at, event, actor_id, actor_name, source, \
              target_id, target_name, state_before, state_after, blast_radius, refusal, \
              operation) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(now())
        .bind(entry.event.stored())
        .bind(entry.actor.as_ref().map(UserId::as_str))
        .bind(&entry.actor_name)
        .bind(entry.source.map(|source| source.to_string()))
        .bind(write.and_then(|write| write.target.as_ref().map(RecordId::as_str)))
        .bind(write.map(|write| write.target_name.as_str()))
        .bind(write.and_then(|write| write.before.as_ref().map(Snapshot::as_str)))
        .bind(write.and_then(|write| write.after.as_ref().map(Snapshot::as_str)))
        .bind(write.map(|write| write.blast_radius.stored()))
        .bind(write.and_then(|write| write.refusal.as_deref()))
        .bind(entry.operation.as_deref())
        .execute(self.connection())
        .await
        .map_err(unavailable)?;

        Ok(())
    }

    async fn recent_entries(&mut self, at_most: u32) -> Result<Vec<RecordedEntry>, StoreError> {
        let rows = sqlx::query(
            "SELECT recorded_at, event, actor_id, actor_name, source, \
             target_id, target_name, state_before, state_after, blast_radius, refusal, \
             operation FROM audit_entries ORDER BY id DESC LIMIT ?",
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

                // The blast radius is the discriminator: a configuration write always
                // carries one, and an authentication event changed no record so it has none.
                let write =
                    row.get::<Option<String>, _>("blast_radius")
                        .map(|radius| ConfigurationWrite {
                            target: row
                                .get::<Option<String>, _>("target_id")
                                .map(|id| RecordId::of(&id)),
                            target_name: row.get("target_name"),
                            before: row
                                .get::<Option<String>, _>("state_before")
                                .map(Snapshot::known),
                            after: row
                                .get::<Option<String>, _>("state_after")
                                .map(Snapshot::known),
                            blast_radius: BlastRadius::known(&radius),
                            refusal: row.get("refusal"),
                        });

                Ok(RecordedEntry {
                    event,
                    actor: row.get::<Option<String>, _>("actor_id").map(UserId::known),
                    actor_name: row.get("actor_name"),
                    source,
                    write,
                    operation: row.get("operation"),
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
    use crate::configuration::users::{NewUser, User, Users};

    fn a_sign_in_by(actor: Option<&UserId>, name: &str) -> AuditEntry {
        AuditEntry {
            event: AuditEvent::SignInSucceeded,
            actor: actor.cloned(),
            actor_name: name.to_owned(),
            source: Some(IpAddr::from([192, 0, 2, 7])),
            write: None,
            operation: None,
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
                write: None,
                operation: None,
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

    /// The stored names are on disk in customer deployments, so a variant that writes one
    /// name and reads back another is a log this binary cannot read. There is no way to make
    /// the compiler check the two `match`es agree, so this checks it instead.
    #[tokio::test]
    async fn every_event_reads_back_as_the_event_it_was_written_as() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        let every = [
            AuditEvent::SignInSucceeded,
            AuditEvent::SignInFailed,
            AuditEvent::SignedOut,
            AuditEvent::BootstrapRedeemed,
            AuditEvent::BootstrapRefused,
            AuditEvent::UserCreated,
            AuditEvent::UserEdited,
            AuditEvent::UserDeleted,
            AuditEvent::AccountLocked,
            AuditEvent::AccountUnlocked,
            AuditEvent::PasswordResetForced,
            AuditEvent::EnrolmentCodeIssued,
            AuditEvent::EnrolmentRedeemed,
            AuditEvent::EnrolmentRefused,
            AuditEvent::PasswordChanged,
            AuditEvent::PasswordChangeRefused,
        ];

        for event in every {
            transaction
                .record(AuditEntry {
                    event,
                    ..a_sign_in_by(None, "flight")
                })
                .await
                .expect("the entry to be recorded");
        }

        let read: Vec<AuditEvent> = transaction
            .recent_entries(100)
            .await
            .expect("the log to be readable")
            .into_iter()
            .rev()
            .map(|entry| entry.event)
            .collect();

        assert_eq!(read, every);
    }

    /// A credential readable out of the audit log is one anybody who may read the log holds.
    #[tokio::test]
    async fn an_enrolment_snapshot_says_when_a_code_dies_and_never_what_it_is() {
        let rendered = Snapshot::of_enrolment(1_800_000_000_000);

        assert_eq!(
            rendered.as_str(),
            "enrolment_code=outstanding expires_at=1800000000000"
        );
    }

    fn a_user_named(username: &str) -> User {
        User {
            id: UserId::known("an-id".to_owned()),
            username: username.to_owned(),
            is_system_administrator: false,
            is_locked: false,
            has_password: true,
            external_identity: None,
        }
    }

    /// Every configuration write is audited with before and after **plus the blast radius**
    /// (v1 §12). The radius is computed on the live side and arrives here as a value, so the
    /// transaction that writes it knows nothing about where it came from ([ADR-0039]).
    ///
    /// [ADR-0039]: ../../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
    #[tokio::test]
    async fn records_a_configuration_write_with_before_and_after_and_the_blast_radius() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let administrator = a_user(&mut transaction, "root").await;
        let target = RecordId::of(a_user(&mut transaction, "flight").await.as_str());
        let radius = BlastRadius::of(vec!["flight loses their session on Capcom".to_owned()]);

        transaction
            .record(AuditEntry {
                event: AuditEvent::AccountLocked,
                actor: Some(administrator),
                actor_name: "root".to_owned(),
                source: None,
                operation: None,
                write: Some(ConfigurationWrite {
                    target: Some(target.clone()),
                    target_name: "flight".to_owned(),
                    before: Some(a_user_named("flight").snapshot()),
                    after: Some(
                        (User {
                            is_locked: true,
                            ..a_user_named("flight")
                        })
                        .snapshot(),
                    ),
                    blast_radius: radius.clone(),
                    refusal: None,
                }),
            })
            .await
            .expect("the entry to be recorded");

        let entries = transaction
            .recent_entries(1)
            .await
            .expect("the log to be readable");
        let write = entries[0].write.as_ref().expect("a configuration write");
        assert_eq!(write.target.as_ref(), Some(&target));
        assert_eq!(write.target_name, "flight");
        assert_ne!(write.before, write.after, "the write recorded no change");
        assert_eq!(write.blast_radius, radius);
        assert_eq!(write.refusal, None);
    }

    /// Refused administration writes are audited; refused reads are not (v1 §3). A refusal
    /// has a before and no after, because nothing happened.
    #[tokio::test]
    async fn records_a_refused_write_with_the_reason_and_nothing_after_it() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        transaction
            .record(AuditEntry {
                event: AuditEvent::UserDeleted,
                actor: None,
                actor_name: "root".to_owned(),
                source: None,
                operation: None,
                write: Some(ConfigurationWrite {
                    target: None,
                    target_name: "root".to_owned(),
                    before: Some(a_user_named("root").snapshot()),
                    after: None,
                    blast_radius: BlastRadius::nothing_live(),
                    refusal: Some("that is the last system administrator".to_owned()),
                }),
            })
            .await
            .expect("the entry to be recorded");

        let entries = transaction
            .recent_entries(1)
            .await
            .expect("the log to be readable");
        let write = entries[0].write.as_ref().expect("a configuration write");
        assert_eq!(write.after, None);
        assert_eq!(
            write.refusal.as_deref(),
            Some("that is the last system administrator")
        );
    }

    /// The blast radius is the discriminator: an entry carrying one is a configuration
    /// write, and an authentication event changed no record so it carries none.
    #[tokio::test]
    async fn an_authentication_event_records_no_configuration_write() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        transaction
            .record(a_sign_in_by(None, "flight"))
            .await
            .expect("the entry to be recorded");

        let entries = transaction
            .recent_entries(1)
            .await
            .expect("the log to be readable");
        assert!(entries[0].write.is_none());
    }

    /// The log outlives the records it references on both sides of the entry ([ADR-0028]).
    ///
    /// [ADR-0028]: ../../../docs/adr/0028-the-audit-log-records-decisions-not-traffic.md
    #[tokio::test]
    async fn deleting_a_user_leaves_the_entries_naming_them_as_a_target_attributed() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");
        let target = RecordId::of(a_user(&mut transaction, "flight").await.as_str());
        transaction
            .record(AuditEntry {
                event: AuditEvent::UserDeleted,
                actor: None,
                actor_name: "root".to_owned(),
                source: None,
                operation: None,
                write: Some(ConfigurationWrite {
                    target: Some(target.clone()),
                    target_name: "flight".to_owned(),
                    before: Some(a_user_named("flight").snapshot()),
                    after: None,
                    blast_radius: BlastRadius::nothing_live(),
                    refusal: None,
                }),
            })
            .await
            .expect("the entry to be recorded");

        sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(target.as_str())
            .execute(transaction.connection())
            .await
            .expect("the user to be deleted");

        let entries = transaction
            .recent_entries(10)
            .await
            .expect("the log to be readable");
        let write = entries[0].write.as_ref().expect("a configuration write");
        assert_eq!(write.target.as_ref(), Some(&target));
        assert_eq!(write.target_name, "flight");
    }

    /// A forced password reset changes nothing a reader can see except the credential, so
    /// the snapshot has to say whether one is set or the entry records two identical lines.
    #[tokio::test]
    async fn a_snapshot_says_whether_a_password_is_set_and_never_what_it_is() {
        let enrolled = a_user_named("flight").snapshot();
        let reset = (User {
            has_password: false,
            ..a_user_named("flight")
        })
        .snapshot();

        assert_ne!(enrolled, reset);
        assert!(reset.as_str().contains("password=none"), "{reset:?}");
    }

    /// A write turned away before it reached a record touched nothing, so it records the
    /// operation instead of a before, an after and a radius there are none of.
    #[tokio::test]
    async fn records_an_attempt_that_never_reached_a_record_by_the_operation_it_named() {
        let (_directory, store) = a_temporary_store().await;
        let mut transaction = store.begin().await.expect("a transaction");

        transaction
            .record(AuditEntry {
                event: AuditEvent::AdministrationRefused,
                actor: None,
                actor_name: "flight".to_owned(),
                source: Some(IpAddr::from([198, 51, 100, 9])),
                write: None,
                operation: Some("POST /api/users".to_owned()),
            })
            .await
            .expect("the entry to be recorded");

        let entries = transaction
            .recent_entries(1)
            .await
            .expect("the log to be readable");
        assert_eq!(entries[0].operation.as_deref(), Some("POST /api/users"));
        assert!(entries[0].write.is_none());
    }

    #[tokio::test]
    async fn a_snapshot_says_everything_a_later_reader_needs_to_see_what_changed() {
        let held = (User {
            is_system_administrator: true,
            ..a_user_named("root")
        })
        .snapshot();
        let taken_away = a_user_named("root").snapshot();

        assert_ne!(held, taken_away);
        assert!(held.as_str().contains("root"), "{held:?}");
    }
}

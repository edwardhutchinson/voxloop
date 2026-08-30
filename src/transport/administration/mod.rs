//! System administration: the audited write path, and the pages that write through it.
//!
//! Users are the console's first page ([`users`]); roles and loops are the two configuration
//! objects voice authority is expressed over ([`roles`], [`loops`]). All three are
//! administered the same way, and three things about that way are settled here rather than
//! per operation:
//!
//! - **The write and its audit entry commit together.** One transaction opened by the
//!   handler, both written through it, one commit ([ADR-0038]) — which is why Audit is not a
//!   module somebody could forget to call ([ADR-0060]).
//! - **Every write is audited with before and after plus the blast radius** (v1 §12). The
//!   radius is computed on the live side and handed in as a value ([ADR-0039]), so nothing
//!   here knows how it was worked out and nothing there knows it is being written down.
//! - **Refused writes are audited; refused reads are not** (v1 §3), and a refusal says *you
//!   may not* with the reason rather than pretending the operation is not there.
//!
//! Account lock and forced password reset live in [`users`], on the system-administration
//! side of ADR-0003's split. Forced *relinquish* is the operational one, it is conferred by
//! the `control` rung, and it is deliberately somewhere else.
//!
//! [ADR-0003]: ../../../docs/adr/0003-operational-authority-follows-the-role.md
//! [ADR-0038]: ../../../docs/adr/0038-sqlite-behind-domain-shaped-repositories.md
//! [ADR-0039]: ../../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
//! [ADR-0060]: ../../../docs/adr/0060-a-seam-names-domain-operations.md

pub(super) mod loops;
pub(super) mod roles;
pub(super) mod users;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use super::{Api, answers, name_as_it_stands};
use crate::authorisation::Caller;
use crate::configuration::{
    AdministrationRefused, AuditEntry, AuditEvent, AuditLog, BlastRadius, Change,
    ConfigurationWrite, Record, StoreError, Transaction, UserId,
};
use crate::telemetry::module;

/// The user whose behalf this request acts on.
///
/// Unreachable as anything else: every route in this module is registered under
/// `SystemAdministration`, which resolves a user before the handler runs. It is an `Option`
/// rather than an assumption so that the impossible case cannot be answered with a
/// mis-attributed audit entry, which is the one failure this module must not have.
fn acting(caller: &Caller) -> Option<&UserId> {
    match caller {
        Caller::User { id, .. } => Some(id),
        Caller::Nobody => None,
    }
}

fn unreachable_caller() -> Response {
    answers::refusal("That operation is for a system administrator.")
}

/// Whoever is administering, as the log will record them.
struct Administrator {
    id: UserId,
    /// The name as it stands, snapshotted into every entry this administrator writes.
    name: String,
}

impl Administrator {
    async fn of(transaction: &mut Transaction, id: &UserId) -> Result<Self, StoreError> {
        Ok(Self {
            id: id.clone(),
            name: name_as_it_stands(transaction, id).await?,
        })
    }

    fn wrote(&self, event: AuditEvent, write: ConfigurationWrite) -> AuditEntry {
        AuditEntry {
            event,
            actor: Some(self.id.clone()),
            actor_name: self.name.clone(),
            // An administration write is an act by somebody the store already recognises, so
            // where it came from adds nothing the actor does not already say.
            source: None,
            write: Some(write),
            operation: None,
        }
    }
}

/// What this write does to anything live at the moment it lands.
///
/// The state authority computes it from sessions, subscriptions and arms, and hands it over
/// as a value ([ADR-0039]) — this is the one line that changes when there are sessions for a
/// lock, a deletion or a grid edit to touch (#37, #53). Until then there is nothing live for
/// any write here to touch, and an empty radius is the honest answer rather than a
/// placeholder.
///
/// [ADR-0039]: ../../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
fn nothing_live() -> BlastRadius {
    BlastRadius::nothing_live()
}

/// Create a record, audit it, and commit the two together.
///
/// Creation is the write with no record before it, so it is the one that cannot go through
/// [`administer`]: there is nothing to read, and a refusal names no id because the record
/// never came into existence.
async fn create<T: Record>(
    api: &Api,
    acting: &UserId,
    event: AuditEvent,
    name: String,
    make: impl AsyncFnOnce(&mut Transaction) -> Result<Option<T>, AdministrationRefused>,
    answer: impl AsyncFnOnce(&mut Transaction, &T) -> Result<Response, StoreError>,
) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let administrator = Administrator::of(&mut transaction, acting).await?;

    let made = match make(&mut transaction).await {
        Ok(Some(made)) => made,
        // Unreachable: the record was written through this very transaction.
        Ok(None) => {
            transaction.roll_back().await?;
            return Ok(answers::cannot("That record could not be created."));
        }
        Err(AdministrationRefused::Store(error)) => return Err(error),
        Err(refusal) => {
            transaction.roll_back().await?;
            return refuse(
                api,
                &administrator,
                event,
                ConfigurationWrite {
                    // Nothing to name, nothing before it and nothing after it: the record
                    // never came into existence.
                    target: None,
                    target_name: name,
                    before: None,
                    after: None,
                    blast_radius: nothing_live(),
                    refusal: Some(reason(&refusal)),
                },
                answer_to(&refusal),
            )
            .await;
        }
    };

    transaction
        .record(administrator.wrote(
            event,
            ConfigurationWrite {
                target: Some(made.recorded_id()),
                target_name: made.recorded_name().to_owned(),
                before: None,
                after: Some(made.snapshot()),
                blast_radius: nothing_live(),
                refusal: None,
            },
        ))
        .await?;

    let answered = answer(&mut transaction, &made).await?;
    transaction.commit().await?;

    tracing::info!(target: module::CONFIGURATION, ?event, "a record was created");

    Ok(answered)
}

/// Make one write to a record, audit it, and commit the two together.
///
/// This is the shape of every configuration write VoxLoop makes, which is why it is one
/// function rather than the same eight lines per operation: one transaction, the write
/// through it, the entry through it carrying before, after and the blast radius, one commit.
/// A write that refuses says so and is audited anyway; a write that named nobody is a
/// not-found rather than a refusal, and there is no record for an entry to be about.
///
/// `what` is the record in the words a human reads — *user*, *role*, *loop* — and is the
/// whole of what differs between the three pages on this path.
async fn administer<T: Record>(
    api: &Api,
    acting: &UserId,
    event: AuditEvent,
    what: &'static str,
    read: impl AsyncFnOnce(&mut Transaction) -> Result<Option<T>, StoreError>,
    write: impl AsyncFnOnce(&mut Transaction) -> Result<Option<Change<T>>, AdministrationRefused>,
    answer: impl AsyncFnOnce(&mut Transaction, &T) -> Result<Response, StoreError>,
) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let administrator = Administrator::of(&mut transaction, acting).await?;

    let change = match write(&mut transaction).await {
        Ok(Some(change)) => change,
        Ok(None) => {
            transaction.roll_back().await?;
            return Ok(answers::no_such(what));
        }
        Err(AdministrationRefused::Store(error)) => return Err(error),
        Err(refusal) => {
            transaction.roll_back().await?;
            return refuse_about(api, &administrator, event, read, &refusal).await;
        }
    };

    transaction
        .record(administrator.wrote(event, ConfigurationWrite::about(&change, nothing_live())))
        .await?;

    // Answered through the same transaction the write went through, so what comes back is
    // the record as it stands rather than as it stood at some other moment.
    let answered = match &change.after {
        Some(after) => Some(answer(&mut transaction, after).await?),
        None => None,
    };
    transaction.commit().await?;

    tracing::info!(target: module::CONFIGURATION, ?event, "a record was administered");

    Ok(answered.unwrap_or_else(|| StatusCode::NO_CONTENT.into_response()))
    // A deletion has nothing to answer with, which is the whole of what it says.
}

/// Why a write did not happen, in one sentence, and how it is answered.
///
/// Configuration answers refused and never *why* in a form somebody can act on, and turning
/// that into something a human reads is Transport's job — the same division `routes.rs` makes
/// for a requirement nobody met. The log holds this sentence rather than the error's own
/// wording, so what the console shows later is what the administrator was told at the time.
///
/// A refusal says *you may not* with the reason rather than hiding the operation (v1 §3). A
/// name already taken is a different thing and answered as one: the caller may make users,
/// and this particular attempt at it will not do.
fn refusal_of(refusal: &AdministrationRefused) -> (String, StatusCode) {
    match refusal {
        AdministrationRefused::LastSystemAdministrator => (
            "This is the last system administrator this deployment can be administered by, and \
             the last one cannot be removed."
                .to_owned(),
            StatusCode::FORBIDDEN,
        ),
        AdministrationRefused::NameTaken { what, name } => (
            format!("The {what} {name:?} is already taken."),
            StatusCode::BAD_REQUEST,
        ),
        AdministrationRefused::NobodyMayOccupy => (
            "A role must admit at least one occupant.".to_owned(),
            StatusCode::BAD_REQUEST,
        ),
        AdministrationRefused::IncompleteOrder => (
            "That order does not name every loop exactly once. Read the loops again and set it \
             from what is there."
                .to_owned(),
            StatusCode::BAD_REQUEST,
        ),
        // Unreachable: a store fault is answered as a fault rather than as a refusal.
        AdministrationRefused::Store(_) => (
            "That write could not be made.".to_owned(),
            StatusCode::BAD_REQUEST,
        ),
    }
}

fn reason(refusal: &AdministrationRefused) -> String {
    refusal_of(refusal).0
}

fn answer_to(refusal: &AdministrationRefused) -> Response {
    let (reason, status) = refusal_of(refusal);

    match status {
        StatusCode::FORBIDDEN => answers::refusal(&reason),
        _ => answers::cannot(&reason),
    }
}

/// Record a write refused over the record it was about, and answer with why.
///
/// The record is read here rather than before every write, because a refusal is the rare
/// path and the entry needs a transaction of its own anyway: the attempt abandoned whatever
/// it had open.
async fn refuse_about<T: Record>(
    api: &Api,
    administrator: &Administrator,
    event: AuditEvent,
    read: impl AsyncFnOnce(&mut Transaction) -> Result<Option<T>, StoreError>,
    refusal: &AdministrationRefused,
) -> Result<Response, StoreError> {
    let mut transaction = api.store.begin().await?;
    let before = read(&mut transaction).await?;
    transaction.roll_back().await?;

    refuse(
        api,
        administrator,
        event,
        ConfigurationWrite {
            target: before.as_ref().map(Record::recorded_id),
            target_name: before
                .as_ref()
                .map_or_else(String::new, |record| record.recorded_name().to_owned()),
            before: before.as_ref().map(Record::snapshot),
            // Nothing after it: the write did not happen.
            after: None,
            blast_radius: nothing_live(),
            refusal: Some(reason(refusal)),
        },
        answer_to(refusal),
    )
    .await
}

/// Record a refused write, and answer with why.
///
/// Refused administration writes are audited (v1 §3), and the entry needs a transaction of
/// its own because the refusal has abandoned whatever the attempt had open.
async fn refuse(
    api: &Api,
    administrator: &Administrator,
    event: AuditEvent,
    write: ConfigurationWrite,
    answer: Response,
) -> Result<Response, StoreError> {
    tracing::warn!(
        target: module::CONFIGURATION,
        actor = %administrator.id.as_str(),
        ?event,
        "an administration write was refused"
    );

    let mut transaction = api.store.begin().await?;
    transaction
        .record(administrator.wrote(event, write))
        .await?;
    transaction.commit().await?;

    Ok(answer)
}

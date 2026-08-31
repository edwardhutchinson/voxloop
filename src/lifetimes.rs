//! The clocks that end things nobody ended.
//!
//! v1 §2 fixes three rules together, and the third only makes sense beside the first two:
//! there is **no idle timeout on a session**, there is **no absolute cap on a sign-in**, and
//! a **sign-in ends after 24 hours with no deliberate act**. That last clock runs **only in
//! the lobby** — assuming a role stops it ([ADR-0023]).
//!
//! Short idle timeouts were refused outright: an operator watching telemetry is idle and very
//! much on console, and a timeout that signs them out is that mistake by another route. What
//! this window is for is reaping **abandoned** sign-ins, and an occupied role is by definition
//! not abandoned — an unattended one already surfaces as `away`, through machinery built for
//! exactly that.
//!
//! The absolute cap was proposed and rejected, so nothing here ends a sign-in for being old.
//! The consequence — a console occupied indefinitely never re-authenticates — is accepted
//! rather than solved, because attribution of voice holds anyway and *who is at that console*
//! is answered by occupancy rather than by the credential.
//!
//! This sits at the top of the call graph beside Transport ([ADR-0062]): it receives a tick
//! and calls Configuration and the state authority, so nothing calls into it and no cycle is
//! made. The two state seams meet here the only way they ever do — by passing a value.
//!
//! [ADR-0023]: ../../docs/adr/0023-sign-in-is-to-the-application-and-a-role-is-assumed.md
//! [ADR-0062]: ../../docs/adr/0062-the-call-graph-is-acyclic-and-effects-modules-are-sinks.md

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::configuration::{AuditEntry, AuditEvent, AuditLog, SignIns, Store, StoreError, Users};
use crate::state::StateAuthority;
use crate::telemetry::module;

/// How long a sign-in standing in the lobby lasts with nobody doing anything deliberate.
const THE_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);

/// How often the window is checked.
///
/// Nothing turns on the precision: the window is a day, and a sign-in reaped a quarter of an
/// hour late is a sign-in reaped. Sweeping rarely is what keeps this off the critical path of
/// a deployment doing real work.
const SWEEP: Duration = Duration::from_secs(15 * 60);

/// The running sweep, and the handle that stops it.
pub(crate) struct Sweeping {
    task: JoinHandle<()>,
}

impl Sweeping {
    /// Stop sweeping. Nothing is half-done: each pass is its own transaction.
    pub(crate) fn stop(self) {
        self.task.abort();
    }
}

/// Start ending abandoned sign-ins, on and on until the process stops.
pub(crate) fn sweeping(store: Arc<Store>, state: Arc<StateAuthority>) -> Sweeping {
    Sweeping {
        task: tokio::spawn(async move {
            let mut sweep = tokio::time::interval(SWEEP);
            sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            loop {
                sweep.tick().await;

                if let Err(error) = end_abandoned_sign_ins(&store, &state, THE_WINDOW).await {
                    tracing::error!(target: module::CONFIGURATION, %error, "the sign-in window could not be swept");
                }
            }
        }),
    }
}

/// End every sign-in that has stood in the lobby for `window` with nobody doing anything.
///
/// The sign-ins holding a session are asked for first and handed to the write as a value:
/// live state and durable state meet by passing data and never by reaching across
/// ([ADR-0039]). Each one ended is audited — it is a sign-in ending without anybody ending
/// it, which is a decision the deployment made and exactly what the log is for.
///
/// [ADR-0039]: ../../docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md
async fn end_abandoned_sign_ins(
    store: &Store,
    state: &StateAuthority,
    window: Duration,
) -> Result<(), StoreError> {
    let holding_a_session = state.sign_ins_holding_a_session();

    let mut transaction = store.begin().await?;
    let ended = transaction
        .end_sign_ins_idle_for(window, &holding_a_session)
        .await?;

    for user in &ended {
        let name = transaction
            .user(user)
            .await?
            .map_or_else(String::new, |user| user.username);

        transaction
            .record(AuditEntry {
                event: AuditEvent::SignInExpired,
                actor: Some(user.clone()),
                actor_name: name,
                // Nobody came from anywhere: this is the deployment noticing a window ran
                // out, not somebody doing something from a machine.
                source: None,
                write: None,
                operation: None,
            })
            .await?;
    }
    transaction.commit().await?;

    if !ended.is_empty() {
        tracing::info!(
            target: module::IDENTITY,
            ended = ended.len(),
            "sign-ins ended after the window with no deliberate act"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{NewRole, NewUser, Roles, SignInToken, UserId, a_temporary_store};

    /// A signed-in user, and the sign-in they hold.
    async fn signed_in(store: &Store, username: &str) -> (UserId, SignInToken) {
        let mut transaction = store.begin().await.expect("a transaction");
        let user = transaction
            .create_user(NewUser {
                username: username.to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("the user to be created");
        let token = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");
        transaction.commit().await.expect("the sign-in to land");

        (user, token)
    }

    async fn is_signed_in(store: &Store, token: &SignInToken) -> bool {
        let mut transaction = store.begin().await.expect("a transaction");
        let holder = transaction
            .holder_of(token)
            .await
            .expect("the read to answer");
        transaction.roll_back().await.expect("the read to close");

        holder.is_some()
    }

    /// The window is a day; a window of nothing is the same rule with the clock run all the
    /// way down, which is how these exercise it without waiting.
    const AT_ONCE: Duration = Duration::ZERO;

    #[tokio::test]
    async fn ends_a_sign_in_that_has_stood_in_the_lobby_past_the_window() {
        let (_directory, store) = a_temporary_store().await;
        let (_user, abandoned) = signed_in(&store, "flight").await;

        end_abandoned_sign_ins(&store, &StateAuthority::empty(), AT_ONCE)
            .await
            .expect("the sweep to run");

        assert!(!is_signed_in(&store, &abandoned).await);
    }

    /// The clock runs only in the lobby: a sign-in holding a session is not abandoned,
    /// whatever its owner has or has not clicked (ADR-0023).
    #[tokio::test]
    async fn leaves_a_sign_in_that_holds_a_session_alone() {
        let (_directory, store) = a_temporary_store().await;
        let (occupant, holding_a_role) = signed_in(&store, "flight").await;
        let mut transaction = store.begin().await.expect("a transaction");
        let role = transaction
            .create_role(NewRole {
                name: "Flight Director".to_owned(),
                max_occupants: Some(1),
            })
            .await
            .expect("the role to be created");
        transaction.commit().await.expect("the role to land");
        let state = StateAuthority::empty();
        state.a_session_is_held(&holding_a_role, &occupant, &role);

        end_abandoned_sign_ins(&store, &state, AT_ONCE)
            .await
            .expect("the sweep to run");

        assert!(
            is_signed_in(&store, &holding_a_role).await,
            "an operator holding a role was signed out for failing to click anything"
        );
    }

    /// A sign-in ending with nobody ending it is a decision the deployment made, and it is
    /// recorded as its own event rather than as a sign-out.
    #[tokio::test]
    async fn records_what_it_ended_against_whoever_held_it() {
        let (_directory, store) = a_temporary_store().await;
        let (user, _abandoned) = signed_in(&store, "flight").await;

        end_abandoned_sign_ins(&store, &StateAuthority::empty(), AT_ONCE)
            .await
            .expect("the sweep to run");

        let expired = entries_of(&store, AuditEvent::SignInExpired).await;
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].actor_name, "flight");
        assert_eq!(expired[0].actor.as_ref(), Some(&user));
    }

    /// A sweep that ends nothing says nothing. A window running out is worth an entry;
    /// a window not running out is the ordinary state of everything.
    #[tokio::test]
    async fn records_nothing_where_there_was_nothing_to_end() {
        let (_directory, store) = a_temporary_store().await;
        let (_user, token) = signed_in(&store, "flight").await;

        end_abandoned_sign_ins(&store, &StateAuthority::empty(), THE_WINDOW)
            .await
            .expect("the sweep to run");

        assert!(is_signed_in(&store, &token).await);
        assert!(
            entries_of(&store, AuditEvent::SignInExpired)
                .await
                .is_empty()
        );
    }

    /// The entries the log holds for one event.
    async fn entries_of(
        store: &Store,
        event: AuditEvent,
    ) -> Vec<crate::configuration::RecordedEntry> {
        let mut transaction = store.begin().await.expect("a transaction");
        let entries = transaction
            .recent_entries(100)
            .await
            .expect("the log to be readable");
        transaction.roll_back().await.expect("the read to close");

        entries
            .into_iter()
            .filter(|entry| entry.event == event)
            .collect()
    }
}

//! What the media plane's reports mean.
//!
//! The media plane is a sink ([ADR-0062]): it calls nothing, answers nothing, and says what
//! it has to say on a channel. **Something has to decide what a report means, and it cannot
//! be the sink** — so this reads the channel and writes live state, sitting at the top of the
//! call graph beside Transport, the on-box CLI and the sign-in clock. It is called by
//! nothing, which is what keeps the graph acyclic.
//!
//! Like those three, it is **not a module** in the [ADR-0060] sense. It promises nothing to
//! anybody and has no interface to substitute; it is a channel, two writes and one decision,
//! and the enumeration in [`docs/spec/modules.md`] says so.
//!
//! There is little judgement here and that is deliberate. The client is the driver of the
//! media path ladder and this end is the backstop ([ADR-0042]), so what arrives is already a
//! rung — the translation from ICE and DTLS happened inside the adapter, which is the only
//! thing entitled to know what those words mean. What is left is *which session*, and *the
//! worker is gone means every session*.
//!
//! [ADR-0042]: ../../docs/adr/0042-the-media-path-has-its-own-ladder.md
//! [ADR-0060]: ../../docs/adr/0060-a-seam-names-domain-operations.md
//! [ADR-0062]: ../../docs/adr/0062-the-call-graph-is-acyclic-and-effects-modules-are-sinks.md
//! [`docs/spec/modules.md`]: ../../docs/spec/modules.md

use std::sync::Arc;

use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::media_plane::{Reported, Reports};
use crate::state::StateAuthority;
use crate::telemetry::module;

/// The running watch, the handle that stops it, and the one thing it can ask for.
pub(crate) struct Watching {
    task: JoinHandle<()>,
    nothing_is_carried: Arc<Notify>,
}

impl Watching {
    /// Wait until the deployment has lost its whole purpose.
    ///
    /// **A dead worker is not a session's problem, it is the deployment's**, and the
    /// difference decides who acts. One session's media path going `lost` is announced,
    /// survivable and the operator's to judge ([ADR-0042]) — nothing here reaps that. The
    /// worker going means every transport is gone and no new one can be built, and
    /// [ADR-0070] rules out recovering in place: a new Worker means a new Router, so every
    /// session's media path would have to be rebuilt from a process that has just proved it
    /// can lose them.
    ///
    /// So this is the signal to stop, and the answer is the one that is already exercised
    /// on every deployment: go down, let systemd bring the unit back, and end every session
    /// honestly rather than serving consoles that will never make a sound again. It is the
    /// same posture as refusing to start without a worker in the first place.
    ///
    /// [ADR-0042]: ../../docs/adr/0042-the-media-path-has-its-own-ladder.md
    /// [ADR-0070]: ../../docs/adr/0070-the-mediasoup-worker-is-a-thread-of-this-process.md
    pub(crate) async fn until_nothing_is_carried(&self) {
        self.nothing_is_carried.notified().await;
    }

    /// Stop watching. Nothing is half-done: each report is one write under one lock.
    pub(crate) fn stop(self) {
        self.task.abort();
    }
}

/// Start turning the media plane's reports into live state, on and on until the process stops.
pub(crate) fn watching(reports: Reports, state: Arc<StateAuthority>) -> Watching {
    let nothing_is_carried = Arc::new(Notify::new());

    Watching {
        task: tokio::spawn({
            let nothing_is_carried = Arc::clone(&nothing_is_carried);
            async move { watch(reports, &state, &nothing_is_carried).await }
        }),
        nothing_is_carried,
    }
}

/// Read the channel until the media plane stops reporting.
///
/// The channel closing means the media plane is gone, and by then either the process is on
/// its way out or the worker's death has already been reported and acted on. There is nothing
/// left to do in either case, so this ends rather than looping on a dead receiver.
async fn watch(mut reports: Reports, state: &StateAuthority, nothing_is_carried: &Notify) {
    while let Some(reported) = reports.recv().await {
        match reported {
            Reported::ThePath { of, is } => {
                tracing::debug!(
                    target: module::MEDIA_PLANE,
                    session = of.as_str(),
                    media_path = is.as_str(),
                    "the server's end of a media path moved"
                );
                state.the_server_sees(&of, is);
            }
            // **Every session at once**, because a worker's death takes every transport with
            // it and there is no per-session version of this fact. It is loud in the log
            // because it is the deployment losing its whole purpose while the console keeps
            // working — exactly the failure that would otherwise be found by somebody
            // pressing a key and being heard by nobody.
            //
            // Live state moves **before** the process is asked to stop, and both happen: the
            // shutdown is graceful, so sockets have a few seconds left in which the last
            // thing they are told about themselves should be true.
            Reported::NothingIsCarried { detail } => {
                tracing::error!(
                    target: module::MEDIA_PLANE,
                    detail,
                    "nothing is carrying audio"
                );
                state.nothing_is_carried();
                // `notify_one` rather than `notify_waiters`, because this leaves a permit
                // for a waiter that has not arrived yet: a worker that dies in the moment
                // between the watch starting and the composition root reaching its select
                // would otherwise be a death nobody was listening for.
                nothing_is_carried.notify_one();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{NewRole, NewUser, Roles, SignIns, Store, Users, a_temporary_store};
    use crate::media_plane::a_recording_media_plane;
    use crate::state::{Assuming, InReach, MediaPath, SessionId};

    /// A live session on a running state authority, and a media plane that carries nothing.
    async fn a_session(store: &Store, state: &StateAuthority) -> SessionId {
        let mut transaction = store.begin().await.expect("a transaction");
        let user = transaction
            .create_user(NewUser {
                username: "flight".to_owned(),
                password_hash: None,
                is_system_administrator: false,
            })
            .await
            .expect("the user to be created");
        let sign_in = transaction
            .open_sign_in(&user)
            .await
            .expect("the sign-in to open");
        let role = transaction
            .create_role(NewRole {
                name: "Flight Director".to_owned(),
                max_occupants: None,
            })
            .await
            .expect("the role to be created");
        transaction.commit().await.expect("the seat to be made");

        state
            .assume(Assuming {
                sign_in,
                occupant: user,
                role,
                limit: None,
            })
            .expect("the seat to be free")
            .session
    }

    /// What the session's document says its media path is, right now.
    fn as_documented(state: &StateAuthority, session: &SessionId) -> MediaPath {
        state
            .presence(session, Vec::<InReach>::new())
            .expect("a live session")
            .1
            .media_path
    }

    /// The server's end is the backstop, and it reaches live state without anybody asking.
    #[tokio::test]
    async fn a_report_about_one_path_moves_that_session_and_no_other() {
        let (_directory, store) = a_temporary_store().await;
        let state = Arc::new(StateAuthority::empty());
        let session = a_session(&store, &state).await;

        let (_media, reports, recording) = a_recording_media_plane();
        let watching = watching(reports, Arc::clone(&state));

        recording.the_worker_says(Reported::ThePath {
            of: session.clone(),
            is: MediaPath::Connected,
        });
        // The client has said nothing, and green needs both ends: one connected reading is
        // not a connected media path.
        settled().await;
        assert_eq!(as_documented(&state, &session), MediaPath::Lost);

        state.the_client_says(&session, MediaPath::Connected);
        assert_eq!(as_documented(&state, &session), MediaPath::Connected);

        watching.stop();
    }

    /// A worker's death is not one session's problem, and is not recorded as one.
    #[tokio::test]
    async fn the_worker_dying_takes_every_session_off_the_air_at_once() {
        let (_directory, store) = a_temporary_store().await;
        let state = Arc::new(StateAuthority::empty());
        let session = a_session(&store, &state).await;
        state.the_client_says(&session, MediaPath::Connected);
        state.the_server_sees(&session, MediaPath::Connected);
        assert_eq!(as_documented(&state, &session), MediaPath::Connected);

        let (_media, reports, recording) = a_recording_media_plane();
        let watching = watching(reports, Arc::clone(&state));

        recording.the_worker_says(Reported::NothingIsCarried {
            detail: "it stopped".to_owned(),
        });
        settled().await;

        assert_eq!(as_documented(&state, &session), MediaPath::Lost);
        // Nothing here ends a session. Sessions end because the process does, and they end
        // together — which is what a restart has always meant (ADR-0039).
        assert!(state.the_role_of(&session).is_some());

        watching.stop();
    }

    /// **A dead worker stops the binary; a dead media path for one session does not.** The
    /// two look alike on the console and are opposite decisions: one session's transport
    /// failing is the operator's to judge (ADR-0042), and the worker going means no
    /// transport can be built for anybody, with nobody left to leave the judgement with.
    #[tokio::test]
    async fn the_worker_dying_asks_the_process_to_stop() {
        let state = Arc::new(StateAuthority::empty());
        let (_media, reports, recording) = a_recording_media_plane();
        let watching = watching(reports, state);

        recording.the_worker_says(Reported::NothingIsCarried {
            detail: "it stopped".to_owned(),
        });

        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            watching.until_nothing_is_carried(),
        )
        .await
        .expect("the process to be asked to stop");

        watching.stop();
    }

    /// One session losing its path is not the deployment losing its purpose, so it asks for
    /// nothing. Getting this the other way round would take a whole control room off the air
    /// because one operator's laptop changed network.
    #[tokio::test]
    async fn one_path_going_asks_for_nothing() {
        let (_directory, store) = a_temporary_store().await;
        let state = Arc::new(StateAuthority::empty());
        let session = a_session(&store, &state).await;
        let (_media, reports, recording) = a_recording_media_plane();
        let watching = watching(reports, Arc::clone(&state));

        recording.the_worker_says(Reported::ThePath {
            of: session,
            is: MediaPath::Lost,
        });
        settled().await;

        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(50),
                watching.until_nothing_is_carried(),
            )
            .await
            .is_err(),
            "one session's media path took the whole deployment down"
        );

        watching.stop();
    }

    /// The reports are read on a task, so a test that asserted immediately would be racing it.
    /// Yielding is enough: there is no timer in the loop, only a receive.
    async fn settled() {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }
}

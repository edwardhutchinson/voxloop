//! The media plane's fake: **a recorder rather than a simulation** ([ADR-0064]).
//!
//! One of exactly two fakes in VoxLoop, and it earns its place by what the real adapter is:
//! a C++ worker negotiating ICE and DTLS, which is slow, external and nondeterministic. Every
//! other seam is tested against the real thing, and an in-memory repository is ruled out by
//! name.
//!
//! **It records what it was told and does nothing else.** It builds no transports, decides
//! nothing, and answers nothing — because the interface it stands behind answers nothing
//! either ([ADR-0062]), which is what makes standing behind it possible at all.
//!
//! **A fake that started making decisions would be the failure [ADR-0063] exists to
//! prevent.** If the fan-out decision lived below the seam this would have to reimplement
//! VoxLoop's routing, and every test about who hears whom would be a test of the
//! reimplementation — passing while the product was broken. So the rule here is absolute:
//! nothing in this file may look at what it was handed and behave differently because of it.
//!
//! It reports too, because the ladder has two ends and a test needs to be able to play the
//! server's. That is a test saying what the worker would have said, not this deciding
//! anything: [`Recording::the_worker_says`] takes the report whole and puts it on the wire.
//!
//! [ADR-0062]: ../../docs/adr/0062-the-call-graph-is-acyclic-and-effects-modules-are-sinks.md
//! [ADR-0063]: ../../docs/adr/0063-the-media-plane-executes-routing-it-never-computes-it.md
//! [ADR-0064]: ../../docs/adr/0064-tests-run-against-the-real-store.md

use std::sync::{Arc, Mutex};

use super::{
    Audience, Carriage, Carried, MediaPlane, Negotiated, Negotiation, Reported, Reporting, Reports,
    Telling, Way,
};
use crate::state::SessionId;

/// One thing the media plane was told to do, in the words it was told it in.
///
/// This is the whole of what a test asserts on. It names domain operations because the
/// interface does; there is no transport, producer or consumer here to assert against, and
/// there never will be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Instructed {
    APathWasOpenedFor(SessionId),
    ThePathWasClosedFor(SessionId),
    ThisClientWillHear {
        session: SessionId,
        what_it_can_decode: Negotiation,
    },
    ThisClientConnected {
        session: SessionId,
        way: Way,
        keys: Negotiation,
    },
    ThisClientSpeaks {
        session: SessionId,
        what_it_is_sending: Negotiation,
    },
    ThisClientHears {
        session: SessionId,
        carriage: Carried,
    },
    TheseShouldHear {
        talker: SessionId,
        audience: Audience,
    },
}

/// The recorder, and the way a test both reads and drives it.
///
/// It is one object rather than two because the instructions and the reports are the two
/// halves of one seam, and a test that had to hold them separately would be a test that could
/// assert on a media plane it was not driving.
pub(crate) struct Recording {
    instructed: Mutex<Vec<Instructed>>,
    reporting: Reporting,
    /// Where each session's signalling was told to go, kept so a test can play the worker's
    /// half of a negotiation without a worker.
    ///
    /// Keeping it is not deciding anything: the channel arrives as an argument like every
    /// other, and what goes down it is whatever [`Recording::the_worker_tells`] is handed.
    telling: Mutex<Vec<(SessionId, Telling)>>,
}

/// A media plane that carries nothing, and the tape it writes.
///
/// The `Reports` half is handed back exactly as the real adapter hands it back, so whatever
/// consumes it cannot tell which of the two it is reading.
pub(crate) fn a_recording_media_plane() -> (MediaPlane, Reports, Arc<Recording>) {
    let (reporting, reports) = tokio::sync::mpsc::unbounded_channel();
    let recording = Arc::new(Recording {
        instructed: Mutex::new(Vec::new()),
        reporting,
        telling: Mutex::new(Vec::new()),
    });

    (
        MediaPlane {
            carriage: Arc::clone(&recording) as Arc<dyn Carriage>,
        },
        reports,
        recording,
    )
}

impl Recording {
    /// Everything the media plane has been told, in the order it was told.
    pub(crate) fn instructions(&self) -> Vec<Instructed> {
        self.tape().clone()
    }

    /// Say what the worker would have said.
    ///
    /// The report is passed through whole. Nothing here composes one, and nothing here
    /// decides that an instruction ought to produce one — a real worker's reports arrive
    /// when ICE and DTLS say so, which is nothing a recorder can know.
    pub(crate) fn the_worker_says(&self, reported: Reported) {
        let _ = self.reporting.send(reported);
    }

    /// Say to one session what the worker would have said to it.
    ///
    /// The same rule as [`Recording::the_worker_says`]: the message is passed through whole,
    /// and nothing here decides that an instruction ought to produce one. A real worker
    /// answers when ICE, DTLS and a browser say so, which is nothing a recorder can know.
    pub(crate) fn the_worker_tells(&self, session: &SessionId, negotiated: Negotiated) {
        let told = match self.telling.lock() {
            Ok(told) => told,
            Err(poisoned) => poisoned.into_inner(),
        };

        for (whose, telling) in told.iter() {
            if whose == session {
                let _ = telling.send(negotiated.clone());
            }
        }
    }

    fn write(&self, instruction: Instructed) {
        self.tape().push(instruction);
    }

    /// A poisoned lock means a panic in a test, and the tape is still the tape.
    fn tape(&self) -> std::sync::MutexGuard<'_, Vec<Instructed>> {
        match self.instructed.lock() {
            Ok(tape) => tape,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Carriage for Recording {
    fn open_a_path_for(&self, session: &SessionId, telling: Telling) {
        match self.telling.lock() {
            Ok(mut told) => told.push((session.clone(), telling)),
            Err(poisoned) => poisoned.into_inner().push((session.clone(), telling)),
        }
        self.write(Instructed::APathWasOpenedFor(session.clone()));
    }

    fn close_the_path_of(&self, session: &SessionId) {
        self.write(Instructed::ThePathWasClosedFor(session.clone()));
    }

    fn the_client_will_hear(&self, session: &SessionId, what_it_can_decode: Negotiation) {
        self.write(Instructed::ThisClientWillHear {
            session: session.clone(),
            what_it_can_decode,
        });
    }

    fn the_client_connects(&self, session: &SessionId, way: Way, keys: Negotiation) {
        self.write(Instructed::ThisClientConnected {
            session: session.clone(),
            way,
            keys,
        });
    }

    fn the_client_speaks(&self, session: &SessionId, what_it_is_sending: Negotiation) {
        self.write(Instructed::ThisClientSpeaks {
            session: session.clone(),
            what_it_is_sending,
        });
    }

    fn the_client_hears(&self, session: &SessionId, carriage: &Carried) {
        self.write(Instructed::ThisClientHears {
            session: session.clone(),
            carriage: carriage.clone(),
        });
    }

    fn these_should_hear(&self, talker: &SessionId, audience: &Audience) {
        self.write(Instructed::TheseShouldHear {
            talker: talker.clone(),
            audience: audience.clone(),
        });
    }
}

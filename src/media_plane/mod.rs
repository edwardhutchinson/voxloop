//! Media plane — the part of the system that carries audio and knows nothing about why.
//!
//! It is told which subscribers should hear which talker and makes it so. **It never works
//! out who** ([ADR-0063]): reach is decided by the state authority and handed down, and a
//! loop reaches this module only as an opaque label on a destination. Getting that the other
//! way round would be quietly fatal — the recorder below would have to reimplement VoxLoop's
//! routing, and every test touching arming, subscription or reach would be testing the
//! reimplementation instead of the product, while looking green.
//!
//! **It is a sink** ([ADR-0062]). It calls nothing, and every operation on it returns
//! nothing: a caller never learns from the call whether the effect happened. What it has to
//! say it says on [`Reports`], and something above it decides what that means. That is not a
//! stylistic choice — the thing on the other side of this interface is a C++ worker running
//! its own event loop, and an interface that could refuse would be one that had to be
//! awaited from inside the state authority's lock.
//!
//! **Nothing mediasoup names leaves this module.** No `Producer`, no `Transport`, no
//! `IceState`. That is what module privacy is for ([ADR-0061]) and it is what makes the
//! recorder possible: everything below is behind [`Carriage`], which is private, and the two
//! things implementing it are [`worker`] and [`recorder`].
//!
//! The worker itself is a **thread of this process rather than a child of it**, which is not
//! what the deployment documents originally assumed — [ADR-0070] records the finding and
//! what supervision means as a result.
//!
//! [ADR-0061]: ../../docs/adr/0061-module-privacy-is-the-seam-enforcement.md
//! [ADR-0062]: ../../docs/adr/0062-the-call-graph-is-acyclic-and-effects-modules-are-sinks.md
//! [ADR-0063]: ../../docs/adr/0063-the-media-plane-executes-routing-it-never-computes-it.md
//! [ADR-0070]: ../../docs/adr/0070-the-mediasoup-worker-is-a-thread-of-this-process.md

// The recorder is a test double and nothing ships with it (ADR-0064): the real adapter is
// the only thing behind this seam in a running deployment.
#[cfg(test)]
mod recorder;
mod worker;

use std::sync::Arc;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::configuration::Media;
use crate::state::{MediaPath, SessionId};

#[cfg(test)]
pub(crate) use recorder::{Instructed, Recording, a_recording_media_plane};
pub(crate) use worker::Carriageway;

/// The channel the media plane reports on, and the only thing it ever says.
///
/// A sink's health is **observed, never returned** ([ADR-0062]), so this is where every fact
/// about the audio path arrives. Whoever holds the other end decides what each report means;
/// nothing here does.
pub(crate) type Reports = UnboundedReceiver<Reported>;

/// What the media plane has to say for itself.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Reported {
    /// The **server's** reading of one session's media path.
    ///
    /// One of the two ends ADR-0042 merges, and the backstop rather than the driver: the
    /// client is far better placed to tell a transient fault from a terminal one, and this
    /// end covers the client that is wedged or lying. [`crate::state::MediaPath`] carries
    /// the argument in full.
    ThePath { of: SessionId, is: MediaPath },
    /// Nothing is carrying audio at all, and this is what happened.
    ///
    /// A worker's death takes every transport with it, so this is not one session's problem
    /// and is not reported as one.
    NothingIsCarried { detail: String },
}

/// Who should hear one session's uplink, and where each of them hears it.
///
/// It is an **answer, not a question** ([ADR-0063]). The state authority worked out the
/// audience and this carries the result down; nothing in this module may narrow it, widen it
/// or ask why it says what it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Audience {
    pub(crate) hearing: Vec<Hearing>,
}

/// One listener, and the destination they hear this talker on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hearing {
    pub(crate) listener: SessionId,
    pub(crate) destination: Destination,
}

/// A place audio is addressed to, as far as the media plane is concerned.
///
/// **It is a label and nothing else** ([ADR-0063]). A loop identifier crosses as one of
/// these because [ADR-0009]'s recording tap is addressed per (talker, destination loop), so
/// the media plane has to be able to tell two destinations apart — but it never reasons
/// about what the label names, and `LoopId` deliberately does not cross.
///
/// [ADR-0009]: ../../docs/adr/0009-recording-taps-plain-rtp-on-loopback.md
/// [ADR-0063]: ../../docs/adr/0063-the-media-plane-executes-routing-it-never-computes-it.md
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Destination(String);

impl Destination {
    /// Label a destination. The caller knows what it names; this does not.
    // Reserved for #39, which is the first ticket with an audience to hand down. The tests
    // below exercise it, so this is allowed where it is genuinely dead and nowhere else.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn labelled(label: String) -> Self {
        Self(label)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// What carries the audio.
///
/// **Every method answers nothing.** A sink cannot refuse ([ADR-0062]), so there is no
/// `Result` here to check and no future here to await — an operation is an instruction, it
/// is taken, and whether it worked shows up on [`Reports`] or not at all. That shape is the
/// whole reason a recorder can stand behind this interface without simulating anything.
///
/// It is private, because module privacy is the seam enforcement ([ADR-0061]) and a `Router`
/// or a `WebRtcTransport` escaping through a widened signature is how this seam would quietly
/// stop existing.
trait Carriage: Send + Sync {
    /// Give this session a media path of its own.
    ///
    /// **The path is bound to the session at creation** (ADR-0026) rather than handed out
    /// and claimed afterwards, so there is no window in which a transport exists that
    /// nobody owns and nothing to present in order to take one over.
    fn open_a_path_for(&self, session: &SessionId);

    /// Take this session's media path away, and everything carried on it with it.
    fn close_the_path_of(&self, session: &SessionId);

    /// Make exactly this audience hear this talker.
    ///
    /// It is the whole audience each time rather than a difference, because a difference
    /// would make the media plane hold an opinion about what it was told last, and the one
    /// thing this module must not do is have a view about who hears whom.
    ///
    /// **Nothing hands one down until #39 and #41**, and the signature is here anyway
    /// because it is the decision ([ADR-0063]): getting it the other way round would be
    /// quietly fatal, and the shape is much harder to change once there are callers. What
    /// stops that being a promise nobody checks is `an_audience_crosses_as_an_answer`, which
    /// asserts that what the media plane is handed is what it records.
    ///
    /// [ADR-0063]: ../../docs/adr/0063-the-media-plane-executes-routing-it-never-computes-it.md
    #[cfg_attr(not(test), allow(dead_code))]
    fn these_should_hear(&self, talker: &SessionId, audience: &Audience);
}

/// The media plane, as everything above it sees it.
///
/// Cloned freely — it is a handle to one worker, not a worker.
#[derive(Clone)]
pub(crate) struct MediaPlane {
    carriage: Arc<dyn Carriage>,
}

/// What stops the media plane from coming up.
///
/// These are startup failures and they are not the same thing as an operation failing. A
/// deployment that cannot start a worker cannot carry audio at all, so it refuses to start
/// rather than serving a console that will never make a sound.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MediaPlaneError {
    #[error("the mediasoup worker could not be started: {detail}")]
    Worker { detail: String },

    #[error("nothing could listen for media on {address}:{port}: {detail}")]
    CouldNotListen {
        address: String,
        port: u16,
        detail: String,
    },

    #[error("the router could not be created: {detail}")]
    Router { detail: String },
}

impl MediaPlane {
    /// Start carrying audio: **one Worker, one Router, one `WebRtcServer` port**, no TURN
    /// ([ADR-0006]).
    ///
    /// One Router because **a loop is not a transport primitive** (v1 §13). A Transport
    /// belongs to one Router, so router-per-loop would give a user monitoring six loops six
    /// ICE and DTLS sessions — the connection explosion this design exists to avoid,
    /// arriving through a different door.
    ///
    /// One port because the firewall conversation is a real cost of deploying this, and it
    /// carries UDP primarily with ICE-TCP on the same number where UDP is blocked.
    ///
    /// [ADR-0006]: ../../docs/adr/0006-mediasoup-carries-the-audio.md
    pub(crate) async fn carrying(
        media: &Media,
    ) -> Result<(Self, Reports, Carriageway), MediaPlaneError> {
        let (carriage, reports, carriageway) = worker::start(media).await?;

        Ok((
            Self {
                carriage: Arc::new(carriage),
            },
            reports,
            carriageway,
        ))
    }

    /// See [`Carriage::open_a_path_for`].
    pub(crate) fn open_a_path_for(&self, session: &SessionId) {
        self.carriage.open_a_path_for(session);
    }

    /// See [`Carriage::close_the_path_of`].
    pub(crate) fn close_the_path_of(&self, session: &SessionId) {
        self.carriage.close_the_path_of(session);
    }

    /// See [`Carriage::these_should_hear`].
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn these_should_hear(&self, talker: &SessionId, audience: &Audience) {
        self.carriage.these_should_hear(talker, audience);
    }
}

/// Where a report goes. Held by the adapters, which is why it is here rather than in one.
type Reporting = UnboundedSender<Reported>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_plane::recorder::a_recording_media_plane;

    /// **The media plane executes routing and never computes it** ([ADR-0063]), and this is
    /// the shape of that: an audience arrives as an answer and is recorded exactly as it
    /// came. Nothing here narrows it, widens it or asks why it says what it says.
    ///
    /// Nothing hands one down until #39 and #41. It is asserted now because the signature
    /// **is** the decision — if the fan-out lived below this seam the recorder would have to
    /// reimplement VoxLoop's routing, and every test about who hears whom would be a test of
    /// the reimplementation, passing while the product was broken.
    ///
    /// [ADR-0063]: ../../docs/adr/0063-the-media-plane-executes-routing-it-never-computes-it.md
    #[test]
    fn an_audience_crosses_as_an_answer_and_is_recorded_as_one() {
        let (media, _reports, recording) = a_recording_media_plane();
        let talker = SessionId::presented("alice".to_owned());
        // Alice is armed on two loops and Bob monitors one of them. Working that out is the
        // state authority's; this is the answer, on its way down.
        let audience = Audience {
            hearing: vec![Hearing {
                listener: SessionId::presented("bob".to_owned()),
                destination: Destination::labelled("flight".to_owned()),
            }],
        };

        media.these_should_hear(&talker, &audience);

        assert_eq!(
            recording.instructions(),
            vec![Instructed::TheseShouldHear {
                talker,
                audience: audience.clone()
            }]
        );
    }

    /// A destination is **a label and nothing else**. It carries the string it was given
    /// back out unchanged, and there is nothing on it to ask what the label names — which is
    /// the whole of why a `LoopId` does not cross this seam.
    #[test]
    fn a_destination_is_a_label_and_nothing_else() {
        assert_eq!(
            Destination::labelled("flight".to_owned()).as_str(),
            "flight"
        );
    }
}

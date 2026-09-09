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
    /// Audio is genuinely arriving from these sessions, as the `AudioLevelObserver` hears it.
    ///
    /// **This is the corroboration [ADR-0008] requires, and it is why the observer runs in
    /// v1 rather than as optional instrumentation.** Keying is the client's act and the
    /// server takes it on trust, which leaves one residual: a defective or hostile client can
    /// keep sending while claiming to be unkeyed. Nothing here knows what anybody claims —
    /// this says only that a voice is on the wire, and something above both seams compares
    /// the two.
    ///
    /// [ADR-0008]: ../../docs/adr/0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md
    TheseAreAudible { talkers: Vec<SessionId> },
    /// Nothing above the threshold is arriving from anybody.
    ///
    /// It is **not** silence on a loop and must never be read as one: DTX means a quiet
    /// talker sends no packets at all ([ADR-0010]), so this is the observer saying it has
    /// nothing to report rather than the deployment saying nobody is speaking.
    ///
    /// [ADR-0010]: ../../docs/adr/0010-opus-mono-and-the-latency-budget.md
    NobodyIsAudible,
}

/// Whatever the client's own media library has to be handed, carried and never read.
///
/// **VoxLoop owns the signalling** ([ADR-0006]) and this is the part of it that is not
/// VoxLoop's to have an opinion about: ICE candidates, DTLS fingerprints and RTP parameters
/// are a conversation between the worker and the library in the browser, and the server's
/// job is to carry it over the one authorised channel rather than to interpret it.
///
/// It is opaque **so that the rule holds that nothing mediasoup names leaves this module**
/// ([ADR-0061]). A `DtlsParameters` crossing the seam would put a mediasoup type in
/// Transport's signature and the seam would have quietly stopped existing; a value nobody
/// above can read cannot do that. Everything VoxLoop *decides* — who may arm, who hears whom
/// — is in domain types beside this and never in here.
///
/// [ADR-0006]: ../../docs/adr/0006-mediasoup-carries-the-audio.md
/// [ADR-0061]: ../../docs/adr/0061-module-privacy-is-the-seam-enforcement.md
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub(crate) struct Negotiation(serde_json::Value);

impl Negotiation {
    /// Take what a client said. Nothing reads it on the way past.
    pub(crate) fn presented(said: serde_json::Value) -> Self {
        Self(said)
    }
}

/// The name the media plane gave something it is carrying.
///
/// A string because it is only ever quoted back: the client presents one to say *this
/// carriage*, and nothing outside this module parses it or may infer anything from it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub(crate) struct Carried(String);

impl Carried {
    /// Take a name a client quoted back.
    pub(crate) fn presented(said: String) -> Self {
        Self(said)
    }
}

/// Which of a session's two ends a client is talking about.
///
/// **Two transports and not one**, because a browser's media library builds a directional
/// one at each end. It is still one ICE and DTLS conversation per direction rather than per
/// loop, which is the thing [ADR-0007] rules out — a loop is not a transport primitive, and
/// these are named for the two layers that ADR does name.
///
/// [ADR-0007]: ../../docs/adr/0007-the-client-emits-one-stream.md
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Way {
    /// The uplink: one stream, whatever the talker is armed on. It transmits; it does not
    /// address.
    Up,
    /// The downlink: one stream per audible talker, mixed in the client.
    Down,
}

impl Way {
    /// The word this direction goes by, on the wire and in a log line alike.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

/// What the media plane has to say to **one** session, rather than about the deployment.
///
/// It is a second channel beside [`Reports`] rather than a variant of it, and the difference
/// is who the audience is. A report is a fact about the running system that something above
/// decides the meaning of; this is signalling addressed to one client, and the socket that
/// holds that session is the only thing that can carry it. Putting it on [`Reports`] would
/// mean supervision routing a payload it has no business reading to a socket it does not
/// know about.
///
/// **It is still a sink.** The channel is handed *in* at [`Carriage::open_a_path_for`], so
/// nothing here calls anybody and no operation answers anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Negotiated {
    /// What this session needs to build its own end of the media path.
    APathToBuild(Negotiation),
    /// The uplink is carried, under this name.
    ///
    /// It is what a client's own library waits for before it will call a microphone
    /// published: the stream exists on this server and has a name here.
    TheUplinkIsCarried(Carried),
    /// One more talker to hear, and what to build in order to hear them.
    ///
    /// **One per audible talker and never per (talker, loop)** ([ADR-0007]): a listener
    /// monitoring two of a talker's destinations is one entry here, or they would hear the
    /// same voice twice.
    ///
    /// [ADR-0007]: ../../docs/adr/0007-the-client-emits-one-stream.md
    OneMoreTalker(Negotiation),
    /// One fewer. This carriage is closed at the server's end and the client should let it go.
    OneFewerTalker(Carried),
}

/// Where the media plane says things to one session. Handed in, so nothing here calls out.
pub(crate) type Telling = UnboundedSender<Negotiated>;

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
    pub(crate) fn labelled(label: String) -> Self {
        Self(label)
    }

    /// The label, back out unchanged.
    ///
    /// **Reserved for the recording tap**, which is addressed per (talker, destination loop)
    /// ([ADR-0009]) and is the only thing in the design that reads one of these. v1 ships no
    /// sink for it, so nothing in a running deployment calls this and the tests below are
    /// what keep the promise honest — a label that could not be read back would be a label
    /// that had quietly become an identifier.
    ///
    /// [ADR-0009]: ../../docs/adr/0009-recording-taps-plain-rtp-on-loopback.md
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
    /// Give this session a media path of its own, and somewhere to say things to it.
    ///
    /// **The path is bound to the session at creation** (ADR-0026) rather than handed out
    /// and claimed afterwards, so there is no window in which a transport exists that
    /// nobody owns and nothing to present in order to take one over. The same is true of
    /// `telling`: the channel arrives with the session that owns it, so there is no way to
    /// ask for somebody else's signalling.
    fn open_a_path_for(&self, session: &SessionId, telling: Telling);

    /// What this session's own end can decode.
    ///
    /// Nothing is carried to a client before this arrives, because a stream it cannot decode
    /// is worse than no stream: it is one the console would show as heard.
    fn the_client_will_hear(&self, session: &SessionId, what_it_can_decode: Negotiation);

    /// The client's keys for one end of its path, on its way to the worker.
    fn the_client_connects(&self, session: &SessionId, way: Way, keys: Negotiation);

    /// The client is sending, and this is what it is sending.
    ///
    /// **One uplink, whatever the talker is armed on** ([ADR-0007]). It exists from the
    /// moment the microphone does and it does not come and go with the key: keying is the
    /// client muting its own track, precisely so that a key press costs no renegotiation.
    ///
    /// [ADR-0007]: ../../docs/adr/0007-the-client-emits-one-stream.md
    fn the_client_speaks(&self, session: &SessionId, what_it_is_sending: Negotiation);

    /// The client has built its end of one carriage and is ready to be sent audio on it.
    ///
    /// A carriage is built paused and resumed here rather than started running, so that
    /// nothing is sent to an end that does not exist yet.
    fn the_client_hears(&self, session: &SessionId, carriage: &Carried);

    /// Take this session's media path away, and everything carried on it with it.
    fn close_the_path_of(&self, session: &SessionId);

    /// Make exactly this audience hear this talker.
    ///
    /// It is the whole audience each time rather than a difference, because a difference
    /// would make the media plane hold an opinion about what it was told last, and the one
    /// thing this module must not do is have a view about who hears whom.
    ///
    /// **A listener may appear more than once**, with a different destination each time, and
    /// this module collapses that into one carriage: the downlink is one stream per audible
    /// talker ([ADR-0007]) and delivering the pair as two would hand somebody the same voice
    /// twice. What the pairs are kept for is the recording tap, which is per (talker,
    /// destination loop) ([ADR-0009]).
    ///
    /// [ADR-0063]: ../../docs/adr/0063-the-media-plane-executes-routing-it-never-computes-it.md
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
    pub(crate) fn open_a_path_for(&self, session: &SessionId, telling: Telling) {
        self.carriage.open_a_path_for(session, telling);
    }

    /// See [`Carriage::close_the_path_of`].
    pub(crate) fn close_the_path_of(&self, session: &SessionId) {
        self.carriage.close_the_path_of(session);
    }

    /// See [`Carriage::the_client_will_hear`].
    pub(crate) fn the_client_will_hear(
        &self,
        session: &SessionId,
        what_it_can_decode: Negotiation,
    ) {
        self.carriage
            .the_client_will_hear(session, what_it_can_decode);
    }

    /// See [`Carriage::the_client_connects`].
    pub(crate) fn the_client_connects(&self, session: &SessionId, way: Way, keys: Negotiation) {
        self.carriage.the_client_connects(session, way, keys);
    }

    /// See [`Carriage::the_client_speaks`].
    pub(crate) fn the_client_speaks(&self, session: &SessionId, what_it_is_sending: Negotiation) {
        self.carriage.the_client_speaks(session, what_it_is_sending);
    }

    /// See [`Carriage::the_client_hears`].
    pub(crate) fn the_client_hears(&self, session: &SessionId, carriage: &Carried) {
        self.carriage.the_client_hears(session, carriage);
    }

    /// See [`Carriage::these_should_hear`].
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

//! The real media plane: mediasoup, supervised, with a transport per session.
//!
//! **One Worker, one Router, one `WebRtcServer` port, and no TURN** ([ADR-0006]). One Router
//! because a loop is not a transport primitive: a Transport belongs to one Router, so a
//! router per loop would give a user monitoring six loops six ICE and DTLS sessions. One port
//! because the firewall conversation is a real cost of deploying this — UDP primarily, with
//! ICE-TCP on the same number where UDP is blocked.
//!
//! **Nothing mediasoup names is `pub`, not even to the parent module.** `IceState`,
//! `WebRtcTransport` and `Router` all stop at this file, and translating them into the
//! ladder's vocabulary is precisely this file's job ([ADR-0060]).
//!
//! ## The bridge into tokio is channels
//!
//! mediasoup's callbacks fire on **mediasoup's** threads (v1 §13). So every callback
//! registered here does exactly one thing — put a value on an unbounded channel — which
//! needs no runtime, blocks on nothing and cannot deadlock against the state authority's
//! lock. Nothing here borrows a tokio handle and nothing here blocks.
//!
//! Instructions travel the same way in the other direction: [`Carriage`]'s methods answer
//! nothing and must not await, so they queue and return, and one task owns every mediasoup
//! object there is. That task is the only place a transport is created, held or dropped,
//! which is why the map below needs no lock.
//!
//! ## The worker is a thread, not a child process
//!
//! [ADR-0070]. The Rust API links the worker in and runs it on a thread of this process, so
//! supervision is observing [`Worker::on_dead`] rather than watching a pid. What is
//! supervised is unchanged; where it lives is not.
//!
//! [ADR-0006]: ../../docs/adr/0006-mediasoup-carries-the-audio.md
//! [ADR-0060]: ../../docs/adr/0060-a-seam-names-domain-operations.md
//! [ADR-0070]: ../../docs/adr/0070-the-mediasoup-worker-is-a-thread-of-this-process.md

use std::collections::HashMap;
use std::num::{NonZeroU8, NonZeroU32};
use std::sync::{Arc, Mutex};

use mediasoup::router::{Router, RouterOptions};
use mediasoup::types::data_structures::{DtlsState, IceState, ListenInfo, Protocol};
use mediasoup::types::rtp_parameters::{
    MimeTypeAudio, RtpCodecCapability, RtpCodecParametersParameters,
};
use mediasoup::webrtc_server::{WebRtcServer, WebRtcServerListenInfos, WebRtcServerOptions};
use mediasoup::webrtc_transport::{WebRtcTransport, WebRtcTransportOptions};
use mediasoup::worker::{Worker, WorkerLogLevel, WorkerSettings};
use mediasoup::worker_manager::WorkerManager;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use super::{Audience, Carriage, MediaPlaneError, Reported, Reporting, Reports};
use crate::state::{MediaPath, SessionId};
use crate::telemetry::module;

/// Opus, 48 kHz, 20 ms frames, inband FEC and DTX both on ([ADR-0010]).
///
/// `channels: 2` is the codec's declared capability rather than a decision to carry stereo —
/// `audio/opus` is `opus/48000/2` on the wire whatever the encoder does, and mono is an
/// `stereo=0` matter between the endpoints. ADR-0010's mono still holds: panning is a
/// presentation choice the console makes over mono sources.
///
/// **DTX is the one to be careful about.** With no packets during silence a client cannot
/// tell a quiet loop from a loop it is deaf to, which is the silent failure this system can
/// least afford — [ADR-0017]'s beacon is what closes it, and nothing here may be read as a
/// substitute.
///
/// [ADR-0010]: ../../docs/adr/0010-opus-mono-and-the-latency-budget.md
/// [ADR-0017]: ../../docs/adr/0017-loop-health-is-measured-not-asserted.md
fn what_the_router_carries() -> Vec<RtpCodecCapability> {
    let mut parameters = RtpCodecParametersParameters::default();
    parameters.insert("useinbandfec", 1_u32);
    parameters.insert("usedtx", 1_u32);

    vec![RtpCodecCapability::Audio {
        mime_type: MimeTypeAudio::Opus,
        preferred_payload_type: None,
        clock_rate: NonZeroU32::new(48_000).expect("48000 is not zero"),
        channels: NonZeroU8::new(2).expect("2 is not zero"),
        parameters,
        rtcp_feedback: Vec::new(),
    }]
}

/// One instruction, on its way to the task that owns every mediasoup object.
enum Instruction {
    Open(SessionId),
    Close(SessionId),
    Hear {
        talker: SessionId,
        audience: Audience,
    },
}

/// The real carriage: a handle to the task, and nothing else.
///
/// It holds no mediasoup object on purpose. Everything mediasoup owns lives in one task, so
/// there is exactly one thread that may touch a transport and no lock to get wrong.
pub(super) struct Carrying {
    instructions: UnboundedSender<Instruction>,
}

/// The running media plane, and the handle that stops it.
pub(crate) struct Carriageway {
    task: JoinHandle<()>,
}

impl Carriageway {
    /// Stop carrying audio.
    ///
    /// Aborting is enough and is the honest thing: an instruction half-done is a transport
    /// that either exists or does not, the process is on its way out, and every session ends
    /// with a restart anyway (ADR-0039). Dropping the task drops the Router and the Worker
    /// with it, which is what closes the port.
    pub(crate) fn stop(self) {
        tracing::info!(target: module::MEDIA_PLANE, "stopping");
        self.task.abort();
    }
}

/// Bring up the worker, the router and the one port, and start taking instructions.
pub(super) async fn start(
    media: &crate::configuration::Media,
) -> Result<(Carrying, Reports, Carriageway), MediaPlaneError> {
    let (reporting, reports) = tokio::sync::mpsc::unbounded_channel();

    let manager = WorkerManager::new();
    let worker = manager
        .create_worker({
            let mut settings = WorkerSettings::default();
            // The worker's own logs go through `tracing` like everything else, at the level
            // the deployment file set for this module. Anything louder than a warning from a
            // C++ event loop is noise until somebody is deliberately looking at it.
            settings.log_level = WorkerLogLevel::Warn;
            settings
        })
        .await
        .map_err(|error| MediaPlaneError::Worker {
            detail: error.to_string(),
        })?;

    // **Supervision, such as it is** (ADR-0070). The worker is a thread of this process, so
    // there is no pid to watch and no exit status to reap — what there is, is this callback,
    // and it fires once. Everything the deployment was carrying is gone by the time it does.
    worker
        .on_dead({
            let reporting = reporting.clone();
            move |outcome| {
                let detail = match outcome {
                    Ok(()) => "the worker stopped".to_owned(),
                    Err(error) => error.to_string(),
                };
                let _ = reporting.send(Reported::NothingIsCarried { detail });
            }
        })
        .detach();

    // One shared port, both protocols, same number. UDP is listed first because it is the
    // preferred one and mediasoup takes these in order of preference; TCP is the fallback
    // for a VPN that blocks UDP outright, at a latency cost this deployment would rather
    // discover than pre-pay for with a second daemon (ADR-0006).
    let port = a_port_or_whatever_is_free(media.port);
    let listening = WebRtcServerListenInfos::new(one_way_in(media, Protocol::Udp, port))
        .insert(one_way_in(media, Protocol::Tcp, port));

    let server = worker
        .create_webrtc_server(WebRtcServerOptions::new(listening))
        .await
        .map_err(|error| MediaPlaneError::CouldNotListen {
            address: media.listen_address.to_string(),
            port: media.port,
            detail: error.to_string(),
        })?;

    let router = worker
        .create_router(RouterOptions::new(what_the_router_carries()))
        .await
        .map_err(|error| MediaPlaneError::Router {
            detail: error.to_string(),
        })?;

    tracing::info!(
        target: module::MEDIA_PLANE,
        announced = %media.announced_address,
        port = media.port,
        "carrying"
    );

    let (instructions, taking) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(carry(taking, manager, worker, router, server, reporting));

    Ok((Carrying { instructions }, reports, Carriageway { task }))
}

/// One way in, on one protocol.
fn one_way_in(
    media: &crate::configuration::Media,
    protocol: Protocol,
    port: Option<u16>,
) -> ListenInfo {
    ListenInfo {
        protocol,
        ip: media.listen_address,
        // What goes into every ICE candidate. It is the address a client dials rather than
        // the one the worker binds, which is the whole reason it has no default.
        announced_address: Some(media.announced_address.clone()),
        expose_internal_ip: false,
        port,
        port_range: None,
        flags: None,
        send_buffer_size: None,
        recv_buffer_size: None,
    }
}

/// A port, or none if the deployment asked for whatever is free.
///
/// Port `0` reads the same way it does under `[listen]`: *the operating system picks*. It is
/// what the tests want and nothing a real deployment should ever set, because a firewall rule
/// cannot name an ephemeral port.
fn a_port_or_whatever_is_free(port: u16) -> Option<u16> {
    (port != 0).then_some(port)
}

/// Everything mediasoup owns, in one place, taking instructions until there are none left.
///
/// The `WorkerManager` is held for its whole life rather than dropped after the worker is
/// made: it owns the executor thread every mediasoup future runs on, and dropping it stops
/// that thread.
async fn carry(
    mut taking: UnboundedReceiver<Instruction>,
    manager: WorkerManager,
    worker: Worker,
    router: Router,
    server: WebRtcServer,
    reporting: Reporting,
) {
    let _held_open = (manager, worker);
    let mut paths: HashMap<SessionId, Path> = HashMap::new();

    while let Some(instruction) = taking.recv().await {
        match instruction {
            Instruction::Open(session) => {
                match open(&router, &server, &session, &reporting).await {
                    Some(path) => {
                        // Re-assuming mints a new session id, so this replaces nothing in
                        // practice; where it does, the old transport is dropped and closed,
                        // which is the right end for a path nothing can name any more.
                        paths.insert(session, path);
                    }
                    None => {
                        // The path could not be built. A sink cannot refuse, so the way this
                        // is said is the way every other media fault is said: on the channel,
                        // as a path that is lost.
                        let _ = reporting.send(Reported::ThePath {
                            of: session,
                            is: MediaPath::Lost,
                        });
                    }
                }
            }
            Instruction::Close(session) => {
                // Dropping the transport closes it, and everything carried on it goes with
                // it. There is nothing to await and nothing that can fail.
                paths.remove(&session);
            }
            Instruction::Hear { talker, audience } => {
                // #39 and #41 are the first tickets with an audience to hand down. Until
                // then this is reachable only from a test, and what it must never grow is a
                // view about who ought to be in the list it was given (ADR-0063).
                tracing::debug!(
                    target: module::MEDIA_PLANE,
                    talker = talker.as_str(),
                    hearing = audience.hearing.len(),
                    "an audience arrived before there is anything to route"
                );
            }
        }
    }
}

/// One session's media path.
///
/// It is the transport and nothing else. The callbacks watching it are **detached** rather
/// than held: a `HandlerId` deregisters its callback when it drops, and the bag they are
/// registered in belongs to the transport itself — so detaching keeps them alive for exactly
/// as long as there is a transport to report about, and holding them here would only be a
/// second way to say the same thing.
struct Path {
    _transport: WebRtcTransport,
}

/// Build one session's transport and start watching it.
///
/// **Bound to the session at creation** (ADR-0026): the transport is created for a session
/// that already exists and is filed under it in the same breath, so there is no moment at
/// which one exists unclaimed and nothing to present in order to claim one.
async fn open(
    router: &Router,
    server: &WebRtcServer,
    session: &SessionId,
    reporting: &Reporting,
) -> Option<Path> {
    let transport = match router
        .create_webrtc_transport(WebRtcTransportOptions::new_with_server(server.clone()))
        .await
    {
        Ok(transport) => transport,
        Err(error) => {
            tracing::error!(
                target: module::MEDIA_PLANE,
                %error,
                session = session.as_str(),
                "a media path could not be built"
            );
            return None;
        }
    };

    // Both ends of one reading, shared by the two callbacks so that neither has to hold the
    // transport that owns it. ICE and DTLS move independently and either can be the one that
    // fails, so what is reported is the worse of the two — the same pessimism ADR-0042
    // applies between the client and the server, applied here between two halves of one end.
    let seen = Arc::new(Mutex::new(Seen {
        ice: transport.ice_state(),
        dtls: transport.dtls_state(),
        said: None,
    }));

    transport
        .on_ice_state_change({
            let seen = Arc::clone(&seen);
            let reporting = reporting.clone();
            let session = session.clone();
            move |ice| {
                say_if_it_moved(&seen, &reporting, &session, |seen| seen.ice = ice);
            }
        })
        .detach();
    transport
        .on_dtls_state_change({
            let seen = Arc::clone(&seen);
            let reporting = reporting.clone();
            let session = session.clone();
            move |dtls| {
                say_if_it_moved(&seen, &reporting, &session, |seen| seen.dtls = dtls);
            }
        })
        .detach();

    // What the server can see of a path nobody has connected to yet, said out loud rather
    // than left to be inferred: a transport exists and no audio can cross it.
    say_if_it_moved(&seen, reporting, session, |_| {});

    Some(Path {
        _transport: transport,
    })
}

/// What the server end has seen, and what it last said about it.
struct Seen {
    ice: IceState,
    dtls: DtlsState,
    /// The last reading put on the channel, so an ICE change that does not move the merged
    /// answer does not wake anything up. The ladder has three rungs and these two states have
    /// nine combinations between them.
    said: Option<MediaPath>,
}

/// Take a change, work out the reading, and report it if it is news.
///
/// **This is the whole of what runs on a mediasoup thread**: a lock held for the length of a
/// comparison, and a send on an unbounded channel. Nothing awaits, nothing blocks and nothing
/// reaches for a tokio handle (v1 §13).
fn say_if_it_moved(
    seen: &Mutex<Seen>,
    reporting: &Reporting,
    session: &SessionId,
    change: impl FnOnce(&mut Seen),
) {
    let mut seen = match seen.lock() {
        Ok(seen) => seen,
        Err(poisoned) => poisoned.into_inner(),
    };
    change(&mut seen);

    let now = the_worse_of(from_ice(seen.ice), from_dtls(seen.dtls));
    if seen.said == Some(now) {
        return;
    }
    seen.said = Some(now);

    let _ = reporting.send(Reported::ThePath {
        of: session.clone(),
        is: now,
    });
}

/// The server's reading of ICE, on the ladder's own terms ([ADR-0042]).
///
/// **`New` is `lost`, and that is not a fault report.** A transport nobody has connected to
/// carries no audio, so a session holding one has no emission path — which is exactly what
/// `lost` means and exactly what the transmit bar has to say. Calling it anything softer
/// would be the console showing an armed, keyable operator whose voice reaches nobody, which
/// is the misrepresentation this ladder exists to prevent.
///
/// There is deliberately nothing here that maps to `impaired`. mediasoup has no `failed` and
/// its `disconnected` is driven by ICE consent freshness, which takes around thirty seconds —
/// longer than the whole signalling ladder — so a server-side `disconnected` is old news by
/// the time it arrives and is taken at its worst. Telling a transient fault from a terminal
/// one is the client's job, and the client is much better at it.
///
/// [ADR-0042]: ../../docs/adr/0042-the-media-path-has-its-own-ladder.md
fn from_ice(ice: IceState) -> MediaPath {
    match ice {
        IceState::Connected | IceState::Completed => MediaPath::Connected,
        IceState::New | IceState::Disconnected => MediaPath::Lost,
    }
}

/// The server's reading of DTLS, on the same terms.
///
/// ADR-0042 names both callbacks as the backstop, and the two say different things: ICE says
/// whether packets are getting through and DTLS says whether they can be decrypted. A
/// transport with ICE connected and DTLS failed carries nothing at all.
fn from_dtls(dtls: DtlsState) -> MediaPath {
    match dtls {
        DtlsState::Connected => MediaPath::Connected,
        DtlsState::New | DtlsState::Connecting | DtlsState::Failed | DtlsState::Closed => {
            MediaPath::Lost
        }
    }
}

/// The worse of two readings. Red needs only one (ADR-0042).
fn the_worse_of(one: MediaPath, other: MediaPath) -> MediaPath {
    one.max(other)
}

impl Carriage for Carrying {
    fn open_a_path_for(&self, session: &SessionId) {
        self.tell(Instruction::Open(session.clone()));
    }

    fn close_the_path_of(&self, session: &SessionId) {
        self.tell(Instruction::Close(session.clone()));
    }

    fn these_should_hear(&self, talker: &SessionId, audience: &Audience) {
        self.tell(Instruction::Hear {
            talker: talker.clone(),
            audience: audience.clone(),
        });
    }
}

impl Carrying {
    /// Queue one instruction and return.
    ///
    /// A send that fails means the task is gone, which means the worker is gone, which is
    /// already on the reports channel as the thing it is. There is nothing to tell the
    /// caller, because a sink cannot refuse.
    fn tell(&self, instruction: Instruction) {
        if self.instructions.send(instruction).is_err() {
            tracing::error!(
                target: module::MEDIA_PLANE,
                "an instruction arrived after the media plane stopped"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The one test in VoxLoop that runs a mediasoup worker.**
    ///
    /// Everything else stands behind the recorder ([ADR-0064]), which is right: a C++ worker
    /// negotiating ICE and DTLS is slow, external and nondeterministic, and a suite that
    /// started one per test would be testing whether they collided for the port. But a seam
    /// with nothing real behind it is a reserved space rather than a proven boundary, and
    /// what this asserts is exactly the part the recorder cannot: that a Worker, a Router and
    /// a `WebRtcServer` come up, that a session gets a transport of its own, and that
    /// mediasoup's callbacks reach a tokio task over the channel rather than by any other
    /// route.
    ///
    /// The port is whatever is free, because a fixed one would make this test a question
    /// about the machine it runs on.
    #[tokio::test]
    async fn a_worker_a_router_and_one_port_come_up_and_a_session_gets_a_path_of_its_own() {
        let media = crate::configuration::Media::on_loopback();

        let (carrying, mut reports, carriageway) =
            start(&media).await.expect("the media plane to come up");

        let session = SessionId::presented("a-session".to_owned());
        carrying.open_a_path_for(&session);

        // The transport exists and nobody has connected to it, so the server's own end of
        // the ladder is `lost` — and it arrives here, on the channel, from a callback that
        // fired on one of mediasoup's threads.
        let said = tokio::time::timeout(std::time::Duration::from_secs(10), reports.recv())
            .await
            .expect("the media plane to report within ten seconds")
            .expect("a report");

        assert_eq!(
            said,
            Reported::ThePath {
                of: session.clone(),
                is: MediaPath::Lost
            }
        );

        // And it goes when the session does, with nothing to await and nothing to check.
        carrying.close_the_path_of(&session);
        carriageway.stop();
    }

    /// The two halves of the server's own reading, and the rule that a transport nobody has
    /// connected to carries no audio however far along either half is.
    #[test]
    fn the_server_reads_ice_and_dtls_together_and_takes_the_worse() {
        assert_eq!(
            the_worse_of(
                from_ice(IceState::Completed),
                from_dtls(DtlsState::Connected)
            ),
            MediaPath::Connected
        );
        // ICE through, DTLS not: packets arrive and nothing can decrypt them.
        assert_eq!(
            the_worse_of(
                from_ice(IceState::Connected),
                from_dtls(DtlsState::Connecting)
            ),
            MediaPath::Lost
        );
        // A transport nobody has connected to yet is the same answer as one that has failed,
        // because the operator can do the same thing about both: nothing, yet.
        assert_eq!(
            the_worse_of(from_ice(IceState::New), from_dtls(DtlsState::New)),
            MediaPath::Lost
        );
        // mediasoup has no `failed`, and its `disconnected` is thirty seconds of ICE consent
        // freshness — old news by the time it lands, so it is taken at its worst.
        assert_eq!(from_ice(IceState::Disconnected), MediaPath::Lost);
    }

    /// A deployment asking for a port is given that port; one asking for nothing in
    /// particular is given whatever is free, which is what a test wants and what a firewall
    /// rule can never name.
    #[test]
    fn a_port_of_zero_means_whatever_is_free() {
        assert_eq!(a_port_or_whatever_is_free(44444), Some(44444));
        assert_eq!(a_port_or_whatever_is_free(0), None);
    }
}

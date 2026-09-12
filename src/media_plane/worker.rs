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
use std::num::{NonZeroU8, NonZeroU16, NonZeroU32};
use std::sync::{Arc, Mutex};

use mediasoup::audio_level_observer::{AudioLevelObserver, AudioLevelObserverOptions};
use mediasoup::consumer::{Consumer, ConsumerOptions};
use mediasoup::direct_transport::{DirectTransport, DirectTransportOptions};
use mediasoup::producer::{Producer, ProducerId, ProducerOptions};
use mediasoup::router::{Router, RouterOptions};
use mediasoup::rtp_observer::{RtpObserver, RtpObserverAddProducerOptions};
use mediasoup::transport::Transport;
use mediasoup::types::data_structures::{DtlsState, IceState, ListenInfo, Protocol};
use mediasoup::types::rtp_parameters::{
    MediaKind, MimeTypeAudio, RtpCapabilities, RtpCodecCapability, RtpCodecParameters,
    RtpCodecParametersParameters, RtpEncodingParameters, RtpParameters,
};
use mediasoup::webrtc_server::{WebRtcServer, WebRtcServerListenInfos, WebRtcServerOptions};
use mediasoup::webrtc_transport::{
    WebRtcTransport, WebRtcTransportOptions, WebRtcTransportRemoteParameters,
};
use mediasoup::worker::{Worker, WorkerLogLevel, WorkerSettings};
use mediasoup::worker_manager::WorkerManager;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use super::{
    Audience, Carriage, Carried, Destination, MediaPlaneError, Negotiated, Negotiation, Reported,
    Reporting, Reports, Telling, Way,
};
use crate::state::{MediaPath, SessionId, THE_BEACON_SOUNDS_EVERY};
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
    Open {
        session: SessionId,
        telling: Telling,
    },
    Close(SessionId),
    WillHear {
        session: SessionId,
        what_it_can_decode: Negotiation,
    },
    Connect {
        session: SessionId,
        way: Way,
        keys: Negotiation,
    },
    Speaks {
        session: SessionId,
        what_it_is_sending: Negotiation,
    },
    Hears {
        session: SessionId,
        carriage: Carried,
    },
    Hear {
        talker: SessionId,
        audience: Audience,
    },
    Beacons(Vec<Destination>),
    Count {
        listener: SessionId,
        on: Vec<Destination>,
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

    // **The observer runs in v1** (ADR-0008), so it is created here beside the router rather
    // than behind a flag: it is what makes the residual of client-side keying detectable, and
    // a deployment that could be started without it would be one where that residual is
    // simply unwatched.
    let speaking = Arc::new(Mutex::new(HashMap::new()));
    let observer = watching_who_is_audible(&router, &speaking, &reporting).await?;

    // **Where every loop's beacon is produced from** (ADR-0017). A direct transport is one the
    // Rust side sends on itself, so the beacon originates on this server and crosses the same
    // router and the same per-loop fan-out that speech does — which is what makes its arrival
    // a measurement of the loop rather than of a side channel. It is one transport for every
    // beacon because a beacon is a producer, not a path.
    let beacons_on = router
        .create_direct_transport(DirectTransportOptions::default())
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
    let task = tokio::spawn(carry(
        taking,
        Owned {
            _held_open: (manager, worker),
            router,
            server,
            observer,
            beacons_on,
        },
        Speaking(speaking),
        reporting,
    ));

    Ok((Carrying { instructions }, reports, Carriageway { task }))
}

/// How loud a talker has to be before the observer will say anything.
///
/// **This is a corroboration threshold rather than a level meter** (ADR-0008): what it is
/// tuned for is *somebody is speaking into a live microphone*, not *how loudly*. Under it are
/// a muted track's comfort noise and a quiet room; over it is a voice. Nothing renders it and
/// nothing may — a level a console drew would be exactly the amplitude ADR-0033 forbids.
const A_VOICE_IS_THIS_LOUD: i8 = -50;

/// How often the observer looks, in milliseconds.
///
/// Twice a second. It answers *is this client sending while claiming not to be*, which is a
/// question about a fault or an abuse rather than about a transmission, and a faster reading
/// would cost the worker work for an answer nobody acts on within seconds anyway.
const THE_OBSERVER_LOOKS_EVERY: u16 = 500;

/// How many talkers the observer will name at once.
///
/// Deliberately larger than the pilot's shape, because the answer this is put to has to be
/// **complete**: a cap that quietly dropped the one client sending while claiming to be
/// unkeyed would make the corroboration worse than useless — it would be a check that passes.
const EVERY_TALKER_THE_PILOT_HAS: u16 = 64;

/// Start the observer, and turn what it hears into sessions.
///
/// **The attribution happens here**, in the module that is entitled to know a producer from a
/// session, so what leaves is a list of sessions. It fires on a mediasoup thread like every
/// other callback here, so it does exactly what those do: take a lock for the length of a
/// lookup, and send.
async fn watching_who_is_audible(
    router: &Router,
    speaking: &Arc<Mutex<HashMap<ProducerId, SessionId>>>,
    reporting: &Reporting,
) -> Result<AudioLevelObserver, MediaPlaneError> {
    let observer = router
        .create_audio_level_observer({
            let mut options = AudioLevelObserverOptions::default();
            options.max_entries =
                NonZeroU16::new(EVERY_TALKER_THE_PILOT_HAS).expect("64 is not zero");
            options.threshold = A_VOICE_IS_THIS_LOUD;
            options.interval = THE_OBSERVER_LOOKS_EVERY;
            options
        })
        .await
        .map_err(|error| MediaPlaneError::Router {
            detail: error.to_string(),
        })?;

    observer
        .on_volumes({
            let speaking = Arc::clone(speaking);
            let reporting = reporting.clone();
            move |volumes| {
                let known = match speaking.lock() {
                    Ok(known) => known,
                    Err(poisoned) => poisoned.into_inner(),
                };
                let talkers = volumes
                    .iter()
                    .filter_map(|volume| known.get(&volume.producer.id()).cloned())
                    .collect();

                let _ = reporting.send(Reported::TheseAreAudible { talkers });
            }
        })
        .detach();

    observer
        .on_silence({
            let reporting = reporting.clone();
            move || {
                let _ = reporting.send(Reported::NobodyIsAudible);
            }
        })
        .detach();

    Ok(observer)
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

/// The mediasoup objects the deployment has exactly one of, held for the task's whole life.
///
/// The `WorkerManager` is in here rather than dropped after the worker is made: it owns the
/// executor thread every mediasoup future runs on, and dropping it stops that thread.
struct Owned {
    /// Held rather than read. The manager owns the executor thread every mediasoup future
    /// runs on, and the worker owns everything else here; dropping either stops the lot.
    _held_open: (WorkerManager, Worker),
    router: Router,
    server: WebRtcServer,
    observer: AudioLevelObserver,
    /// What every loop's beacon is produced on. Nothing on it is ever added to `observer`.
    beacons_on: DirectTransport,
}

/// Which session each uplink belongs to, shared with the observer's callback.
///
/// It is behind a lock because it is the one structure two threads read: this task writes it
/// when a client starts or stops sending, and the observer's callback — which fires on a
/// mediasoup thread — reads it to turn a producer into a session.
struct Speaking(Arc<Mutex<HashMap<ProducerId, SessionId>>>);

impl Speaking {
    fn now(&self, producer: ProducerId, session: SessionId) {
        self.held().insert(producer, session);
    }

    fn no_longer(&self, session: &SessionId) {
        self.held().retain(|_, whose| whose != session);
    }

    fn held(&self) -> std::sync::MutexGuard<'_, HashMap<ProducerId, SessionId>> {
        match self.0.lock() {
            Ok(held) => held,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// One session's media path: two transports, what it is sending, and what it is hearing.
///
/// **Two transports rather than one**, because a browser's media library builds a directional
/// transport at each end. That is still one ICE and DTLS conversation per direction rather
/// than one per loop, which is the thing ADR-0007 rules out.
///
/// The callbacks watching the transports are **detached** rather than held: a `HandlerId`
/// deregisters its callback when it drops, and the bag they are registered in belongs to the
/// transport itself — so detaching keeps them alive for exactly as long as there is a
/// transport to report about.
struct Path {
    up: WebRtcTransport,
    down: WebRtcTransport,
    /// Where this session's own signalling goes. Handed in with the session that owns it.
    telling: Telling,
    /// What this client's end can decode, once it has said. Nothing is carried to it before.
    can_decode: Option<RtpCapabilities>,
    /// This session's uplink, once its microphone is publishing. **One, whatever it is armed
    /// on** (ADR-0007), and it outlives every key press.
    producer: Option<Producer>,
    /// One carriage per audible talker, which is what makes the downlink per talker rather
    /// than per (talker, loop).
    hearing: HashMap<SessionId, Heard>,
    /// One carriage per loop whose beacon this session counts, and never per (loop, talker).
    counting: HashMap<Destination, Consumer>,
}

/// One carriage, and the destinations its client was last told it is heard on.
///
/// The destinations are kept so that they are said again only when they move: the client
/// plays the carriage at the loudest volume among them (v1 §5), and a talker arming one more
/// of this listener's loops changes that without changing the stream.
struct Heard {
    carriage: Consumer,
    on: Vec<Destination>,
}

impl Path {
    /// Stop carrying one talker to this session, and say so.
    ///
    /// Both halves together, in one place, because they are one act said twice over: dropping
    /// the `Consumer` closes it at this end, and the client is told so that it is not left
    /// holding a carriage it will never be sent audio on again. It is the same act whether the
    /// audience withdrew somebody or the talker went, which is why it is not written out in
    /// each of them.
    fn stop_hearing(&mut self, talker: &SessionId) {
        if let Some(heard) = self.hearing.remove(talker) {
            let _ = self.telling.send(Negotiated::OneFewerTalker(Carried(
                heard.carriage.id().to_string(),
            )));
        }
    }

    /// Stop carrying one loop's beacon to this session, and say so — the beacon's half of
    /// [`Path::stop_hearing`], for the same reason.
    fn stop_counting(&mut self, on: &Destination) {
        if let Some(carriage) = self.counting.remove(on) {
            let _ = self.telling.send(Negotiated::OneFewerBeacon(Carried(
                carriage.id().to_string(),
            )));
        }
    }

    /// Tell the client a carriage it already has is now heard on these destinations, where
    /// that is not what it was last told.
    fn heard_on(&mut self, talker: &SessionId, on: &[Destination]) {
        let Some(heard) = self.hearing.get_mut(talker) else {
            return;
        };
        if heard.on == on {
            return;
        }

        heard.on = on.to_vec();
        let _ = self.telling.send(Negotiated::HeardOn {
            carriage: Carried(heard.carriage.id().to_string()),
            heard_on: on.to_vec(),
        });
    }
}

/// Everything the task holds that is not one of the deployment's singletons.
struct Paths {
    paths: HashMap<SessionId, Path>,
    /// The last audience handed down for each talker.
    ///
    /// **It is a memory of an instruction, never an opinion.** Nothing here narrows it,
    /// widens it or decides when it is wrong ([ADR-0063]) — it is replayed unchanged in the
    /// two places where an answer arrived before there was anything to execute it with: a
    /// listener that had not yet said what it can decode, and a talker that was not yet
    /// sending. Without it, a client that finished negotiating after the routing settled
    /// would wait for somebody else to click something.
    ///
    /// [ADR-0063]: ../../docs/adr/0063-the-media-plane-executes-routing-it-never-computes-it.md
    audiences: HashMap<SessionId, Audience>,
    /// Every loop's beacon, by the label the loop was handed down as.
    beacons: HashMap<Destination, Beacon>,
    /// The last set of beacons each listener was said to count — a memory of an instruction
    /// for exactly the reason `audiences` is one, replayed when a listener says what it can
    /// decode and when a beacon it was already meant to count comes into being.
    counting: HashMap<SessionId, Vec<Destination>>,
    /// The synchronisation source the next beacon is given. Every beacon is produced on the
    /// one direct transport, which tells its producers apart by this, so no two may share one.
    next_ssrc: u32,
}

/// One loop's beacon: a producer on the direct transport, and where its stream has got to.
///
/// **It is silent** ([ADR-0017]): every packet carries Opus's own silence frame. A faintly
/// audible beacon is a rendering choice over the same packets and v1 does not make it, and
/// nothing plays one anyway — the client counts a beacon and never mixes it.
///
/// [ADR-0017]: ../../docs/adr/0017-loop-health-is-measured-not-asserted.md
struct Beacon {
    producer: Producer,
    ssrc: u32,
    sequence: u16,
    timestamp: u32,
}

/// The payload type a beacon is produced under. It is the producer's own number on the direct
/// transport and is rewritten for each carriage, so it only has to agree with itself.
const THE_BEACON_PAYLOAD_TYPE: u8 = 100;

/// Opus's silence frame: one 20 ms CELT frame at full band, mono, decoding to nothing.
const OPUS_SILENCE: [u8; 3] = [0xf8, 0xff, 0xfe];

impl Beacon {
    /// Sound once: one silent packet, stamped on from the last.
    fn sound(&mut self) {
        let packet = a_silent_packet(self.sequence, self.timestamp, self.ssrc);
        self.sequence = self.sequence.wrapping_add(1);
        // The clock the packet says it was sampled on moves by as long as the beacon was
        // silent for, so a receiver reads one packet an interval rather than a burst.
        self.timestamp = self
            .timestamp
            .wrapping_add(48_000 * THE_BEACON_SOUNDS_EVERY.as_secs() as u32);

        if let Producer::Direct(direct) = &self.producer
            && let Err(error) = direct.send(packet)
        {
            tracing::warn!(
                target: module::MEDIA_PLANE,
                %error,
                "a beacon did not sound, so every loop reads as not received until it does"
            );
        }
    }
}

/// One RTP packet, version 2, no padding, no extension, no marker, carrying Opus silence.
fn a_silent_packet(sequence: u16, timestamp: u32, ssrc: u32) -> Vec<u8> {
    let mut packet = vec![0x80, THE_BEACON_PAYLOAD_TYPE];
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.extend_from_slice(&ssrc.to_be_bytes());
    packet.extend_from_slice(&OPUS_SILENCE);

    packet
}

/// Everything mediasoup owns, in one place, taking instructions until there are none left.
///
/// **One at a time, in the order they were given**, and that is the guarantee rather than an
/// oversight. Assuming elsewhere closes the displaced session's path and then opens the new
/// one, and a loop that ran opens concurrently could reorder those two — leaving a transport
/// for a session nothing can name any more, which is the exact hazard the close exists to
/// avoid. The cost is that a queued instruction waits behind a transport being built, which is
/// a local round trip to a worker on the next thread; if that round trip is not local and
/// quick then the worker is wedged, and an instruction it cannot process is no better than one
/// it has not been given.
async fn carry(
    mut taking: UnboundedReceiver<Instruction>,
    owned: Owned,
    speaking: Speaking,
    reporting: Reporting,
) {
    let mut held = Paths {
        paths: HashMap::new(),
        audiences: HashMap::new(),
        beacons: HashMap::new(),
        counting: HashMap::new(),
        next_ssrc: 1,
    };

    // **The beacons sound on this task's own clock**, between instructions rather than beside
    // them, because this task is the only thing that may touch a producer. A beacon waiting
    // behind a transport being built is waiting a local round trip, which the window it is
    // measured against was sized to absorb many times over.
    let mut sounding = tokio::time::interval(THE_BEACON_SOUNDS_EVERY);
    sounding.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        let instruction = tokio::select! {
            instruction = taking.recv() => match instruction {
                Some(instruction) => instruction,
                None => break,
            },
            _ = sounding.tick() => {
                for beacon in held.beacons.values_mut() {
                    beacon.sound();
                }
                continue;
            }
        };

        match instruction {
            Instruction::Open { session, telling } => {
                match open(
                    &owned.router,
                    &owned.server,
                    &session,
                    telling.clone(),
                    &reporting,
                )
                .await
                {
                    Some(path) => {
                        // Re-assuming mints a new session id, so this replaces nothing in
                        // practice; where it does, the old transports are dropped and closed,
                        // which is the right end for a path nothing can name any more.
                        held.paths.insert(session, path);
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
                // Dropping the transports closes them, and the producer and consumers carried
                // on them go too. There is nothing to await and nothing that can fail.
                held.paths.remove(&session);
                held.audiences.remove(&session);
                held.counting.remove(&session);
                speaking.no_longer(&session);
                // **Everybody hearing this talker is told.** Their carriage is closed at this
                // end by the producer going, and a client left holding one it will never be
                // sent audio on again would show a talker who is not there.
                nobody_hears(&mut held, &session);
            }
            Instruction::WillHear {
                session,
                what_it_can_decode,
            } => {
                if let Some(can_decode) = what_a_client_can_decode(what_it_can_decode) {
                    if let Some(path) = held.paths.get_mut(&session) {
                        path.can_decode = Some(can_decode);
                    }
                    // Whatever this session was already meant to hear, now that it can.
                    carry_what_was_already_said(&owned.router, &mut held, &session).await;
                    if let Some(on) = held.counting.get(&session).cloned() {
                        these_count(&owned.router, &mut held, &session, &on).await;
                    }
                }
            }
            Instruction::Connect { session, way, keys } => {
                connect(&held, &session, way, keys).await;
            }
            Instruction::Speaks {
                session,
                what_it_is_sending,
            } => {
                speak(&owned, &speaking, &mut held, &session, what_it_is_sending).await;
            }
            Instruction::Hears { session, carriage } => {
                resume(&held, &session, &carriage).await;
            }
            Instruction::Hear { talker, audience } => {
                held.audiences.insert(talker.clone(), audience.clone());
                these_hear(&owned.router, &mut held, &talker, &audience).await;
            }
            Instruction::Beacons(loops) => {
                every_loop_runs_a_beacon(&owned, &mut held, &loops).await;
            }
            Instruction::Count { listener, on } => {
                held.counting.insert(listener.clone(), on.clone());
                these_count(&owned.router, &mut held, &listener, &on).await;
            }
        }
    }
}

/// Make exactly these loops run beacons: start one for each loop that has none, and stop the
/// beacon of each loop no longer named, with every carriage of it.
///
/// **A loop nobody counts runs its beacon all the same** (ADR-0017). It is started here, when
/// the loop is named, rather than when somebody first counts it — otherwise the mechanism
/// would be unavailable at exactly the moment somebody subscribes.
///
/// **Nothing here reaches the `AudioLevelObserver`**, and nothing here could reach the
/// recording tap: a beacon is not a session's uplink, so it is never in `Speaking` and never
/// a talker in an audience. Letting it into either would register a permanent talker on
/// every loop and a permanent signal in every recording.
async fn every_loop_runs_a_beacon(owned: &Owned, held: &mut Paths, loops: &[Destination]) {
    let gone: Vec<Destination> = held
        .beacons
        .keys()
        .filter(|running| !loops.contains(running))
        .cloned()
        .collect();
    for loop_gone in &gone {
        held.beacons.remove(loop_gone);
        for path in held.paths.values_mut() {
            path.stop_counting(loop_gone);
        }
        // And it is forgotten as something anybody was told to count, so what is remembered
        // stays what was instructed **of the loops there are**. Keeping it would leave a
        // destination in here that nothing can ever satisfy, which is a memory of an
        // instruction quietly becoming a superset of one.
        for counted in held.counting.values_mut() {
            counted.retain(|named| named != loop_gone);
        }
    }

    for named in loops {
        if held.beacons.contains_key(named) {
            continue;
        }

        let ssrc = held.next_ssrc;
        held.next_ssrc = held.next_ssrc.wrapping_add(1);
        match owned
            .beacons_on
            .produce(ProducerOptions::new(
                MediaKind::Audio,
                what_a_beacon_sends(ssrc),
            ))
            .await
        {
            Ok(producer) => {
                held.beacons.insert(
                    named.clone(),
                    Beacon {
                        producer,
                        ssrc,
                        sequence: 0,
                        timestamp: 0,
                    },
                );
            }
            Err(error) => tracing::error!(
                target: module::MEDIA_PLANE,
                %error,
                "a loop has no beacon, so everybody monitoring it will read it as not received"
            ),
        }
    }

    // Whoever was already meant to count a beacon that has just come into being.
    let already: Vec<(SessionId, Vec<Destination>)> = held
        .counting
        .iter()
        .map(|(listener, on)| (listener.clone(), on.clone()))
        .collect();
    for (listener, on) in already {
        these_count(&owned.router, held, &listener, &on).await;
    }
}

/// What a beacon's producer says it is sending: Opus, under one synchronisation source.
///
/// One per beacon, because every beacon is produced on the one direct transport and a
/// transport tells its producers apart by exactly this.
fn what_a_beacon_sends(ssrc: u32) -> RtpParameters {
    RtpParameters {
        codecs: vec![RtpCodecParameters::Audio {
            mime_type: MimeTypeAudio::Opus,
            payload_type: THE_BEACON_PAYLOAD_TYPE,
            clock_rate: NonZeroU32::new(48_000).expect("48000 is not zero"),
            channels: NonZeroU8::new(2).expect("2 is not zero"),
            parameters: RtpCodecParametersParameters::default(),
            rtcp_feedback: Vec::new(),
        }],
        encodings: vec![RtpEncodingParameters {
            ssrc: Some(ssrc),
            ..RtpEncodingParameters::default()
        }],
        ..RtpParameters::default()
    }
}

/// Make exactly these beacons reach this listener: a carriage for each loop named that it does
/// not have yet, and none for any loop it was not named on.
///
/// It is the same reconciliation [`these_hear`] does for a talker, over loops rather than
/// talkers — and it is **one carriage per loop**, never per (loop, talker) (v1 §16).
async fn these_count(router: &Router, held: &mut Paths, listener: &SessionId, on: &[Destination]) {
    let Some(path) = held.paths.get_mut(listener) else {
        return;
    };
    let stale: Vec<Destination> = path
        .counting
        .keys()
        .filter(|counted| !on.contains(counted))
        .cloned()
        .collect();
    for loop_dropped in &stale {
        path.stop_counting(loop_dropped);
    }

    for named in on {
        one_more_beacon(router, held, listener, named).await;
    }
}

/// Give one listener a carriage of one loop's beacon, where they do not have one already.
///
/// Built **paused**, like a talker's, and resumed when the client says it has built its end:
/// a beacon sent to an end that does not exist yet is a packet the client cannot count, and
/// the loop would read as not received for no reason but the order things were built in.
async fn one_more_beacon(
    router: &Router,
    held: &mut Paths,
    listener: &SessionId,
    on: &Destination,
) {
    let Some(beacon) = held.beacons.get(on).map(|beacon| beacon.producer.id()) else {
        // The loop has no beacon yet. The instruction is kept and replayed when it has.
        return;
    };
    let Some(path) = held.paths.get(listener) else {
        return;
    };
    if path.counting.contains_key(on) {
        return;
    }

    let Some((carriage, what_to_build)) =
        a_paused_carriage(router, path, listener, beacon, "a loop's beacon").await
    else {
        // Nothing is counted on a carriage that was not built, so the loop reads as not
        // received — which is the truth about that listener.
        return;
    };

    if let Some(path) = held.paths.get_mut(listener) {
        let _ = path.telling.send(Negotiated::OneMoreBeacon {
            beacon: what_to_build,
            on: on.clone(),
        });
        path.counting.insert(on.clone(), carriage);
    }
}

/// Build one session's two transports and start watching them.
///
/// **Bound to the session at creation** (ADR-0026): they are created for a session that
/// already exists and are filed under it in the same breath, so there is no moment at which
/// one exists unclaimed and nothing to present in order to claim one.
async fn open(
    router: &Router,
    server: &WebRtcServer,
    session: &SessionId,
    telling: Telling,
    reporting: &Reporting,
) -> Option<Path> {
    let up = one_transport(router, server, session).await?;
    let down = one_transport(router, server, session).await?;

    // Both ends of one reading, shared by four callbacks so that none of them has to hold a
    // transport that owns it. ICE and DTLS move independently, either can be the one that
    // fails, and so can either direction — so what is reported is the worst of the four. That
    // is the same pessimism ADR-0042 applies between the client and the server, applied here
    // across the whole of the server's own end: a session that cannot be heard has no
    // emission path whichever half of its path is broken.
    let seen = Arc::new(Mutex::new(Seen {
        up: (up.ice_state(), up.dtls_state()),
        down: (down.ice_state(), down.dtls_state()),
        said: None,
    }));

    for (transport, way) in [(&up, Way::Up), (&down, Way::Down)] {
        transport
            .on_ice_state_change({
                let seen = Arc::clone(&seen);
                let reporting = reporting.clone();
                let session = session.clone();
                move |ice| {
                    say_if_it_moved(&seen, &reporting, &session, |seen| seen.end(way).0 = ice);
                }
            })
            .detach();
        transport
            .on_dtls_state_change({
                let seen = Arc::clone(&seen);
                let reporting = reporting.clone();
                let session = session.clone();
                move |dtls| {
                    say_if_it_moved(&seen, &reporting, &session, |seen| seen.end(way).1 = dtls);
                }
            })
            .detach();
    }

    // What the server can see of a path nobody has connected to yet, said out loud rather
    // than left to be inferred: transports exist and no audio can cross them.
    say_if_it_moved(&seen, reporting, session, |_| {});

    // **What the client needs in order to build its own end**, and the first thing it is
    // sent. It goes down the session's own channel rather than being answered to a caller,
    // because a sink answers nothing (ADR-0062).
    match what_to_build(router, &up, &down) {
        Some(offer) => {
            let _ = telling.send(Negotiated::APathToBuild(offer));
        }
        None => {
            tracing::error!(
                target: module::MEDIA_PLANE,
                session = session.as_str(),
                "a media path could not be described to its client"
            );
            return None;
        }
    }

    Some(Path {
        up,
        down,
        telling,
        can_decode: None,
        producer: None,
        hearing: HashMap::new(),
        counting: HashMap::new(),
    })
}

/// One transport on the one router and the one port.
async fn one_transport(
    router: &Router,
    server: &WebRtcServer,
    session: &SessionId,
) -> Option<WebRtcTransport> {
    match router
        .create_webrtc_transport(WebRtcTransportOptions::new_with_server(server.clone()))
        .await
    {
        Ok(transport) => Some(transport),
        Err(error) => {
            tracing::error!(
                target: module::MEDIA_PLANE,
                %error,
                session = session.as_str(),
                "a media path could not be built"
            );
            None
        }
    }
}

/// What the client's own library has to be handed to build its end of the path.
///
/// It is assembled as JSON and crosses the seam as an opaque [`Negotiation`], which is what
/// keeps `IceParameters` and the rest of mediasoup's vocabulary inside this module.
fn what_to_build(
    router: &Router,
    up: &WebRtcTransport,
    down: &WebRtcTransport,
) -> Option<Negotiation> {
    serde_json::to_value(serde_json::json!({
        "router": router.rtp_capabilities(),
        "up": one_end(up),
        "down": one_end(down),
    }))
    .ok()
    .map(Negotiation::presented)
}

/// One transport, as the far end has to see it.
fn one_end(transport: &WebRtcTransport) -> serde_json::Value {
    serde_json::json!({
        "id": transport.id(),
        "iceParameters": transport.ice_parameters(),
        "iceCandidates": transport.ice_candidates(),
        "dtlsParameters": transport.dtls_parameters(),
    })
}

/// What a client says it can decode, or nothing if it said something unreadable.
///
/// A client whose capabilities cannot be read is a client that will be sent nothing, which is
/// the honest end: it will hear silence and its console will say the path is not carrying,
/// rather than being sent streams it cannot play.
fn what_a_client_can_decode(said: Negotiation) -> Option<RtpCapabilities> {
    match serde_json::from_value(said.0) {
        Ok(can_decode) => Some(can_decode),
        Err(error) => {
            tracing::warn!(
                target: module::MEDIA_PLANE,
                %error,
                "a client described what it can decode in terms this server could not read"
            );
            None
        }
    }
}

/// Hand the client's keys for one end of its path to the worker.
async fn connect(held: &Paths, session: &SessionId, way: Way, keys: Negotiation) {
    let Some(path) = held.paths.get(session) else {
        return;
    };

    let dtls_parameters = match serde_json::from_value(keys.0) {
        Ok(parameters) => parameters,
        Err(error) => {
            tracing::warn!(
                target: module::MEDIA_PLANE,
                %error,
                session = session.as_str(),
                way = way.as_str(),
                "a client's keys could not be read"
            );
            return;
        }
    };

    let transport = match way {
        Way::Up => &path.up,
        Way::Down => &path.down,
    };

    if let Err(error) = transport
        .connect(WebRtcTransportRemoteParameters { dtls_parameters })
        .await
    {
        // Nothing is told, because there is nobody to tell: a transport that will not connect
        // is a media path that stays `lost`, which the ladder already says on its own.
        tracing::warn!(
            target: module::MEDIA_PLANE,
            %error,
            session = session.as_str(),
            way = way.as_str(),
            "one end of a media path would not connect"
        );
    }
}

/// Take a client's uplink: one producer, for as long as its microphone exists.
///
/// **Audio only, whatever the client said.** VoxLoop carries voice and nothing else, and the
/// kind is decided here rather than taken from the message, so there is no way to ask this
/// server to carry video by saying so.
async fn speak(
    owned: &Owned,
    speaking: &Speaking,
    held: &mut Paths,
    session: &SessionId,
    what_it_is_sending: Negotiation,
) {
    let rtp_parameters = match serde_json::from_value::<Sending>(what_it_is_sending.0) {
        Ok(sending) => sending.rtp_parameters,
        Err(error) => {
            tracing::warn!(
                target: module::MEDIA_PLANE,
                %error,
                session = session.as_str(),
                "a client described what it is sending in terms this server could not read"
            );
            return;
        }
    };

    let Some(path) = held.paths.get(session) else {
        return;
    };

    let producer = match path
        .up
        .produce(ProducerOptions::new(MediaKind::Audio, rtp_parameters))
        .await
    {
        Ok(producer) => producer,
        Err(error) => {
            tracing::error!(
                target: module::MEDIA_PLANE,
                %error,
                session = session.as_str(),
                "an uplink could not be taken"
            );
            return;
        }
    };

    let name = producer.id();
    speaking.now(name, session.clone());

    // **The observer is told about every uplink there is** (ADR-0008). A producer left out of
    // it would be the one client whose discrepancy nothing could see, which is the same as
    // not running the observer at all for that client.
    if let Err(error) = owned
        .observer
        .add_producer(RtpObserverAddProducerOptions::new(name))
        .await
    {
        tracing::error!(
            target: module::MEDIA_PLANE,
            %error,
            session = session.as_str(),
            "an uplink is not being watched, so a client sending while unkeyed would go unseen"
        );
    }

    if let Some(path) = held.paths.get_mut(session) {
        path.producer = Some(producer);
        let _ = path
            .telling
            .send(Negotiated::TheUplinkIsCarried(Carried(name.to_string())));
    }

    // Whatever this talker was already meant to be heard by, now that there is something to
    // hear. The answer is the one that was handed down; nothing here decided it.
    if let Some(audience) = held.audiences.get(session).cloned() {
        these_hear(&owned.router, held, session, &audience).await;
    }
}

/// Make exactly this audience hear this talker.
///
/// **The whole audience each time, and nothing here has a view about it** ([ADR-0063]). What
/// this does is reconcile: everyone in the answer who is not already hearing this talker gets
/// a carriage, and everyone hearing them who is not in the answer loses theirs.
///
/// **A listener named twice gets one carriage.** The pairs arrive per (listener, destination)
/// because the recording tap is addressed that way ([ADR-0009]); the downlink is per audible
/// talker ([ADR-0007]), so collapsing them is this module's job and delivering two would hand
/// somebody the same voice twice. The destinations go with the carriage, and move with it
/// when the answer does, because the client plays it at the loudest of their volumes.
///
/// [ADR-0007]: ../../docs/adr/0007-the-client-emits-one-stream.md
/// [ADR-0009]: ../../docs/adr/0009-recording-taps-plain-rtp-on-loopback.md
/// [ADR-0063]: ../../docs/adr/0063-the-media-plane-executes-routing-it-never-computes-it.md
async fn these_hear(router: &Router, held: &mut Paths, talker: &SessionId, audience: &Audience) {
    let named = audience.by_listener();

    for (listener, path) in held.paths.iter_mut() {
        match named.iter().find(|(named, _)| named == listener) {
            Some((_, on)) => path.heard_on(talker, on),
            None => path.stop_hearing(talker),
        }
    }

    let Some(uplink) = held
        .paths
        .get(talker)
        .and_then(|path| path.producer.as_ref())
        .map(Producer::id)
    else {
        // The talker has an audience and is not sending yet. The answer is kept and replayed
        // the moment they are, which is what `Paths::audiences` is for.
        return;
    };

    for (listener, on) in named {
        one_more_talker(router, held, &listener, talker, uplink, on).await;
    }
}

/// Everything this session was already meant to hear, carried now that it can be.
async fn carry_what_was_already_said(router: &Router, held: &mut Paths, listener: &SessionId) {
    let already: Vec<(SessionId, Audience)> = held
        .audiences
        .iter()
        .filter(|(_, audience)| {
            audience
                .hearing
                .iter()
                .any(|hearing| &hearing.listener == listener)
        })
        .map(|(talker, audience)| (talker.clone(), audience.clone()))
        .collect();

    for (talker, audience) in already {
        these_hear(router, held, &talker, &audience).await;
    }
}

/// Give one listener a carriage for one talker, where they do not have one already.
///
/// It is built **paused**, and stays that way until the client says it has built its own end.
/// Audio sent to an end that does not exist yet is audio nobody hears, and a talker whose
/// first word went that way would be the *"Flight, CAPCOM"* that identifies the speaker.
async fn one_more_talker(
    router: &Router,
    held: &mut Paths,
    listener: &SessionId,
    talker: &SessionId,
    uplink: ProducerId,
    on: Vec<Destination>,
) {
    let Some(path) = held.paths.get(listener) else {
        return;
    };
    if path.hearing.contains_key(talker) {
        return;
    }

    let Some((carriage, what_to_build)) =
        a_paused_carriage(router, path, listener, uplink, "a talker").await
    else {
        return;
    };

    if let Some(path) = held.paths.get_mut(listener) {
        let _ = path.telling.send(Negotiated::OneMoreTalker {
            talker: what_to_build,
            heard_on: on.clone(),
        });
        path.hearing.insert(talker.clone(), Heard { carriage, on });
    }
}

/// Build one paused carriage on this listener's downlink, and what their client needs in order
/// to build the far end of it.
///
/// **The two things a listener is sent are built exactly alike**, and this is that shape
/// written once: what the client says it can decode is checked against what it would be sent,
/// the carriage is built **paused**, and the description goes back for the client to answer.
///
/// What differs stays with the callers, because it is the only part that is not this: what the
/// carriage is filed under — a talker, or a loop — and which message says so. `carrying` is for
/// the log, so that a carriage that could not be built says which of the two it was.
async fn a_paused_carriage(
    router: &Router,
    path: &Path,
    listener: &SessionId,
    of: ProducerId,
    carrying: &'static str,
) -> Option<(Consumer, Negotiation)> {
    // Nothing where the client has not said what it can decode yet: the answer is kept above
    // and replayed when it does.
    let can_decode = path.can_decode.clone()?;
    if !router.can_consume(&of, &can_decode) {
        tracing::warn!(
            target: module::MEDIA_PLANE,
            listener = listener.as_str(),
            carrying,
            "a client cannot decode what this deployment carries, and will hear nothing"
        );
        return None;
    }

    let carriage = match path
        .down
        .consume({
            let mut options = ConsumerOptions::new(of, can_decode);
            options.paused = true;
            options
        })
        .await
    {
        Ok(carriage) => carriage,
        Err(error) => {
            tracing::error!(
                target: module::MEDIA_PLANE,
                %error,
                listener = listener.as_str(),
                carrying,
                "a carriage could not be built, so a listener is not being sent something"
            );
            return None;
        }
    };

    let what_to_build = serde_json::json!({
        "id": carriage.id(),
        "producerId": carriage.producer_id(),
        "kind": carriage.kind(),
        "rtpParameters": carriage.rtp_parameters(),
    });

    Some((carriage, Negotiation::presented(what_to_build)))
}

/// Nobody hears this talker any more, because there is no longer a talker to hear.
fn nobody_hears(held: &mut Paths, talker: &SessionId) {
    for path in held.paths.values_mut() {
        path.stop_hearing(talker);
    }
}

/// Start sending on a carriage the client has now built its own end of.
async fn resume(held: &Paths, session: &SessionId, carriage: &Carried) {
    let Some(path) = held.paths.get(session) else {
        return;
    };

    let Some(carriage) = path
        .hearing
        .values()
        .map(|heard| &heard.carriage)
        .chain(path.counting.values())
        .find(|held| held.id().to_string() == carriage.0)
    else {
        // A name for a carriage this session is not being sent audio on. It is stale rather
        // than sinister — a talker who stopped between the offer and the answer — and the
        // right response to a carriage that is gone is to do nothing with it.
        return;
    };

    if let Err(error) = carriage.resume().await {
        tracing::warn!(
            target: module::MEDIA_PLANE,
            %error,
            session = session.as_str(),
            "a carriage would not start, so somebody is not hearing a talker"
        );
    }
}

/// What a client says it is sending, in the shape its own library sends it.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sending {
    rtp_parameters: RtpParameters,
}

/// What the server end has seen of one session's two transports, and what it last said.
struct Seen {
    /// ICE and DTLS on the uplink.
    up: (IceState, DtlsState),
    /// ICE and DTLS on the downlink.
    down: (IceState, DtlsState),
    /// The last reading put on the channel, so a change that does not move the merged answer
    /// does not wake anything up. The ladder has three rungs and these four states have far
    /// more combinations than that between them.
    said: Option<MediaPath>,
}

impl Seen {
    /// One end, to be written by whichever callback fired.
    fn end(&mut self, way: Way) -> &mut (IceState, DtlsState) {
        match way {
            Way::Up => &mut self.up,
            Way::Down => &mut self.down,
        }
    }

    /// What the four of them amount to: the worst reading of the lot.
    fn merged(&self) -> MediaPath {
        [self.up, self.down]
            .into_iter()
            .map(|(ice, dtls)| from_ice(ice).pessimistically_with(from_dtls(dtls)))
            .fold(MediaPath::Connected, MediaPath::pessimistically_with)
    }
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

    let now = seen.merged();
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

impl Carriage for Carrying {
    fn open_a_path_for(&self, session: &SessionId, telling: Telling) {
        self.tell(Instruction::Open {
            session: session.clone(),
            telling,
        });
    }

    fn close_the_path_of(&self, session: &SessionId) {
        self.tell(Instruction::Close(session.clone()));
    }

    fn the_client_will_hear(&self, session: &SessionId, what_it_can_decode: Negotiation) {
        self.tell(Instruction::WillHear {
            session: session.clone(),
            what_it_can_decode,
        });
    }

    fn the_client_connects(&self, session: &SessionId, way: Way, keys: Negotiation) {
        self.tell(Instruction::Connect {
            session: session.clone(),
            way,
            keys,
        });
    }

    fn the_client_speaks(&self, session: &SessionId, what_it_is_sending: Negotiation) {
        self.tell(Instruction::Speaks {
            session: session.clone(),
            what_it_is_sending,
        });
    }

    fn the_client_hears(&self, session: &SessionId, carriage: &Carried) {
        self.tell(Instruction::Hears {
            session: session.clone(),
            carriage: carriage.clone(),
        });
    }

    fn these_should_hear(&self, talker: &SessionId, audience: &Audience) {
        self.tell(Instruction::Hear {
            talker: talker.clone(),
            audience: audience.clone(),
        });
    }

    fn these_loops_run_beacons(&self, loops: &[Destination]) {
        self.tell(Instruction::Beacons(loops.to_vec()));
    }

    fn these_beacons_reach(&self, listener: &SessionId, on: &[Destination]) {
        self.tell(Instruction::Count {
            listener: listener.clone(),
            on: on.to_vec(),
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
    /// a `WebRtcServer` come up, that a session gets transports of its own, that what its
    /// client needs in order to build the far end is composed and put on the session's own
    /// channel, and that mediasoup's callbacks reach a tokio task over the channel rather
    /// than by any other route.
    ///
    /// The port is whatever is free, because a fixed one would make this test a question
    /// about the machine it runs on.
    #[tokio::test]
    async fn a_worker_a_router_and_one_port_come_up_and_a_session_gets_a_path_of_its_own() {
        let media = crate::configuration::Media::on_loopback();

        let (carrying, mut reports, carriageway) =
            start(&media).await.expect("the media plane to come up");

        let session = SessionId::presented("a-session".to_owned());
        let (telling, mut told) = tokio::sync::mpsc::unbounded_channel();
        carrying.open_a_path_for(&session, telling);

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

        // **What the client is handed to build its own end**, on the session's own channel
        // rather than as the answer to a call, because a sink answers nothing. It is opaque
        // above this module, so what is asserted here is that it is the shape a media library
        // can act on: what the router carries, and one described transport each way.
        let offer = tokio::time::timeout(std::time::Duration::from_secs(10), told.recv())
            .await
            .expect("the media plane to describe the path within ten seconds")
            .expect("something to build");

        let Negotiated::APathToBuild(Negotiation(offer)) = offer else {
            panic!("the first thing a session is told was not the path to build");
        };
        assert!(
            offer["router"]["codecs"].is_array(),
            "a client was not told what this deployment carries: {offer}"
        );
        for way in ["up", "down"] {
            for named in ["id", "iceParameters", "iceCandidates", "dtlsParameters"] {
                assert!(
                    !offer[way][named].is_null(),
                    "the {way}link was described without {named}: {offer}"
                );
            }
        }
        // **Two ends of one path and not two paths**: a directional transport each way, on
        // the one router and the one port, rather than anything per loop (ADR-0007).
        assert_ne!(offer["up"]["id"], offer["down"]["id"]);

        // **A loop runs its beacon whether or not anybody counts it** (ADR-0017), and a
        // session monitoring it is carried one carriage of it — built paused, named by the
        // loop it measures, and on the session's own channel like everything else. What the
        // client can decode is the router's own offer read back, which is what a browser's
        // library would say after loading it.
        let flight = Destination::labelled("flight".to_owned());
        carrying.these_loops_run_beacons(std::slice::from_ref(&flight));
        carrying.the_client_will_hear(&session, Negotiation::presented(offer["router"].clone()));
        carrying.these_beacons_reach(&session, std::slice::from_ref(&flight));

        let beacon = tokio::time::timeout(std::time::Duration::from_secs(10), told.recv())
            .await
            .expect("the beacon to be carried within ten seconds")
            .expect("a beacon");
        let Negotiated::OneMoreBeacon {
            beacon: Negotiation(beacon),
            on,
        } = beacon
        else {
            panic!("a session monitoring a loop was not carried its beacon: {beacon:?}");
        };
        assert_eq!(on, flight);
        assert_eq!(beacon["kind"], "audio");

        // And the loop leaving the set takes its beacon, and the carriage of it, with it.
        carrying.these_loops_run_beacons(&[]);
        let gone = tokio::time::timeout(std::time::Duration::from_secs(10), told.recv())
            .await
            .expect("the beacon to go within ten seconds")
            .expect("something said");
        assert_eq!(
            gone,
            Negotiated::OneFewerBeacon(Carried(beacon["id"].as_str().expect("an id").to_owned()))
        );

        // And it goes when the session does, with nothing to await and nothing to check.
        carrying.close_the_path_of(&session);
        carriageway.stop();
    }

    /// The server's own end is the worse of **four** readings, because there are two
    /// transports and either of them failing is a session that cannot be heard.
    #[test]
    fn one_end_of_the_path_failing_takes_the_whole_of_it() {
        let both_through = Seen {
            up: (IceState::Connected, DtlsState::Connected),
            down: (IceState::Connected, DtlsState::Connected),
            said: None,
        };
        assert_eq!(both_through.merged(), MediaPath::Connected);

        let downlink_gone = Seen {
            down: (IceState::Disconnected, DtlsState::Connected),
            ..both_through
        };
        assert_eq!(downlink_gone.merged(), MediaPath::Lost);
    }

    /// The two halves of the server's own reading, and the rule that a transport nobody has
    /// connected to carries no audio however far along either half is.
    #[test]
    fn the_server_reads_ice_and_dtls_together_and_takes_the_worse() {
        assert_eq!(
            from_ice(IceState::Completed).pessimistically_with(from_dtls(DtlsState::Connected)),
            MediaPath::Connected
        );
        // ICE through, DTLS not: packets arrive and nothing can decrypt them.
        assert_eq!(
            from_ice(IceState::Connected).pessimistically_with(from_dtls(DtlsState::Connecting)),
            MediaPath::Lost
        );
        // A transport nobody has connected to yet is the same answer as one that has failed,
        // because the operator can do the same thing about both: nothing, yet.
        assert_eq!(
            from_ice(IceState::New).pessimistically_with(from_dtls(DtlsState::New)),
            MediaPath::Lost
        );
        // mediasoup has no `failed`, and its `disconnected` is thirty seconds of ICE consent
        // freshness — old news by the time it lands, so it is taken at its worst.
        assert_eq!(from_ice(IceState::Disconnected), MediaPath::Lost);
    }

    /// **A beacon is one silent RTP packet**, stamped on from the last: version 2, the
    /// beacon's own payload type, its own synchronisation source, and Opus's silence frame as
    /// the whole of the payload. Nothing in it could register as a voice.
    #[test]
    fn a_beacon_sounds_one_silent_packet_at_a_time() {
        let packet = a_silent_packet(7, 240_000, 42);

        assert_eq!(
            packet[0], 0x80,
            "not RTP version 2, or carrying padding or extensions"
        );
        assert_eq!(
            packet[1], THE_BEACON_PAYLOAD_TYPE,
            "marked, or under another type"
        );
        assert_eq!(&packet[2..4], &7_u16.to_be_bytes());
        assert_eq!(&packet[4..8], &240_000_u32.to_be_bytes());
        assert_eq!(&packet[8..12], &42_u32.to_be_bytes());
        assert_eq!(&packet[12..], &OPUS_SILENCE);
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

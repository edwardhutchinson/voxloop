// The client's Audio module: the microphone, the streams that arrive, and the mixing.
//
// **Three layers, and only the middle one knows what a loop is** (ADR-0007). This is the two
// ends. The **uplink** is one stream, encoded once, whatever the operator is armed on — it
// transmits and it does not address, so there is nothing here that names a loop and nothing
// here that changes when an arm does. The **downlink** is one stream per audible talker, and
// the mixing is the browser playing them together.
//
// **Keying is done here and signalled, never asked for** (ADR-0008). The key is the local
// track being enabled or disabled, which is why it costs no round trip and no renegotiation —
// key-to-first-audio under 100 ms is what makes this feel like a radio rather than a
// conference call. Nothing in here lights a lamp: the console lights that from the presence
// document, when the server says so.
//
// **Nothing that arrives says who is talking**, and there is nowhere in the message that
// could (ADR-0033). A carriage is a stream and a name to quote back, and this plays it.
//
// **Each talker plays at the loudest volume among the loops it is heard on** (v1 §5,
// ADR-0007). The server says which of this operator's loops each carriage is heard on and the
// presence document says what each loop is set to, and `loudest` below is the whole of the
// rule — the browser's per-element volume is the gain. **Priority plays at full gain**, and it
// bypasses the rule rather than competing with it (ADR-0045): `theGain` is the two together.
//
// **Nothing here ever lowers anything for anybody else** (v1 §4). There is no ducking in
// VoxLoop, so no talker's gain is ever read off another talker's.
//
// **Each loop's beacon is counted here and never played** (ADR-0017). It is a carriage like a
// talker's, built the same way on the same downlink, and that is the point: its arrival is a
// measurement over the same transport, router and fan-out that speech would take. What goes
// back up is the running count per loop and nothing concluded from it — the server judges —
// so a client that has wedged says nothing, and nothing is read as receiving nothing.

import { Device } from 'mediasoup-client';

// Opus, mono, 20 ms frames, a ceiling around 32 kbps, inband FEC and DTX both on (ADR-0010).
// The router advertises `useinbandfec` and `usedtx`; these are the encoder's half of the same
// decision, and they are here because the encoder is the browser's.
const HOW_IT_IS_ENCODED = {
	opusStereo: false,
	opusFec: true,
	opusDtx: true,
	opusPtime: 20,
	opusMaxPlaybackRate: 48000,
	opusMaxAverageBitrate: 32000
};

// Mono, because panning is a presentation choice the console makes over mono sources and
// costs nothing on the wire (ADR-0010). The three cleanups are the browser's own and are
// what a headset in a control room wants.
// How often each beacon's count is read, and said where it has moved. Faster than the beacon
// sounds, so that a packet is reported within a second of arriving rather than an interval
// late, and nothing is sent at all while nothing arrives.
const BEACONS_ARE_COUNTED_EVERY = 1000;

const WHAT_THE_MICROPHONE_IS_ASKED_FOR = {
	audio: {
		channelCount: 1,
		echoCancellation: true,
		noiseSuppression: true,
		autoGainControl: true
	}
};

/**
 * Build this session's end of the audio path, and answer with what the socket hands it.
 *
 * `say` is the Session module's media acts. Audio calls Session and Session never calls
 * Audio, which is what keeps the client's call graph acyclic like the server's (ADR-0062):
 * everything that arrives comes in through the four handlers below, and everything that
 * leaves goes out through `say`.
 *
 * `onMediaPath` is where this end's reading of the ladder goes. **The client drives it**
 * (ADR-0042), because it is the end that can tell a transient `disconnected` from a terminal
 * `failed`; the server merges it pessimistically with its own and pushes the answer back in
 * the presence document, which is the only thing the console renders.
 */
export function openAudio({
	say,
	onMediaPath,
	// What reads the beacons' counts on a clock, handed in for the reason `openSignalling`
	// takes one: nothing in here reaches for a global timer.
	ticking = (act, every) => {
		const timer = setInterval(act, every);

		return () => clearInterval(timer);
	}
}) {
	const device = new Device();
	// One element per audible talker, and the loops it is heard on. The browser mixing
	// several at once **is** the client-side mixing ADR-0007 asks for; the gain on each is
	// the loudest volume among its loops.
	const playing = new Map();
	// The loops as the last presence document had them — what each is set to and whether it
	// is muted. Nothing here renders it: it is read for the gain and for nothing else.
	let loops = [];

	function play(held) {
		held.heard.volume = theGain(held.heardOn, loops);
	}

	let microphone = null;
	// The `produce` callback, held between asking the server to carry the uplink and being
	// told its name. mediasoup-client will not call a producer published until it has one.
	let waitingForTheUplink = null;
	let sending = null;
	let receiving = null;
	let closed = false;

	// One carriage per loop whose beacon this session counts, by carriage id, and the loop it
	// measures. **None of them is ever given an element to play through**: a beacon is silent
	// anyway, and one mixed into somebody's ears would be a talker nobody is.
	const counting = new Map();
	// What was last said, so that a count that has not moved is not said again: the server
	// reads the silence as the beacon not arriving, which is exactly what it is.
	let saidCounted = null;

	async function countTheBeacons() {
		if (counting.size === 0) return;

		const counted = {};
		for (const { carriage, on } of counting.values()) {
			try {
				counted[on] = packetsIn(await carriage.getStats());
			} catch {
				// A carriage closing under the read counts nothing this time, and is gone by
				// the next.
			}
		}

		const now = JSON.stringify(counted);
		if (closed || now === saidCounted) return;
		saidCounted = now;
		say.beaconsCounted(counted);
	}

	const stopCounting = ticking(countTheBeacons, BEACONS_ARE_COUNTED_EVERY);

	// This end of the ladder, merged the same way the server merges its two: green needs
	// both, red needs one. A session that cannot receive is as unable to work as one that
	// cannot send, and the transmit bar has one thing to say about either.
	const ends = { up: 'lost', down: 'lost' };
	let said = null;

	function reading(way, state) {
		ends[way] = onTheLadder(state);
		const now = worst(ends.up, ends.down);
		if (now === said) return;

		said = now;
		onMediaPath(now);
	}

	async function buildTheTransports(path) {
		await device.load({ routerRtpCapabilities: path.router });
		// **What this end can decode**, said before anything is carried to it: a stream a
		// client cannot play is worse than no stream, because the console would show it as
		// heard.
		say.mediaCanDecode(device.rtpCapabilities);

		sending = device.createSendTransport(path.up);
		receiving = device.createRecvTransport(path.down);

		for (const [transport, way] of [
			[sending, 'up'],
			[receiving, 'down']
		]) {
			// **The server answers nothing** (ADR-0062), so the keys go and the callback is
			// called: there is no acknowledgement to wait for, and a transport that will not
			// connect says so through the ladder rather than through this.
			transport.on('connect', ({ dtlsParameters }, carry) => {
				say.mediaConnect(way, dtlsParameters);
				carry();
			});
			transport.on('connectionstatechange', (state) => reading(way, state));
		}

		sending.on('produce', ({ rtpParameters }, carry) => {
			waitingForTheUplink = carry;
			say.mediaSpeaks({ rtpParameters });
		});
	}

	async function openTheMicrophone() {
		const stream = await navigator.mediaDevices.getUserMedia(WHAT_THE_MICROPHONE_IS_ASKED_FOR);
		[microphone] = stream.getAudioTracks();
		// **Unkeyed is the state a console starts in**, and the track carries that from the
		// moment it exists rather than from the first unkey: an operator who has just taken a
		// seat is not transmitting, and a microphone that was live for the instant between
		// getting it and being told otherwise would be exactly the open mic this product is
		// about.
		microphone.enabled = false;

		// The `Producer` is not held here. The transport it was made on holds it, and closing
		// that transport is what ends it — so a second reference would only be a second thing
		// to remember to let go of.
		await sending.produce({ track: microphone, codecOptions: HOW_IT_IS_ENCODED });
		// A microphone unplugged is a source that has died, and it is reported as this end of
		// the path going rather than being left to be discovered by nobody hearing anything.
		microphone.addEventListener('ended', () => reading('up', 'failed'));
	}

	return {
		/**
		 * The server has described this session's path. Build the far end of it.
		 *
		 * Everything after this is driven by the two transports and by what the server says
		 * next; nothing here polls and nothing here retries. A path that will not come up
		 * shows as a media path that is `lost`, which is what the transmit bar says and what
		 * the operator can act on.
		 */
		async aPathToBuild(path) {
			try {
				await buildTheTransports(path);
				await openTheMicrophone();
			} catch (why) {
				// The one place this end can fail outright — no microphone, no permission, an
				// unreadable offer — and the honest reading of all of them is the same: this
				// session has no way to be heard.
				reading('up', 'failed');
				console.error('VoxLoop could not build this session’s audio path', why);
			}
		},

		/** The uplink is carried, under this name. It is quoted straight back to the library. */
		theUplinkIsCarried(carriage) {
			waitingForTheUplink?.({ id: carriage });
			waitingForTheUplink = null;
		},

		/**
		 * One more talker to hear.
		 *
		 * The carriage is built and then **said to be built**, in that order, because the
		 * server holds it paused until it hears — audio sent to an end that does not exist
		 * yet is audio nobody hears, and a talker whose first word went that way would be the
		 * *"Flight, CAPCOM"* that identifies the speaker.
		 */
		async oneMoreTalker(talker, heardOn = []) {
			if (closed || !receiving) return;

			try {
				const carriage = await receiving.consume(talker);
				const heard = new Audio();
				heard.autoplay = true;
				heard.srcObject = new MediaStream([carriage.track]);
				const held = { carriage, heard, heardOn };
				// The level is set before the first sample plays rather than after, or a
				// talker on a loop turned down would open at full volume for a moment.
				play(held);
				playing.set(carriage.id, held);
				await heard.play().catch(() => {});

				say.mediaHears(carriage.id);
			} catch (why) {
				console.error('VoxLoop could not hear a talker', why);
			}
		},

		/**
		 * A carriage this end already has is now heard on these loops: the talker armed or
		 * disarmed one this operator monitors, or the operator took one up or muted it. The
		 * stream is the same stream, so only the level can move.
		 */
		heardOn(carriage, heardOn) {
			const held = playing.get(carriage);
			if (!held) return;

			held.heardOn = heardOn;
			play(held);
		},

		/**
		 * The loops as the presence document now has them.
		 *
		 * **The level played and the level shown are read off the same document** (ADR-0007),
		 * so an operator who has just turned a loop down hears it go down when the card says
		 * it has — never before, because nothing here moves on the slider, and never after.
		 * The same is true of priority: the gain comes up when the mark does, because they
		 * are one fact arriving (ADR-0059).
		 */
		theLoopsAre(now) {
			loops = now;
			for (const held of playing.values()) play(held);
		},

		/**
		 * One loop's beacon, to count and never to play (ADR-0017).
		 *
		 * Built and then said to be built, like a talker's carriage and for the same reason:
		 * the server holds it paused until it hears, and a packet sent to an end that does not
		 * exist yet is one this end could never count.
		 */
		async oneMoreBeacon(beacon, on) {
			if (closed || !receiving) return;

			try {
				const carriage = await receiving.consume(beacon);
				counting.set(carriage.id, { carriage, on });

				say.mediaHears(carriage.id);
			} catch (why) {
				// Nothing is counted on a carriage that could not be built, so the loop reads
				// as not received — which is the truth about this end.
				console.error('VoxLoop could not count a loop’s beacon', why);
			}
		},

		/** That beacon's carriage is closed at the far end, and there is nothing left to count. */
		oneFewerBeacon(carriage) {
			const held = counting.get(carriage);
			if (!held) return;

			held.carriage.close();
			counting.delete(carriage);
		},

		/** One fewer. The carriage is closed at the far end and there is nothing left to play. */
		oneFewerTalker(carriage) {
			const held = playing.get(carriage);
			if (!held) return;

			held.heard.pause();
			held.heard.srcObject = null;
			held.carriage.close();
			playing.delete(carriage);
		},

		/**
		 * Key, or unkey.
		 *
		 * **This is the whole of keying at this end** (ADR-0008): the local track is enabled
		 * or disabled, and the server is told separately so that it can tell everybody else.
		 * Nothing here waits for the server and nothing here draws anything.
		 */
		keying(wants) {
			if (microphone) microphone.enabled = wants;
		},

		/** The session is over. Everything opened here goes with it. */
		close() {
			closed = true;
			stopCounting();
			for (const { carriage } of counting.values()) carriage.close();
			counting.clear();
			for (const { carriage, heard } of playing.values()) {
				heard.pause();
				heard.srcObject = null;
				carriage.close();
			}
			playing.clear();
			microphone?.stop();
			sending?.close();
			receiving?.close();
			microphone = null;
		}
	};
}

/**
 * How loud to play a talker heard on these loops, as a gain from 0 to 1.
 *
 * **Full gain where any of them carries a priority transmission**, whatever it is set to, and
 * the loudest volume among them otherwise (v1 §4, ADR-0045). Priority bypasses loudest-wins
 * rather than competing with it, so one marked loop is enough and nothing is compared.
 *
 * **Mute stays sovereign.** A muted loop is not an applicable one here any more than it is in
 * `loudest`, so a priority mark on it raises nothing: the mark still shows on the card, and the
 * audio does not arrive (ADR-0059).
 *
 * **The mark is the loop's, and so is the gain.** The console cannot tell one talker on a loop
 * from another (ADR-0033) and the priority attribute rides the presence document rather than
 * the media path (ADR-0045), so what arrives is *this loop carries a priority transmission*.
 * Anybody heard on that loop while the mark is up plays at full gain with it; nobody anywhere
 * plays quieter for it.
 *
 * @param {string[]} heardOn the ids of the loops this talker is heard on
 * @param {{ id: string, volume: number, muted: boolean, priority?: boolean }[]} loops as the
 *   document has them
 */
export function theGain(heardOn, loops) {
	const atPriority = heardOn.some((id) =>
		loops.some((reachable) => reachable.id === id && reachable.priority && !reachable.muted)
	);
	if (atPriority) return 1;

	return loudest(heardOn, loops);
}

/**
 * How loud to play a talker heard on these loops when none of them carries priority: **the
 * loudest volume among them** (v1 §5, ADR-0007).
 *
 * Volume is an attenuation control, so a transmission also going to a loop the operator kept
 * up is one they have already said they want to hear, and the loudest applicable volume wins.
 * Quietest-wins would let a suppressed loop silence a transmission they care about.
 *
 * **A muted loop is not an applicable one.** The server stops carrying a talker on a loop the
 * operator muted; until that lands, the document already says so, and a mute that is shown is
 * a mute that is heard. A talker heard only on muted loops therefore plays at nothing.
 *
 * **A loop the document does not describe plays at unity.** It is a carriage that arrived
 * before the document naming its loop, and guessing it down would risk the one failure this
 * rule exists to prevent: a transmission somebody wanted going unheard.
 *
 * @param {string[]} heardOn the ids of the loops this talker is heard on
 * @param {{ id: string, volume: number, muted: boolean }[]} loops as the document has them
 */
export function loudest(heardOn, loops) {
	if (heardOn.length === 0) return 1;

	let gain = 0;
	for (const id of heardOn) {
		const described = loops.find((reachable) => reachable.id === id);
		if (!described) return 1;
		if (!described.muted) gain = Math.max(gain, described.volume / 100);
	}

	return gain;
}

/**
 * How many packets have arrived on one carriage, from its receiver's statistics.
 *
 * **What arrived at this end's RTP receiver, and nothing else** (ADR-0017): the inbound stream's
 * `packetsReceived`. The report carries the transport and the codec beside it, and a count
 * taken off the transport would be every stream on the downlink rather than this one — which
 * is the talker's audio standing in for the beacon, the substitution ADR-0017 is written
 * against. A carriage nothing has arrived on yet has no inbound stream at all, and counts
 * nothing.
 *
 * @param {Map<string, { type: string, packetsReceived?: number }>} report a receiver's
 *   `RTCStatsReport`, or anything shaped like one
 */
export function packetsIn(report) {
	let packets = 0;
	for (const stat of report.values()) {
		if (stat.type === 'inbound-rtp') packets += stat.packetsReceived ?? 0;
	}

	return packets;
}

/**
 * One transport's connection state, on the ladder's own terms (ADR-0042).
 *
 * `disconnected` is `impaired` and `failed` is `lost`, which is the distinction this end
 * exists to make: mediasoup's server-side `iceState` has no `failed` at all and takes around
 * thirty seconds of consent freshness to say anything. Anything this browser has no name for
 * is read as `lost`, which is the safe direction — a console that cannot tell what its audio
 * path is doing has no business offering a key control over it.
 */
function onTheLadder(state) {
	if (state === 'connected') return 'connected';
	if (state === 'disconnected') return 'impaired';

	return 'lost';
}

/** The worse of two readings. Green needs both, red needs one. */
function worst(one, other) {
	const ladder = ['lost', 'impaired', 'connected'];

	return ladder.indexOf(one) < ladder.indexOf(other) ? one : other;
}

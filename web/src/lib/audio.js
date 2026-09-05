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
// Per-loop volume, loudest-wins and priority at full gain (#44, #45) turn the playback below
// into a mixer with a gain per stream. What is here is the part ADR-0007 actually requires:
// separate streams per talker, mixed at the client rather than at the server.

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
export function openAudio({ say, onMediaPath }) {
	const device = new Device();
	// One element per audible talker. The browser mixing several at once **is** the
	// client-side mixing ADR-0007 asks for; what #44 adds is a gain per stream, not a
	// different place for the mixing to happen.
	const playing = new Map();

	let uplink = null;
	let microphone = null;
	// The `produce` callback, held between asking the server to carry the uplink and being
	// told its name. mediasoup-client will not call a producer published until it has one.
	let waitingForTheUplink = null;
	let sending = null;
	let receiving = null;
	let closed = false;

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

		uplink = await sending.produce({
			track: microphone,
			codecOptions: HOW_IT_IS_ENCODED
		});
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
		async oneMoreTalker(talker) {
			if (closed || !receiving) return;

			try {
				const carriage = await receiving.consume(talker);
				const heard = new Audio();
				heard.autoplay = true;
				heard.srcObject = new MediaStream([carriage.track]);
				playing.set(carriage.id, { carriage, heard });
				await heard.play().catch(() => {});

				say.mediaHears(carriage.id);
			} catch (why) {
				console.error('VoxLoop could not hear a talker', why);
			}
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

		/** Whether there is a microphone to key at all, which the console says in words. */
		canBeHeard: () => Boolean(uplink),

		/** The session is over. Everything opened here goes with it. */
		close() {
			closed = true;
			for (const { carriage, heard } of playing.values()) {
				heard.pause();
				heard.srcObject = null;
				carriage.close();
			}
			playing.clear();
			microphone?.stop();
			sending?.close();
			receiving?.close();
			uplink = null;
			microphone = null;
		}
	};
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

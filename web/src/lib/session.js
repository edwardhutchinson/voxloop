// The client's Session module: the socket, what is said on it, and the documents that arrive.
//
// This and `server.js` are the two halves of the client's Session module (`modules.md`):
// what is asked for over HTTP, and what travels on the socket. Nothing else in the console
// talks to VoxLoop over the signalling channel. One socket per tab, opened at sign-in
// (ADR-0054), and it is the only channel live state travels on —
// which is what makes the state on screen one thing that was simultaneously true rather
// than several that were each true at some point (ADR-0019).
//
// A document that arrives **replaces** what is on screen and is never merged into it. That
// is the whole reason it is sent whole and versioned: a merge is how a console ends up
// showing a role's occupants as of one moment beside its limit as of another, and each half
// being true is exactly what makes the combination a lie.
//
// **A tab is at one of two tiers and the server says which.** It opens in the lobby and is
// sent the lobby document; assuming a role moves it to a session and it is sent the presence
// document instead. Neither is inferred here — the console renders whichever document last
// arrived, because the server is the only thing entitled to say whether somebody holds a
// role.
//
// **One state on this page is measured here rather than pushed**, and it is the only one:
// where this tab stands with the channel itself (ADR-0018). The one thing a server cannot do
// to a console it has lost is tell it that it has been lost, so the console counts the
// heartbeats it is not getting. The ladder is `connection.js`'s; this is where the heartbeats
// arrive and are answered.
//
// Resuming a session by name, and the gap events that come with it, are still to come (#50).
//
// **Not everything a tab says is a person saying it.** A media path report is this client
// noticing something about its own transport, and the server does not count it towards the
// window that reaps sign-ins nobody is sitting at (v1 §2) — which is why the acts a person
// performs and the one the machine performs sit side by side below without being written the
// same way.

import {
	CONFIRMED,
	DISCONNECTED,
	SETTLES_EVERY,
	UNCONFIRMED,
	theConnection,
	worse
} from './connection.js';

// **The ladder's words are the Session module's, and this is the way in to them** — the same
// rule that makes `$lib/input` the only way into Input (`modules.md`, ADR-0061). The console
// renders the rungs and merges the server's reading with this tab's, so it needs the
// vocabulary; what it must not need is the file that runs the clock.
export { CONFIRMED, DISCONNECTED, UNCONFIRMED, worse };

const HELLO = JSON.stringify({ message: 'hello' });

/** Where the signalling channel is, on whatever the console was itself served from. */
function where() {
	const { protocol, host } = window.location;

	return `${protocol === 'https:' ? 'wss' : 'ws'}://${host}/api/signalling`;
}

/**
 * Open the signalling channel and start listening.
 *
 * Six things can happen to it and the console shows a different thing for each, because
 * they are different facts about the deployment rather than six shades of *offline*:
 *
 * - `onLobby(document)` — the lobby, whole, to be rendered as it stands.
 * - `onPresence(document)` — the presence document, whole, for the session this tab holds.
 * - `onSessionEnded(reason)` — the role is given up and audio has stopped, and this is why.
 *   It arrives **before** the lobby that follows it, so the console can say what happened
 *   rather than merely reappearing somewhere else.
 * - `onRefused(reason)` — the server would not do that, and said what was not met. **A
 *   refusal is not the end of anything**: the socket stands, and the next message is judged
 *   on its own (ADR-0054).
 * - `onEnded(reason)` — the server said why it is going. The sign-in is over.
 * - `onLost()` — the channel went away without saying anything. Nothing has ended; the
 *   console simply cannot see any more, and says so rather than blanking.
 * - `onConnection({ state, since, aLatchStands })` — where this tab stands with the channel,
 *   how long ago it was last confirmed, and whether a latched emission may still stand
 *   (ADR-0018). It is the one thing here the server did not say.
 *
 * Five more arrive for the Audio module rather than for the console, and they are **not
 * documents**: they carry the client's own media negotiation, which VoxLoop owns the channel
 * for and has no opinion about (ADR-0006). Nothing on screen comes out of them — what the
 * console draws about the audio path is `media_path` in the presence document.
 *
 * - `onPathToBuild(path)` — what this session's media library has to build.
 * - `onUplinkCarried(carriage)` — the uplink is carried, under this name.
 * - `onOneMoreTalker(talker, heardOn)` — one more talker to hear, what to build to hear them,
 *   and which of this session's own loops they are heard on. **It names nobody** (ADR-0033),
 *   and there is no field in it that could; the loops are ones this session monitors, so it
 *   says nothing about where else the talker went (ADR-0057).
 * - `onHeardOn(carriage, heardOn)` — a carriage this tab already has is now heard on these
 *   loops. The stream did not change; the loudest volume among them may have.
 * - `onOneFewerTalker(carriage)` — that carriage is closed at the server's end.
 * - `onOneMoreBeacon(beacon, on)` — one loop's beacon, what to build to count it, and the loop
 *   it measures. **A message of its own and never one more talker** (ADR-0017): a beacon is
 *   counted and never played, a talker played and never counted, and neither has to be told
 *   apart from the other by looking at it.
 * - `onOneFewerBeacon(carriage)` — that beacon's carriage is closed at the server's end.
 *
 * Answers with the acts a tab can perform on its own session, and the way to close it.
 */
export function openSignalling({
	onLobby,
	onPresence,
	onSessionEnded,
	onRefused,
	onEnded,
	onLost,
	onConnection = () => {},
	onPathToBuild,
	onUplinkCarried,
	onOneMoreTalker,
	onHeardOn = () => {},
	onOneFewerTalker,
	onOneMoreBeacon = () => {},
	onOneFewerBeacon = () => {},
	// What runs the ladder's clock: it is handed an act and an interval and answers with the
	// way to stop. It is a parameter rather than a reach for `setInterval` so that nothing in
	// here touches a global — the console is rendered on the server at build time, and a
	// timer started there would be one nobody ever clears.
	ticking = (act, every) => {
		const timer = setInterval(act, every);

		return () => clearInterval(timer);
	}
}) {
	const socket = new WebSocket(where());
	const channel = theConnection({ onConnection });
	let stopTicking = null;
	// A reason arrives before the close does, and a console that showed both would tell the
	// operator their sign-in ended and then that the network did.
	let told = false;

	socket.addEventListener('open', () => {
		// The clock starts at the open rather than at the first heartbeat: a console with a
		// socket it has just opened has not missed anything, and measuring from nothing would
		// have it start the shift a rung up the ladder.
		channel.opened(Date.now());
		stopTicking = ticking(() => channel.settle(Date.now()), SETTLES_EVERY);
		socket.send(HELLO);
	});

	socket.addEventListener('message', (event) => {
		const said = read(event.data);

		if (said?.message === 'heartbeat') {
			// **Answered rather than merely counted.** Both ends measure the same gap from
			// opposite sides, which is what lets this tab withdraw its own push-to-talk at the
			// moment the server closes its fan-out.
			//
			// **Only a heartbeat confirms the channel**, and not the documents arriving beside
			// it at five a second. Measuring off *anything arrived* would make the ladder a
			// function of how busy the deployment is, so a quiet console would freeze where a
			// busy one would not — and the rung would stop meaning what it says.
			say(socket, { message: 'heartbeat' });
			channel.confirmed(Date.now(), said.ladder);
		} else if (said?.message === 'lobby') {
			onLobby(said);
		} else if (said?.message === 'presence') {
			onPresence(said);
		} else if (said?.message === 'session-ended') {
			onSessionEnded(said.reason ?? null);
		} else if (said?.message === 'refused') {
			// Somebody may not do something. That is a fact about one message and not about
			// the sign-in: reading it as *you are signed out* would take an operator off a
			// console over a message they were never entitled to send in the first place.
			onRefused(said.reason ?? null);
		} else if (said?.message === 'closing') {
			told = true;
			onEnded(said.reason ?? null);
		} else if (said?.message === 'a-path-to-build') {
			onPathToBuild(said.path);
		} else if (said?.message === 'the-uplink-is-carried') {
			onUplinkCarried(said.carriage);
		} else if (said?.message === 'one-more-talker') {
			onOneMoreTalker(said.talker, said.heard_on ?? []);
		} else if (said?.message === 'heard-on') {
			onHeardOn(said.carriage, said.heard_on ?? []);
		} else if (said?.message === 'one-fewer-talker') {
			onOneFewerTalker(said.carriage);
		} else if (said?.message === 'one-more-beacon') {
			onOneMoreBeacon(said.beacon, said.on);
		} else if (said?.message === 'one-fewer-beacon') {
			onOneFewerBeacon(said.carriage);
		}
	});

	socket.addEventListener('close', () => {
		// **A socket that closed is a fact rather than a silence**, so the ladder is skipped
		// and this tab is disconnected at once. The rungs are for the gap nobody reported.
		stopTicking?.();
		stopTicking = null;
		channel.gone(Date.now());
		if (!told) onLost();
	});

	return {
		/** Take up a role. The server answers with the presence document, or with a refusal. */
		assume: (role) => say(socket, { message: 'assume', role }),
		/**
		 * Give it up. **A full stop rather than a transition** (v1 §2): the server answers
		 * with why the session ended and then with the lobby, and nothing here pretends the
		 * two are one thing.
		 */
		relinquish: () => say(socket, { message: 'relinquish' }),
		/**
		 * Monitor a loop, or stop monitoring it.
		 *
		 * **Two acts rather than one toggle**, and the console picks which by reading the
		 * document it last received. Optimistic rendering is banned (ADR-0016) so the card
		 * lags the click; a second click on a card that has not caught up yet says the same
		 * thing twice and lands on the same state, where a toggle would undo the first and
		 * leave somebody off a loop they had just taken up.
		 *
		 * The server answers with the presence document, or with a refusal where the role no
		 * longer holds `monitor` on that loop. **Nothing here renders off what it just
		 * said**: this tab asks, and then reads the document like everything else.
		 */
		subscribe: (heldOn) => say(socket, { message: 'subscribe', loop: heldOn }),
		unsubscribe: (heldOn) => say(socket, { message: 'unsubscribe', loop: heldOn }),
		/**
		 * Arm a loop as a destination for this session's voice, or disarm it.
		 *
		 * **Two acts rather than one toggle**, and independent of monitoring in both
		 * directions (ADR-0013): arming puts a loop in nobody's ears and monitoring makes no
		 * destination. The server refuses a loop this role may not emit on, and **that
		 * refusal is the whole of the enforcement** — the fan-out is built from the arm set,
		 * so a loop that never got past it has no route (ADR-0008).
		 *
		 * Arming and disarming are instant and cost no renegotiation: the uplink already
		 * exists and does not address, so the change is a routing one at the server
		 * (ADR-0007).
		 */
		arm: (heldOn) => say(socket, { message: 'arm', loop: heldOn }),
		disarm: (heldOn) => say(socket, { message: 'disarm', loop: heldOn }),
		/**
		 * Say that this client is transmitting, or that it has stopped.
		 *
		 * **The client has already keyed by the time this is sent** (ADR-0008). The local
		 * track went live first, because that is what buys key-to-first-audio under 100 ms;
		 * this is the signal, and the server is the sole authority for telling anybody —
		 * including this operator, whose own transmitting lamp lights on the document that
		 * comes back and never on their own button going down.
		 */
		key: () => say(socket, { message: 'key' }),
		unkey: () => say(socket, { message: 'unkey' }),
		/**
		 * Say that this client's transmission is at priority, or that it no longer is.
		 *
		 * **The priority level, and nothing else** (ADR-0046). Whether the client is keying at
		 * all is said above, as the OR of every level, so this never keys anything on its own.
		 * It carries no loop, because **priority applies to the whole arm set** (ADR-0045): one
		 * stream fanned out at the server cannot be priority on one armed loop and ordinary on
		 * another. The server marks every loop it lands on and audits the press; the document
		 * that comes back is what says it happened.
		 */
		keyPriority: () => say(socket, { message: 'key-priority' }),
		unkeyPriority: () => say(socket, { message: 'unkey-priority' }),
		/**
		 * Silence a loop in this operator's own ears, or hear it again.
		 *
		 * **Not an unsubscribe** (v1 §5): the loop stays monitored, so its talking indicator
		 * keeps arriving, and nobody else on it is touched. **Two acts rather than one toggle**
		 * for the reason subscribe and unsubscribe are. The server answers with the document,
		 * and the card shows the mute when that says so.
		 */
		mute: (heldOn) => say(socket, { message: 'mute', loop: heldOn }),
		unmute: (heldOn) => say(socket, { message: 'unmute', loop: heldOn }),
		/**
		 * Set how loud a loop plays in this operator's ears, as a percentage of full volume.
		 *
		 * **One act with a level rather than two**, because a volume is not a toggle: saying
		 * the same level twice lands on the same state. It is personalisation, remembered per
		 * (user, role, loop) as the server applies it (ADR-0050), and nothing here renders off
		 * it — the level on the card is the document's.
		 */
		setVolume: (heldOn, volume) => say(socket, { message: 'set-volume', loop: heldOn, volume }),
		/**
		 * Say that this operator is not in the chair, or that they are back.
		 *
		 * **The one asserted state in the product, and it is only ever said by hand** (ADR-0016).
		 * Nothing here infers it: there is no idle timer, no `mousemove` listener and no focus
		 * handler anywhere in the console, because an operator watching telemetry is idle at the
		 * keyboard and very much on console.
		 *
		 * **Coming back is not a special act.** Any deliberate act clears the claim at the server
		 * — keying, a subscription, an arm, answering a prompt, dismissing a banner — and
		 * `onConsole` is here for somebody who has come back and has nothing else to do yet. The
		 * claim goes when the document says it has, like everything else on the page.
		 */
		offConsole: () => say(socket, { message: 'off-console' }),
		onConsole: () => say(socket, { message: 'on-console' }),
		/**
		 * The four halves of the client's own media negotiation, carried and never read here.
		 *
		 * They are the Audio module's, and this file's only part in them is that they go on
		 * the one authorised channel rather than on a second one of their own (ADR-0006).
		 */
		mediaCanDecode: (whatItCanDecode) =>
			say(socket, { message: 'media-can-decode', what_it_can_decode: whatItCanDecode }),
		mediaConnect: (way, keys) => say(socket, { message: 'media-connect', way, keys }),
		mediaSpeaks: (whatItIsSending) =>
			say(socket, { message: 'media-speaks', what_it_is_sending: whatItIsSending }),
		mediaHears: (carriage) => say(socket, { message: 'media-hears', carriage }),
		/**
		 * Say where this tab's media path stands: `connected`, `impaired` or `lost`.
		 *
		 * **The client drives this ladder** (ADR-0042) because it is the end that can tell a
		 * transient `RTCPeerConnection` `disconnected` from a terminal `failed`, which the
		 * server cannot do in time. `disconnected` is `impaired` and `failed` is `lost`.
		 *
		 * The server merges this with its own end pessimistically — green needs both, red
		 * needs one — and pushes the answer back in the presence document. **Nothing here
		 * renders off what it just said**: this tab reports, and then reads the document like
		 * everything else (ADR-0016).
		 *
		 * The peer connection that drives it is the Audio module's, and it is not built yet.
		 */
		mediaPath: (state) => say(socket, { message: 'media-path', state }),
		/**
		 * Say how many packets of each loop's beacon this tab has counted, as running totals
		 * by loop id.
		 *
		 * **The client counts and the server judges** (ADR-0017): nothing here concludes that
		 * a loop is received or lost, because a client that could conclude it could also be
		 * wrong about it, and a wedged one would say nothing at all — which the server reads as
		 * receiving nothing, the safe way round. Like the media path report, this is the
		 * machine noticing something about its own transport, and it renews no sign-in.
		 */
		beaconsCounted: (counted) => say(socket, { message: 'beacons-counted', counted }),
		close: () => {
			told = true;
			stopTicking?.();
			stopTicking = null;
			socket.close();
		}
	};
}

/** Say one thing, where the socket is still open to say it on. */
function say(socket, message) {
	if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify(message));
}

/** What the server said, or nothing at all if it was not something this console reads. */
function read(said) {
	try {
		return JSON.parse(said);
	} catch {
		return null;
	}
}

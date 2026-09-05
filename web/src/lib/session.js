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
// Resuming a session by name, and the gap events that come with it, are still to come (#50).
//
// **Not everything a tab says is a person saying it.** A media path report is this client
// noticing something about its own transport, and the server does not count it towards the
// window that reaps sign-ins nobody is sitting at (v1 §2) — which is why the acts a person
// performs and the one the machine performs sit side by side below without being written the
// same way.

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
 *
 * Four more arrive for the Audio module rather than for the console, and they are **not
 * documents**: they carry the client's own media negotiation, which VoxLoop owns the channel
 * for and has no opinion about (ADR-0006). Nothing on screen comes out of them — what the
 * console draws about the audio path is `media_path` in the presence document.
 *
 * - `onPathToBuild(path)` — what this session's media library has to build.
 * - `onUplinkCarried(carriage)` — the uplink is carried, under this name.
 * - `onOneMoreTalker(talker)` — one more talker to hear, and what to build to hear them. **It
 *   names nobody** (ADR-0033), and there is no field in it that could.
 * - `onOneFewerTalker(carriage)` — that carriage is closed at the server's end.
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
	onPathToBuild,
	onUplinkCarried,
	onOneMoreTalker,
	onOneFewerTalker
}) {
	const socket = new WebSocket(where());
	// A reason arrives before the close does, and a console that showed both would tell the
	// operator their sign-in ended and then that the network did.
	let told = false;

	socket.addEventListener('open', () => socket.send(HELLO));

	socket.addEventListener('message', (event) => {
		const said = read(event.data);

		if (said?.message === 'lobby') {
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
			onOneMoreTalker(said.talker);
		} else if (said?.message === 'one-fewer-talker') {
			onOneFewerTalker(said.carriage);
		}
	});

	socket.addEventListener('close', () => {
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
		close: () => {
			told = true;
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

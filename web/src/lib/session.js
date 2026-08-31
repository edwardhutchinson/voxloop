// The client's Session module: the socket, and the documents that arrive on it.
//
// This and `server.js` are the two halves of the client's Session module (`modules.md`):
// what is asked for over HTTP, and what arrives on the socket. Nothing else in the console
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
// Assuming a role, resuming a session, and everything the presence document carries are
// still to come; what this opens is the lobby, and the lobby has no acts in it.

const HELLO = JSON.stringify({ message: 'hello' });

/** Where the signalling channel is, on whatever the console was itself served from. */
function where() {
	const { protocol, host } = window.location;

	return `${protocol === 'https:' ? 'wss' : 'ws'}://${host}/api/signalling`;
}

/**
 * Open the signalling channel and start listening.
 *
 * Four things can happen to it and the console shows a different thing for each, because
 * they are different facts about the deployment rather than four shades of *offline*:
 *
 * - `onLobby(document)` — the lobby, whole, to be rendered as it stands.
 * - `onRefused(reason)` — the server would not do that, and said what was not met. **A
 *   refusal is not the end of anything**: the socket stands, and the next message is judged
 *   on its own (ADR-0054).
 * - `onEnded(reason)` — the server said why it is going. The sign-in is over.
 * - `onLost()` — the channel went away without saying anything. Nothing has ended; the
 *   console simply cannot see any more, and says so rather than blanking.
 *
 * Answers with the way to close it, which is what a tab does on its way out.
 */
export function openSignalling({ onLobby, onRefused, onEnded, onLost }) {
	const socket = new WebSocket(where());
	// A reason arrives before the close does, and a console that showed both would tell the
	// operator their sign-in ended and then that the network did.
	let told = false;

	socket.addEventListener('open', () => socket.send(HELLO));

	socket.addEventListener('message', (event) => {
		const said = read(event.data);

		if (said?.message === 'lobby') {
			onLobby(said);
		} else if (said?.message === 'refused') {
			// Somebody may not do something. That is a fact about one message and not about
			// the sign-in: reading it as *you are signed out* would take an operator off a
			// console over a message they were never entitled to send in the first place.
			onRefused(said.reason ?? null);
		} else if (said?.message === 'closing') {
			told = true;
			onEnded(said.reason ?? null);
		}
	});

	socket.addEventListener('close', () => {
		if (!told) onLost();
	});

	return () => {
		told = true;
		socket.close();
	};
}

/** What the server said, or nothing at all if it was not something this console reads. */
function read(said) {
	try {
		return JSON.parse(said);
	} catch {
		return null;
	}
}

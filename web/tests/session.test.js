// The client's Session module: what it says to the server, and what it tells the console.
//
// It is the one thing in the console that talks over the signalling channel, so the four
// things that can happen to a socket are four things the rest of the console never has to
// know how to spell. They are tested here rather than through a component because they are
// facts about the protocol, not about the page.

import assert from 'node:assert/strict';
import test, { afterEach, beforeEach } from 'node:test';

import { openSignalling } from '../src/lib/session.js';

/** A socket the test holds both ends of. */
class ASocket {
	static opened = [];

	static OPEN = 1;

	constructor(url) {
		this.url = url;
		this.sent = [];
		this.closed = false;
		this.listeners = {};
		this.readyState = ASocket.OPEN;
		ASocket.opened.push(this);
	}

	addEventListener(event, listener) {
		this.listeners[event] = listener;
	}

	send(said) {
		this.sent.push(said);
	}

	close() {
		this.closed = true;
		this.happens('close');
	}

	/** The server's side of it: what a real socket would deliver to the console. */
	happens(event, data) {
		this.listeners[event]?.({ data });
	}

	says(message) {
		this.happens('message', JSON.stringify(message));
	}
}

/** Everything the console was told, in the order it was told it. */
function listening() {
	const told = [];

	return {
		told,
		onLobby: (lobby) => told.push(['lobby', lobby]),
		onPresence: (presence) => told.push(['presence', presence]),
		onSessionEnded: (reason) => told.push(['session-ended', reason]),
		onRefused: (reason) => told.push(['refused', reason]),
		onEnded: (reason) => told.push(['ended', reason]),
		onLost: () => told.push(['lost']),
		// The four halves of the client's own media negotiation. They go to Audio rather than
		// to the console, and nothing on screen comes out of them.
		onPathToBuild: (path) => told.push(['a-path-to-build', path]),
		onUplinkCarried: (carriage) => told.push(['the-uplink-is-carried', carriage]),
		onOneMoreTalker: (talker, heardOn) => told.push(['one-more-talker', talker, heardOn]),
		onHeardOn: (carriage, heardOn) => told.push(['heard-on', carriage, heardOn]),
		onOneFewerTalker: (carriage) => told.push(['one-fewer-talker', carriage])
	};
}

beforeEach(() => {
	ASocket.opened = [];
	globalThis.WebSocket = ASocket;
	globalThis.window = { location: { protocol: 'https:', host: 'voxloop.example:8443' } };
});

afterEach(() => {
	// The connection ladder runs on a real interval, and a test that opened a socket and
	// walked away would leave one ticking for the life of the process. Closing the socket is
	// what a tab does and it is what stops it — so this is the tab being closed rather than a
	// cleanup this file invented.
	for (const socket of ASocket.opened) if (!socket.closed) socket.close();
});

const lastSocket = () => ASocket.opened.at(-1);

/** A console listening on an open channel, and the socket the server would talk on. */
function openTold() {
	const page = listening();
	openSignalling(page);
	const socket = lastSocket();
	socket.happens('open');

	return { ...page, socket };
}

test('opens the signalling channel on whatever the console was served from', () => {
	openSignalling(listening());

	assert.equal(lastSocket().url, 'wss://voxloop.example:8443/api/signalling');
});

test('an unencrypted deployment is not a different console', () => {
	globalThis.window.location.protocol = 'http:';

	openSignalling(listening());

	assert.equal(lastSocket().url, 'ws://voxloop.example:8443/api/signalling');
});

test('says hello, so the server answers rather than pushing at a tab that is not ready', () => {
	openSignalling(listening());

	lastSocket().happens('open');

	assert.deepEqual(lastSocket().sent, ['{"message":"hello"}']);
});

test('hands the lobby out whole, as it arrived', () => {
	const page = listening();
	openSignalling(page);
	const lobby = { message: 'lobby', version: 1, roles: [{ id: 'a', name: 'CAPCOM' }] };

	lastSocket().says(lobby);

	assert.deepEqual(page.told, [['lobby', lobby]]);
});

test('a sign-in that has ended is said to have ended, with the reason the server gave', () => {
	const page = listening();
	openSignalling(page);

	lastSocket().says({ message: 'closing', reason: 'That sign-in has ended.' });

	assert.deepEqual(page.told, [['ended', 'That sign-in has ended.']]);
});

// A refusal is a fact about one message, not about the sign-in behind it: every message on
// this socket is judged on its own (ADR-0054), so reading one as a sign-out would take an
// operator off a console over a message they were never entitled to send.
test('a refusal is about the message, and ends nothing', () => {
	const page = listening();
	openSignalling(page);

	lastSocket().says({
		message: 'refused',
		was: 'hello',
		reason: 'That message is for a signed-in user.'
	});

	assert.deepEqual(page.told, [['refused', 'That message is for a signed-in user.']]);
});

test('anything the server says that this console has no reading of is not acted on', () => {
	const page = listening();
	openSignalling(page);

	lastSocket().says({ message: 'something-from-a-later-ticket', version: 4 });

	assert.deepEqual(page.told, []);
});

// Losing the channel and being told the sign-in is over are different facts, and the console
// shows a different thing for each. A close arrives after a reason as well, so a console that
// took both would tell the operator their sign-in ended and then that the network did.
test('a channel that goes away without saying why is lost rather than ended', () => {
	const page = listening();
	openSignalling(page);

	lastSocket().happens('close');

	assert.deepEqual(page.told, [['lost']]);
});

test('a channel that said why is not also reported lost', () => {
	const page = listening();
	openSignalling(page);

	lastSocket().says({ message: 'closing', reason: 'That sign-in has ended.' });
	lastSocket().happens('close');

	assert.deepEqual(page.told, [['ended', 'That sign-in has ended.']]);
});

test('a tab on its way out closes the socket and reports nothing', () => {
	const page = listening();

	openSignalling(page).close();

	assert.equal(lastSocket().closed, true);
	assert.deepEqual(page.told, []);
});

test('hands the presence document out whole, as it arrived', () => {
	const page = listening();
	openSignalling(page);
	const presence = {
		message: 'presence',
		version: 3,
		session: 'a-session',
		role: { id: 'a', name: 'Flight Director' },
		loops: [{ id: 'b', name: 'Air-to-ground', permission: 'emit', subscribed: true }]
	};

	lastSocket().says(presence);

	assert.deepEqual(page.told, [['presence', presence]]);
});

// **A session ending is not a sign-in ending, and neither is a lost channel.** Three ways to
// end up with no console in front of you, and the operator is owed the difference: one is
// over, one is somebody else's act, and one is the network.
test('a session that ended is said to have ended, and the sign-in is untouched', () => {
	const page = listening();
	openSignalling(page);

	lastSocket().says({
		message: 'session-ended',
		reason: 'You relinquished the role. Audio has stopped.'
	});

	assert.deepEqual(page.told, [['session-ended', 'You relinquished the role. Audio has stopped.']]);
});

test('taking up a role and giving it up are the two things a tab says about its session', () => {
	const channel = openSignalling(listening());
	lastSocket().happens('open');

	channel.assume('a-role');
	channel.relinquish();

	assert.deepEqual(lastSocket().sent, [
		'{"message":"hello"}',
		'{"message":"assume","role":"a-role"}',
		'{"message":"relinquish"}'
	]);
});

// **Two acts rather than one toggle** (ADR-0016). The card lags the click, so a second click
// on one that has not caught up says the same thing twice and lands on the same state; a
// toggle would undo the first and leave somebody off a loop they had just taken up. Which of
// the two a click is comes from the document, one level up.
test('taking a loop up and dropping it are two things a tab says, not one', () => {
	const channel = openSignalling(listening());
	lastSocket().happens('open');

	channel.subscribe('a-loop');
	channel.unsubscribe('a-loop');

	assert.deepEqual(lastSocket().sent.slice(1), [
		'{"message":"subscribe","loop":"a-loop"}',
		'{"message":"unsubscribe","loop":"a-loop"}'
	]);
});

// Nothing renders off what a tab just said (ADR-0016). The card changes when the presence
// document says it has, which is the whole reason the click visibly lags.
test('taking a loop up tells the console nothing on its own', () => {
	const page = listening();
	const channel = openSignalling(page);
	lastSocket().happens('open');

	channel.subscribe('a-loop');

	assert.deepEqual(page.told, []);
});

// **The client drives the media path ladder** (ADR-0042), so this is the message that ends
// up merged with the server's own reading. It is the machine talking rather than the person
// — the server does not count it towards the window that reaps sign-ins nobody is sitting at
// — and it is said on the one channel live state travels on, like everything else.
test('a tab says where its own media path stands, in the ladder’s own words', () => {
	const channel = openSignalling(listening());
	lastSocket().happens('open');

	channel.mediaPath('impaired');
	channel.mediaPath('lost');

	assert.deepEqual(lastSocket().sent.slice(1), [
		'{"message":"media-path","state":"impaired"}',
		'{"message":"media-path","state":"lost"}'
	]);
});

// Reporting is not rendering (ADR-0016). What a tab says about its own transport is one of
// two readings the server merges pessimistically, and the console shows the answer that
// comes back in the presence document rather than the half it just sent.
test('reporting a media path tells the console nothing on its own', () => {
	const page = listening();
	const channel = openSignalling(page);
	lastSocket().happens('open');

	channel.mediaPath('lost');

	assert.deepEqual(page.told, []);
});

// A socket that has gone is not somewhere to shout into. Nothing is queued either: an assume
// that arrived after a reconnection would be a role taken up seconds after somebody asked
// for it, on a console they may have walked away from.
// **Two acts rather than one toggle**, for the reason subscribe and unsubscribe are, and a
// wholly separate pair from them: arming a loop puts it in nobody's ears and monitoring one
// makes no destination (ADR-0013).
test('arming a loop and disarming it are two things a tab says, not one', () => {
	const channel = openSignalling(listening());
	lastSocket().happens('open');

	channel.arm('a-loop');
	channel.disarm('a-loop');

	assert.deepEqual(lastSocket().sent.slice(1), [
		'{"message":"arm","loop":"a-loop"}',
		'{"message":"disarm","loop":"a-loop"}'
	]);
});

// **The key is a signal rather than a request** (ADR-0008): the client has already keyed by
// the time this goes, which is what buys key-to-first-audio under 100 ms. It carries no loop,
// because the uplink transmits and does not address (ADR-0007) — what it may reach was
// settled when the arms were made.
test('keying says so and names no loop', () => {
	const channel = openSignalling(listening());
	lastSocket().happens('open');

	channel.key();
	channel.unkey();

	assert.deepEqual(lastSocket().sent.slice(1), ['{"message":"key"}', '{"message":"unkey"}']);
});

// **Priority is a second level, said on its own** (ADR-0046), and it names no loop either:
// priority applies to the whole arm set because there is one stream (ADR-0045), so there is no
// loop it could be said of.
test('keying priority says so and names no loop', () => {
	const channel = openSignalling(listening());
	lastSocket().happens('open');

	channel.keyPriority();
	channel.unkeyPriority();

	assert.deepEqual(lastSocket().sent.slice(1), [
		'{"message":"key-priority"}',
		'{"message":"unkey-priority"}'
	]);
});

// **Nothing renders off what a tab just said** (ADR-0016), and the transmitting lamp is the
// sharpest case of it: it is lit by the server's acknowledgement coming back in the presence
// document, never by the button going down (ADR-0008).
test('keying tells the console nothing on its own', () => {
	const page = listening();
	const channel = openSignalling(page);
	lastSocket().happens('open');

	channel.key();
	channel.arm('a-loop');

	assert.deepEqual(page.told, []);
});

// **VoxLoop owns the signalling and has no opinion about this part of it** (ADR-0006). The
// four messages carry what one media library says to another, and this file's only part in
// them is that they go on the one authorised channel rather than on a second one.
test('the client’s own media negotiation goes out on the one channel, untouched', () => {
	const channel = openSignalling(listening());
	lastSocket().happens('open');

	channel.mediaCanDecode({ codecs: ['opus'] });
	channel.mediaConnect('up', { fingerprints: [] });
	channel.mediaSpeaks({ rtpParameters: { codecs: [] } });
	channel.mediaHears('a-carriage');

	assert.deepEqual(lastSocket().sent.slice(1).map(JSON.parse), [
		{ message: 'media-can-decode', what_it_can_decode: { codecs: ['opus'] } },
		{ message: 'media-connect', way: 'up', keys: { fingerprints: [] } },
		{ message: 'media-speaks', what_it_is_sending: { rtpParameters: { codecs: [] } } },
		{ message: 'media-hears', carriage: 'a-carriage' }
	]);
});

// The other direction, and the same rule: what arrives is handed on whole, to Audio rather
// than to the console. **Nothing that arrives says who is talking** (ADR-0033) and there is
// nothing here that would know what to do with it if it did.
test('what the media plane says is handed on whole, and none of it is a document', () => {
	const page = openTold();

	page.socket.says({ message: 'a-path-to-build', path: { router: {}, up: {}, down: {} } });
	page.socket.says({ message: 'the-uplink-is-carried', carriage: 'an-uplink' });
	page.socket.says({
		message: 'one-more-talker',
		talker: { id: 'a-carriage' },
		heard_on: ['l-flight']
	});
	page.socket.says({
		message: 'heard-on',
		carriage: 'a-carriage',
		heard_on: ['l-flight', 'l-sim']
	});
	page.socket.says({ message: 'one-fewer-talker', carriage: 'a-carriage' });

	assert.deepEqual(page.told, [
		['a-path-to-build', { router: {}, up: {}, down: {} }],
		['the-uplink-is-carried', 'an-uplink'],
		['one-more-talker', { id: 'a-carriage' }, ['l-flight']],
		['heard-on', 'a-carriage', ['l-flight', 'l-sim']],
		['one-fewer-talker', 'a-carriage']
	]);
});

// **Mute is two acts and volume is one**, and none of them is rendered off here (ADR-0016):
// the card shows a loop muted, or turned down, when the document says so. A mute is two
// because it is a toggle whose control lags the click; a volume is one because it is a level,
// and saying the same level twice lands on the same state.
test('muting, unmuting and setting a volume are what a tab says, and it renders none of them', () => {
	const page = listening();
	const channel = openSignalling(page);
	lastSocket().happens('open');

	channel.mute('a-loop');
	channel.unmute('a-loop');
	channel.setVolume('a-loop', 40);

	assert.deepEqual(lastSocket().sent.slice(1), [
		'{"message":"mute","loop":"a-loop"}',
		'{"message":"unmute","loop":"a-loop"}',
		'{"message":"set-volume","loop":"a-loop","volume":40}'
	]);
	assert.deepEqual(page.told, []);
});

test('nothing is said on a socket that is not open', () => {
	const channel = openSignalling(listening());
	lastSocket().happens('open');
	lastSocket().readyState = 3;

	channel.assume('a-role');

	assert.deepEqual(lastSocket().sent, ['{"message":"hello"}']);
});

test('anything unreadable on the wire is not read as a document', () => {
	const page = listening();
	openSignalling(page);

	lastSocket().happens('message', 'not json at all');

	assert.deepEqual(page.told, []);
});

// ---- #43: the heartbeat and the connection ladder -----------------------------------------

// The ladder is a clock, so these hold the clock. `Date.now` is what the module reads and the
// interval is what asks it, and both are the test's here — nothing sleeps and nothing waits.
const theRealNow = Date.now;
let now = 0;

/** A console on an open channel, with the ladder's clock in this test's hand. */
function openWatching() {
	const standing = [];
	let tick = null;

	now = 0;
	Date.now = () => now;

	const channel = openSignalling({
		...listening(),
		onConnection: (said) => standing.push(said),
		ticking: (act) => {
			tick = act;

			return () => {
				tick = null;
			};
		}
	});
	const socket = lastSocket();
	socket.happens('open');

	return {
		channel,
		socket,
		standing,
		last: () => standing.at(-1),
		/** Move the clock to `when` and let the console look at its watch. */
		at: (when) => {
			now = when;
			tick?.();
		},
		ticking: () => tick !== null
	};
}

afterEach(() => {
	Date.now = theRealNow;
});

// **Answered rather than merely counted.** Both ends measure the same gap from opposite
// sides, which is what lets a tab withdraw its own push-to-talk at the moment the server
// closes its fan-out (ADR-0018).
test('a heartbeat is answered, so the server measures the same gap from its side', () => {
	const page = openWatching();

	page.socket.says({ message: 'heartbeat', ladder: THE_LADDER });

	assert.deepEqual(page.socket.sent, ['{"message":"hello"}', '{"message":"heartbeat"}']);
});

// It is not a document and nothing on the page comes out of it. What the console draws about
// the channel is where the ladder stands, which is worked out here.
test('a heartbeat reaches nothing the console renders', () => {
	const page = listening();
	openSignalling(page);
	lastSocket().happens('open');

	lastSocket().says({ message: 'heartbeat', ladder: THE_LADDER });

	assert.deepEqual(page.told, []);
});

// **The clock is the deployment's** (v1 §7), and it arrives on every heartbeat rather than
// once — so a console that missed one still runs the site's ladder rather than the spec's.
test('the console runs the ladder the heartbeat carried', () => {
	const page = openWatching();

	page.socket.says({
		message: 'heartbeat',
		ladder: {
			heartbeat_ms: 3000,
			unconfirmed_ms: 8000,
			latch_dropped_ms: 2000,
			disconnected_ms: 20_000
		}
	});
	page.at(12_000);

	assert.equal(page.last().state, 'unconfirmed');

	page.at(20_000);
	assert.equal(page.last().state, 'disconnected');
});

// **Only a heartbeat confirms the channel**, and not the documents arriving beside it five
// times a second. Measuring off *anything arrived* would make the ladder a function of how
// busy the deployment is, so a quiet console would freeze where a busy one would not.
test('a document arriving does not confirm the channel', () => {
	const page = openWatching();

	page.at(6000);
	page.socket.says({ message: 'presence', version: 4, loops: [] });
	page.at(6500);

	assert.equal(page.last().state, 'unconfirmed');
});

// **A socket that closed is a fact rather than a silence**, so there is nothing to wait out.
// The rungs are for the gap nobody reported.
test('a socket that closed is disconnected at once, and stops the clock', () => {
	const page = openWatching();

	page.socket.close();

	assert.equal(page.last().state, 'disconnected');
	assert.equal(page.ticking(), false, 'the ladder went on being read after the socket went');
});

const THE_LADDER = {
	heartbeat_ms: 2000,
	unconfirmed_ms: 5000,
	latch_dropped_ms: 2000,
	disconnected_ms: 12_000
};

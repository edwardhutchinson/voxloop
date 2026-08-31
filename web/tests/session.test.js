// The client's Session module: what it says to the server, and what it tells the console.
//
// It is the one thing in the console that talks over the signalling channel, so the four
// things that can happen to a socket are four things the rest of the console never has to
// know how to spell. They are tested here rather than through a component because they are
// facts about the protocol, not about the page.

import assert from 'node:assert/strict';
import test, { beforeEach } from 'node:test';

import { openSignalling } from '../src/lib/session.js';

/** A socket the test holds both ends of. */
class ASocket {
	static opened = [];

	constructor(url) {
		this.url = url;
		this.sent = [];
		this.closed = false;
		this.listeners = {};
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
		onEnded: (reason) => told.push(['ended', reason]),
		onLost: () => told.push(['lost'])
	};
}

beforeEach(() => {
	ASocket.opened = [];
	globalThis.WebSocket = ASocket;
	globalThis.window = { location: { protocol: 'https:', host: 'voxloop.example:8443' } };
});

const lastSocket = () => ASocket.opened.at(-1);

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

test('a refusal is the sign-in no longer standing, because the lobby is all that is asked for', () => {
	const page = listening();
	openSignalling(page);

	lastSocket().says({
		message: 'refused',
		was: 'hello',
		reason: 'That message is for a signed-in user.'
	});

	assert.deepEqual(page.told, [['ended', 'That message is for a signed-in user.']]);
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

	openSignalling(page)();

	assert.equal(lastSocket().closed, true);
	assert.deepEqual(page.told, []);
});

test('anything unreadable on the wire is not read as a document', () => {
	const page = listening();
	openSignalling(page);

	lastSocket().happens('message', 'not json at all');

	assert.deepEqual(page.told, []);
});

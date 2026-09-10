// Connection state: the one thing on the console the server did not say.
//
// The ladder is arithmetic over a clock, so it is tested as arithmetic over a clock: the
// module is handed the moment rather than reaching for one, and every test here stands at a
// rung by naming the second it is standing at. Nothing sleeps.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
	CONFIRMED,
	DISCONNECTED,
	THE_LADDER_V1_FIXES,
	UNCONFIRMED,
	theConnection,
	worse
} from '../src/lib/connection.js';

/** A channel opened at zero, and everything it has said since. */
function opened() {
	const said = [];
	const channel = theConnection({ onConnection: (standing) => said.push(standing) });
	channel.opened(0);

	return { channel, said, last: () => said.at(-1) };
}

/** Where it stands after `at` milliseconds with nothing arriving. */
function at(channel, when) {
	channel.settle(when);
}

test('a channel that has just opened is confirmed, and has missed nothing', () => {
	const { last } = opened();

	assert.equal(last().state, CONFIRMED);
	assert.equal(last().since, 0);
});

// **The three rungs at the thresholds v1 §7 fixes.** The band between them is the whole
// design: a single threshold would mute the Flight Director mid-sentence for a VPN reroute,
// trading a state-honesty problem for a worse availability one.
test('the ladder climbs at five seconds and at twelve', () => {
	const { channel, last } = opened();

	for (const [when, rung] of [
		[4000, CONFIRMED],
		[5000, UNCONFIRMED],
		[11_999, UNCONFIRMED],
		[12_000, DISCONNECTED],
		[600_000, DISCONNECTED]
	]) {
		at(channel, when);

		assert.equal(last().state, rung, `${when} ms after the last heartbeat`);
	}
});

// **The running age is what stops a frozen console being mistaken for a live one**
// (ADR-0018). Last-known state is not blanked — an empty page reads as *nothing is
// happening* when everything may be — so what makes it honest is the number beside it
// moving.
test('the age is said again every second while the state is not confirmed', () => {
	const { channel, said } = opened();

	at(channel, 5000);
	at(channel, 5400);
	at(channel, 6000);
	at(channel, 7000);

	const stale = said.filter((standing) => standing.state === UNCONFIRMED);
	assert.deepEqual(
		stale.map((standing) => Math.floor(standing.since / 1000)),
		[5, 6, 7],
		'the age either stuttered or stood still'
	);
});

// A caller told the same sentence five times a second would be a caller redrawing a banner
// that has not changed a word.
test('a confirmed channel says nothing on every tick', () => {
	const { channel, said } = opened();
	const before = said.length;

	at(channel, 400);
	at(channel, 1200);
	at(channel, 3000);

	assert.equal(said.length, before);
});

// **A heartbeat is the only thing that moves the ladder back down**, and it moves it the
// whole way: the gap it is measured from starts again.
test('a heartbeat confirms the channel again, from that moment', () => {
	const { channel, last } = opened();
	at(channel, 9000);
	assert.equal(last().state, UNCONFIRMED);

	channel.confirmed(9500, THE_LADDER_V1_FIXES);
	at(channel, 13_000);

	assert.equal(last().state, CONFIRMED, 'the gap was measured from the wrong moment');
});

// **The clock is the deployment's, not the console's** (v1 §7). The four timers are startup
// settings tuned against the site's own VPN, and they arrive on every heartbeat — so a
// console that missed the first one still runs the right ladder from the next.
test('the ladder the deployment sent is the one that is run', () => {
	const { channel, last } = opened();

	channel.confirmed(0, {
		heartbeat_ms: 3000,
		unconfirmed_ms: 8000,
		latch_dropped_ms: 2000,
		disconnected_ms: 20_000
	});
	at(channel, 12_000);

	assert.equal(last().state, UNCONFIRMED, 'the console ran the spec’s numbers over the site’s');

	at(channel, 20_000);
	assert.equal(last().state, DISCONNECTED);
});

// **A socket that closed is a fact rather than a silence.** There is nothing to wait out: the
// rungs are for the case nobody reported, which is the wedged client and the flapping VPN.
test('a channel known to have gone is disconnected at once', () => {
	const { channel, last } = opened();

	channel.gone(200);

	assert.equal(last().state, DISCONNECTED);
	assert.equal(last().aLatchStands, false);
});

// **A latched emission is dropped after a couple of seconds of `unconfirmed`, and a momentary
// key survives** (ADR-0018). A latch is an assertion made once, possibly minutes ago, and its
// entire safety story is that the console will show it to you; that story is void the moment
// the console cannot be trusted. A held button is a human continuously asserting intent, so
// it survives — which is why this is one flag about latching rather than a second emission
// predicate.
test('a latch stops standing two seconds into unconfirmed, well before emission is withdrawn', () => {
	const { channel, last } = opened();

	at(channel, 6900);
	assert.equal(last().aLatchStands, true, 'the latch died inside its grace');

	at(channel, 7000);
	assert.equal(last().aLatchStands, false);
	assert.equal(last().state, UNCONFIRMED, 'emission was withdrawn along with the latch');
});

test('a latch stands again once the channel is confirmed again', () => {
	const { channel, last } = opened();
	at(channel, 8000);
	assert.equal(last().aLatchStands, false);

	channel.confirmed(8100, THE_LADDER_V1_FIXES);

	assert.equal(last().aLatchStands, true);
});

// **Green needs both, red needs one** — the same pessimistic merge the media path's two ends
// already use (ADR-0042), applied to the other axis. This tab measures the heartbeats it is
// not getting; the server measures the answers it is not getting. They are two different
// silences and either of them is enough.
test('two readings of one channel merge pessimistically', () => {
	assert.equal(worse(CONFIRMED, CONFIRMED), CONFIRMED);
	assert.equal(worse(CONFIRMED, UNCONFIRMED), UNCONFIRMED);
	assert.equal(worse(UNCONFIRMED, CONFIRMED), UNCONFIRMED);
	assert.equal(worse(CONFIRMED, DISCONNECTED), DISCONNECTED);
	assert.equal(worse(DISCONNECTED, CONFIRMED), DISCONNECTED);
	assert.equal(worse(UNCONFIRMED, DISCONNECTED), DISCONNECTED);
});

// A rung the console has no reading of is read as the best case rather than the worst, and
// deliberately so: this merge is only ever the *other* reading, and treating a missing one as
// red would withdraw emission on every document sent by a server that has not learned to say
// it. The safe direction is the console's own clock, which is always there.
test('a reading that is not a rung leaves the other one standing', () => {
	assert.equal(worse(UNCONFIRMED, undefined), UNCONFIRMED);
	assert.equal(worse(undefined, DISCONNECTED), DISCONNECTED);
});

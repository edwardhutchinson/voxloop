// The emission modes, above the Input seam: momentary, latched, and no third (v1 §4).
//
// **Latch is never derived from a momentary press** (ADR-0022), so most of what is asked here
// is what does *not* happen: a press of any length on the key you hold never latches, and
// nothing anywhere counts a tap. The rest is the pair of failures the modes own rather than
// the seam — a latch that survives the source it was pressed with, and a latch that does not
// survive a withdrawal.
//
// The keyboard is handed a window to listen on, the way the seam's own tests hand it one.
// There is no browser here either: two modes over two readings is arithmetic.

import assert from 'node:assert/strict';
import test from 'node:test';

import { keyingModes, LATCHED, MOMENTARY, modes } from '../src/lib/modes.js';

/** Something for the keyboard sources to listen on, and a way to make them hear something. */
function aWindow() {
	const listeners = new Map();
	const fire = (type, event) => (listeners.get(type) ?? []).forEach((listener) => listener(event));

	return {
		on: {
			addEventListener: (type, listener) =>
				listeners.set(type, [...(listeners.get(type) ?? []), listener]),
			removeEventListener: () => {}
		},
		press: (event) => fire('keydown', event),
		release: (event) => fire('keyup', event),
		leave: () => fire('blur', {})
	};
}

const key = (code, how = {}) => ({
	code,
	shiftKey: how.shift === true,
	ctrlKey: false,
	altKey: false,
	metaKey: false,
	repeat: how.repeat === true,
	target: null,
	preventDefault: () => {}
});

/** The modes, with a record of everything they have said, ready to key. */
function operating() {
	const there = aWindow();
	const keyed = [];
	const latched = [];
	const dropped = [];

	const keys = keyingModes({
		on: there.on,
		onKeying: (wants) => keyed.push(wants),
		onLatched: (is) => latched.push(is),
		onDropped: (source) => dropped.push(source)
	});
	keys.available(true);

	return { there, keyed, latched, dropped, keys };
}

/** The default keys, pressed and released as a hand does it. */
const held = () => key('Backquote');
const latching = () => key('Backquote', { shift: true });

test('the defaults are the backtick, and the backtick with shift', () => {
	assert.deepEqual(
		modes.map(({ named, binding }) => [named, binding]),
		[
			[MOMENTARY, { code: 'Backquote' }],
			[LATCHED, { code: 'Backquote', shift: true }]
		]
	);
});

test('the key you hold talks while you hold it', () => {
	const { there, keyed } = operating();

	there.press(held());
	there.release(held());

	assert.deepEqual(keyed, [true, false]);
});

// **Press to open, press to close** (v1 §4). The second press is a press and not a release:
// a source that stopped reporting its release would otherwise close the latch it opened, and
// a latch that closes itself on a fault is a latch nobody can rely on.
test('the latch key opens on a press and closes on the next one', () => {
	const { there, keyed, latched } = operating();

	there.press(latching());
	there.release(latching());
	assert.deepEqual(keyed, [true], 'letting go of the latch key stopped the transmission');
	assert.deepEqual(latched, [true]);

	there.press(latching());
	assert.deepEqual(keyed, [true, false]);
	assert.deepEqual(latched, [true, false]);
});

// **The whole of ADR-0022 in one assertion.** No short tap, no double tap, no held duration:
// the key an operator holds has one meaning on every device and in every tier, and there is
// nothing in the module that could give it a second one.
test('the key you hold never latches, however it is pressed', () => {
	const { there, keyed, latched } = operating();

	// A tap.
	there.press(held());
	there.release(held());
	// Two of them.
	there.press(held());
	there.release(held());
	there.press(held());
	there.release(held());
	// And one held long enough to autorepeat.
	there.press(held());
	there.press(key('Backquote', { repeat: true }));
	there.release(held());

	assert.deepEqual(keyed, [true, false, true, false, true, false, true, false]);
	assert.deepEqual(latched, [], 'a momentary press latched');
});

// **The level model absorbs both with no special case** (ADR-0021): emission is the OR of the
// two, so keying over a latch changes nothing and letting go of it does not close the latch.
test('the key you hold does not close a latch', () => {
	const { there, keyed, latched } = operating();
	there.press(latching());
	there.release(latching());

	there.press(held());
	there.release(held());

	assert.deepEqual(keyed, [true], 'a momentary press interrupted a latched transmission');
	assert.deepEqual(latched, [true]);
});

// **The on-screen control is the other way to reach each mode** (ADR-0022): latch is
// available from the keyboard and from the console, and a single-button device is momentary
// only because there is no gesture that reaches this.
test('the buttons on the bar reach both modes', () => {
	const { keys, keyed, latched } = operating();

	keys.onScreen[MOMENTARY].down();
	keys.onScreen[MOMENTARY].up();
	assert.deepEqual(keyed, [true, false]);

	keys.onScreen[LATCHED].down();
	keys.onScreen[LATCHED].up();
	assert.deepEqual(keyed, [true, false, true]);
	assert.deepEqual(latched, [true]);
});

// **A latched transmission is the console holding the key open**, not the button that opened
// it. The button can go — the pointer leaves it, the view is switched — and the latch stands.
test('a latch outlives the control that opened it', () => {
	const { keys, keyed, latched, dropped } = operating();
	keys.onScreen[LATCHED].down();
	keys.onScreen[LATCHED].up();

	keys.onScreen[LATCHED].down();
	keys.available(false);

	assert.deepEqual(latched, [true, false]);
	assert.deepEqual(keyed, [true, false]);
	assert.deepEqual(dropped, [], 'a latch button dying was reported as a key being taken');
});

// **Withdrawal drops the latch** (v1 §7). Key state never returns: a console holding the key
// open across an outage is the hot mic with a randomly-timed start, arriving at a moment the
// operator cannot know about.
test('a withdrawal drops a latch and does not bring it back', () => {
	const { there, keys, keyed, latched } = operating();
	there.press(latching());
	there.release(latching());

	keys.available(false);
	assert.deepEqual(keyed, [true, false]);
	assert.deepEqual(latched, [true, false]);

	keys.available(true);
	assert.deepEqual(keyed, [true, false], 'the latch came back on its own');
});

// **A source that dies while keyed forces an unkey and says so locally** (ADR-0021), and it
// says which source, because *the key control went away* and *your keyboard went away* send
// an operator to look at two different things.
test('a key held when keying is withdrawn drops, and names the source', () => {
	const { there, keys, keyed, dropped } = operating();
	there.press(held());

	keys.available(false);

	assert.deepEqual(keyed, [true, false]);
	assert.deepEqual(dropped, ['the keyboard']);
});

test('the key control going while it is held names itself', () => {
	const { keys, dropped } = operating();
	keys.onScreen[MOMENTARY].down();

	keys.available(false);

	assert.deepEqual(dropped, ['the key control']);
});

// A window losing focus is the operator's own act rather than a fault, so it drops the level
// and says nothing (ADR-0021). Nothing has gone away; they went away.
test('a window losing focus drops the key quietly', () => {
	const { there, keyed, dropped } = operating();
	there.press(held());

	there.leave();

	assert.deepEqual(keyed, [true, false]);
	assert.deepEqual(dropped, [], 'switching applications was reported as a fault');
});

// A latch is not a key being held, so it survives the operator going to another application
// — which is most of what anybody latches for.
test('a latch survives the window losing focus', () => {
	const { there, keyed, latched } = operating();
	there.press(latching());
	there.release(latching());

	there.leave();

	assert.deepEqual(keyed, [true]);
	assert.deepEqual(latched, [true]);
});

test('a mode can be bound to another key, and the refusals are the seam’s', () => {
	const { there, keys, keyed } = operating();

	assert.equal(keys.rebind(MOMENTARY, { code: 'KeyF' }), null);
	there.press(key('KeyF'));
	assert.deepEqual(keyed, [true]);

	assert.match(keys.rebind(LATCHED, { code: 'Space' }) ?? '', /Space/);
	assert.match(keys.rebind(LATCHED, { code: 'KeyF' }) ?? '', /already in use/);
});

// **A console going is keying stopping**, whatever was holding it open. A role given up under
// a held key or under a latch has to reach Audio and the server as an unkey rather than as
// listeners quietly going away (ADR-0021) — the microphone is the thing an operator cannot
// see, and it is the thing they would most want to know about.
test('relinquishing under a held key stops it', () => {
	const { there, keys, keyed } = operating();
	there.press(held());

	keys.stop();

	assert.deepEqual(keyed, [true, false]);
});

test('relinquishing under a latch stops it', () => {
	const { there, keys, keyed, latched } = operating();
	there.press(latching());
	there.release(latching());

	keys.stop();

	assert.deepEqual(keyed, [true, false]);
	assert.deepEqual(latched, [true, false]);
});

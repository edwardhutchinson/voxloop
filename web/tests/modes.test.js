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

import { keyingModes, LATCHED, MOMENTARY, modes, PRIORITY } from '../src/lib/modes.js';

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
	ctrlKey: how.ctrl === true,
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
	const announced = [];
	const elevated = [];

	const keys = keyingModes({
		on: there.on,
		onKeying: (wants) => keyed.push(wants),
		onPriority: (is) => elevated.push(is),
		onLatched: (is) => latched.push(is),
		onDropped: (source) => dropped.push(source),
		onLatchDropped: () => announced.push(true)
	});
	keys.available(true);

	return { there, keyed, latched, dropped, announced, elevated, keys };
}

/** The default keys, pressed and released as a hand does it. */
const held = () => key('Backquote');
const latching = () => key('Backquote', { shift: true });
const prioritising = () => key('Backquote', { ctrl: true });

test('the defaults are the backtick, and the backtick with shift and with control', () => {
	assert.deepEqual(
		modes.map(({ named, binding }) => [named, binding]),
		[
			[MOMENTARY, { code: 'Backquote' }],
			[LATCHED, { code: 'Backquote', shift: true }],
			[PRIORITY, { code: 'Backquote', ctrl: true }]
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

// ---- #43: a latch this console can no longer show ------------------------------------------

// **A latched emission is dropped once the console cannot be trusted to show it, and a
// momentary key survives** (ADR-0018). The asymmetry is the whole rule: a held button is a
// human continuously asserting intent with their thumb, and a latch is an assertion made
// once, possibly minutes ago, whose entire safety story is that the console will show it to
// you.
test('a latch the console cannot show is dropped', () => {
	const { there, keys, keyed, latched } = operating();
	there.press(latching());
	there.release(latching());

	keys.theLatchCannotBeShown();

	assert.deepEqual(latched, [true, false]);
	assert.deepEqual(keyed, [true, false]);
});

test('a momentary key survives what takes the latch down', () => {
	const { there, keys, keyed } = operating();
	there.press(held());

	keys.theLatchCannotBeShown();

	assert.deepEqual(keyed, [true], 'a held button was cut off mid-word');
});

// **The one user-facing message in the product that does not originate at the server**
// (ADR-0018). The case it exists for is the console being unable to be told anything, so
// waiting to be told would be waiting forever — and an operator who believes they are still
// transmitting is the failure the rule was written to remove, arriving through the other
// door.
test('a latch taken down by anything but the operator is announced', () => {
	const { there, keys, announced } = operating();
	there.press(latching());
	there.release(latching());

	keys.theLatchCannotBeShown();

	assert.deepEqual(announced, [true]);
});

// Withdrawal is the other end of the same outage and takes everything, latch included — so it
// announces the latch too, because *why* the console stopped emitting is said elsewhere and
// *what it cost* is said here.
test('a withdrawal announces the latch it took with it', () => {
	const { there, keys, announced, latched } = operating();
	there.press(latching());
	there.release(latching());

	keys.available(false);

	assert.deepEqual(latched, [true, false]);
	assert.deepEqual(announced, [true]);
});

// A latch the operator closed themselves needs no announcement: they are the one who did it,
// and telling somebody what they just did is noise where it matters most.
test('a latch the operator closed is not announced', () => {
	const { there, announced } = operating();
	there.press(latching());
	there.release(latching());

	there.press(latching());
	there.release(latching());

	assert.deepEqual(announced, []);
});

// Said once per latch rather than on every reading. The console asks whenever the ladder
// moves, and a rung that stayed where it was has taken nothing down.
test('nothing is announced where there was no latch to drop', () => {
	const { keys, announced, keyed } = operating();

	keys.theLatchCannotBeShown();
	keys.theLatchCannotBeShown();

	assert.deepEqual(announced, []);
	assert.deepEqual(keyed, []);
});

// ---- #45: priority ----------------------------------------------------------------------------

// **The level model absorbs priority with no special case** (ADR-0046):
// `emitting = ordinary OR priority` and `is-priority = priority`. From cold, the priority key
// both keys and elevates, and letting go ends the transmission.
test('the priority key from cold keys and elevates, and letting go ends both', () => {
	const { there, keyed, elevated } = operating();

	there.press(prioritising());
	assert.deepEqual(keyed, [true]);
	assert.deepEqual(elevated, [true]);

	there.release(prioritising());
	assert.deepEqual(keyed, [true, false]);
	assert.deepEqual(elevated, [true, false]);
});

// Pressing priority while latched raises the priority level without touching the latch, so
// releasing it returns to a latched, ordinary transmission with nothing to restore.
test('the priority key over a latch elevates it and leaves the latch standing', () => {
	const { there, keyed, latched, elevated } = operating();
	there.press(latching());
	there.release(latching());

	there.press(prioritising());
	there.release(prioritising());

	assert.deepEqual(keyed, [true], 'priority interrupted a latched transmission');
	assert.deepEqual(latched, [true]);
	assert.deepEqual(elevated, [true, false]);
});

// Holding both keys is one transmission at priority, not two (ADR-0007): the ordinary key
// going up under priority changes nothing, and priority going up under the ordinary key lowers
// the transmission without ending it.
test('holding the ordinary key and the priority key is one transmission at priority', () => {
	const { there, keyed, elevated } = operating();

	there.press(held());
	there.press(prioritising());
	assert.deepEqual(keyed, [true]);
	assert.deepEqual(elevated, [true]);

	// The priority key's release: letting go of `` ` `` releases every binding on it, because
	// a release is matched on the key alone — so the ordinary key goes with it.
	there.release(prioritising());
	assert.deepEqual(keyed, [true, false]);
	assert.deepEqual(elevated, [true, false]);
});

test('priority released under a held on-screen key lowers without ending', () => {
	const { there, keys, keyed, elevated } = operating();
	keys.onScreen[MOMENTARY].down();

	there.press(prioritising());
	there.release(prioritising());

	assert.deepEqual(keyed, [true], 'letting go of priority ended a held transmission');
	assert.deepEqual(elevated, [true, false]);
});

// **Priority never latches** (v1 §4). It is momentary only: no press of any length, and no
// number of them, leaves it up once the key is released.
test('the priority key never latches, however it is pressed', () => {
	const { there, keyed, latched, elevated } = operating();

	there.press(prioritising());
	there.release(prioritising());
	there.press(prioritising());
	there.press(key('Backquote', { ctrl: true, repeat: true }));
	there.release(prioritising());

	assert.deepEqual(elevated, [true, false, true, false]);
	assert.deepEqual(keyed, [true, false, true, false]);
	assert.deepEqual(latched, [], 'a priority press latched');
});

test('the priority button on the bar is momentary too', () => {
	const { keys, keyed, elevated } = operating();

	keys.onScreen[PRIORITY].down();
	keys.onScreen[PRIORITY].up();

	assert.deepEqual(keyed, [true, false]);
	assert.deepEqual(elevated, [true, false]);
});

// A stuck priority control defeats everybody's volume setting for as long as it is stuck (ADR-0046),
// so the source going takes it down like any other held key, and says so.
test('a priority key held when keying is withdrawn drops, and names the source', () => {
	const { there, keys, keyed, elevated, dropped } = operating();
	there.press(prioritising());

	keys.available(false);

	assert.deepEqual(keyed, [true, false]);
	assert.deepEqual(elevated, [true, false]);
	assert.deepEqual(dropped, ['the keyboard']);
});

// Priority is not reached through the latch: what the network takes down is the latch, and a
// finger on the priority key is a human continuously asserting intent, exactly like one on the
// ordinary key.
test('a priority key survives what takes the latch down', () => {
	const { there, keys, keyed, elevated } = operating();
	there.press(prioritising());

	keys.theLatchCannotBeShown();

	assert.deepEqual(keyed, [true]);
	assert.deepEqual(elevated, [true]);
});

test('relinquishing under a held priority key stops it', () => {
	const { there, keys, keyed, elevated } = operating();
	there.press(prioritising());

	keys.stop();

	assert.deepEqual(keyed, [true, false]);
	assert.deepEqual(elevated, [true, false]);
});

// Three bindings on one key are three distinct presses, and a rebind onto another mode's key is
// refused — one key does one thing (ADR-0022).
test('the priority key is bound like the others and refused like them', () => {
	const { there, keys, keyed, elevated } = operating();

	assert.match(keys.rebind(PRIORITY, { code: 'Backquote' }) ?? '', /already in use/);
	assert.equal(keys.rebind(PRIORITY, { code: 'KeyP' }), null);

	there.press(key('KeyP'));
	assert.deepEqual(keyed, [true]);
	assert.deepEqual(elevated, [true]);
});

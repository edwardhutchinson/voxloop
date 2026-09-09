// The Input seam: a level and a liveness flag, the OR over them, and the sources that publish
// into it.
//
// ADR-0021's whole argument is about what happens when something goes wrong, and every case
// it names is here: a release that never arrives, a source that dies while it is held, a
// second source that is not live, a window that loses focus under a held key. There is no
// browser in these tests because there is nothing browser-shaped in the seam — a source
// publishes two booleans, and the keyboard is handed the thing it listens on.
//
// **The names here are deliberately not the names of the modes.** The seam is handed a
// binding per name and reports under the same names, and it has no idea what any of them
// means (ADR-0022); tests written in `momentary` and `latch` would read as though it did.
// What the modes do with these readings is `modes.test.js`.
//
// `input-seam.test.js` beside this one asks the other question, which is whether the build
// refuses anybody reaching past this interface.

import assert from 'node:assert/strict';
import test from 'node:test';

import { describe, fromEvent, keying, refusal } from '../src/lib/input/index.js';
import { levels } from '../src/lib/input/level.js';

const A_KEY = { code: 'Backquote' };
const THE_SAME_KEY_SHIFTED = { code: 'Backquote', shift: true };

/** Something for the keyboard source to listen on, and a way to make it hear something. */
function aWindow() {
	const listeners = new Map();
	const fire = (type, event) => (listeners.get(type) ?? []).forEach((listener) => listener(event));

	return {
		on: {
			addEventListener: (type, listener) =>
				listeners.set(type, [...(listeners.get(type) ?? []), listener]),
			removeEventListener: (type, listener) =>
				listeners.set(
					type,
					(listeners.get(type) ?? []).filter((held) => held !== listener)
				)
		},
		press: (event) => fire('keydown', event),
		release: (event) => fire('keyup', event),
		leave: () => fire('blur', {}),
		listening: () => [...listeners.values()].flat().length
	};
}

/** A key event, as a browser delivers one. `target` is what had focus when it happened. */
const key = (code, how = {}) => ({
	code,
	shiftKey: how.shift === true,
	ctrlKey: how.ctrl === true,
	altKey: how.alt === true,
	metaKey: how.meta === true,
	repeat: how.repeat === true,
	target: how.target ?? null,
	preventDefault: () => {}
});

/** An element, as far as the focus guard is concerned. */
const focusedOn = (selectors) => ({
	closest: (asked) => (asked.split(', ').some((one) => selectors.includes(one)) ? {} : null)
});

/** An Input with a record of everything it has said. */
function watching({ bindings = { anything: A_KEY }, on } = {}) {
	const wanted = [];
	const dropped = [];

	const input = keying({
		bindings,
		on,
		onIntent: (named, wants) => wanted.push([named, wants]),
		onDropped: (named, source) => dropped.push([named, source])
	});

	return { wanted, dropped, input };
}

/** Just the answers, where a test is about one name and does not care which. */
const answers = (wanted) => wanted.map(([, wants]) => wants);

test('a control held down wants to emit, and a control released does not', () => {
	const { wanted, input } = watching();
	input.available(true);

	input.controls.anything.down();
	input.controls.anything.up();

	assert.deepEqual(answers(wanted), [true, false]);
});

// **A control that is not on screen is a source that is not there.** Emission is withdrawn on
// a lost audio path (ADR-0042), so the key control goes — and a source that is not live is not
// in the OR, whatever a pointer happens to be doing over the space it left.
test('a control that is not on screen cannot key', () => {
	const { wanted, input } = watching();

	input.controls.anything.down();

	assert.deepEqual(wanted, [], 'a control nobody can see keyed');
});

// **The failure the level exists to prevent.** A key control that vanishes under a held
// pointer delivers no release: under an event-shaped interface the transmission hangs, the
// microphone stays open, and the server goes on telling everybody that a session with no audio
// path is transmitting. Here the source stops being live, leaves the OR, and the key drops.
test('a control that goes while it is held drops the key', () => {
	const { wanted, dropped, input } = watching();
	input.available(true);
	input.controls.anything.down();
	assert.deepEqual(answers(wanted), [true]);

	input.available(false);

	assert.deepEqual(answers(wanted), [true, false], 'a control that vanished left the key held');
	assert.deepEqual(
		dropped,
		[['anything', 'the key control']],
		'the key dropped and nothing said which source took it'
	);
});

// **And it does not come back on its own.** v1 §7's rule for the other end of an outage is
// that a source which was high across a withdrawal contributes nothing until it goes low and
// high again; a control that returned still holding what it held would be a transmission
// starting at a moment nobody chose.
test('a control that comes back is not still holding what it held', () => {
	const { wanted, input } = watching();
	input.available(true);
	input.controls.anything.down();
	input.available(false);

	input.available(true);

	assert.deepEqual(answers(wanted), [true, false], 'the key came back without a hand on it');
});

// **The answer moves when it moves.** A level is sampled rather than counted, so a caller
// told the same thing twice would be a caller that had to remember what it was last told in
// order to act on it — which is how a signal ends up sent per sample.
test('saying the same thing twice is said once', () => {
	const { wanted, input } = watching();
	input.available(true);

	input.controls.anything.down();
	input.controls.anything.down();
	input.controls.anything.up();
	input.controls.anything.up();

	assert.deepEqual(answers(wanted), [true, false]);
});

// **Sources are additive and the client ORs the live ones** (ADR-0021). The reading itself
// rather than the assembled seam, because the promise the Tauri wrapper is owed is about what
// happens when a source it added is beside the ones already there (ADR-0020).
test('any live source wanting to emit is enough', () => {
	const wanted = [];
	const reading = levels({ onIntent: (wants) => wanted.push(wants) });
	const control = reading.add('the key control');
	const footswitch = reading.add('a footswitch');
	control.publish(false, true);
	footswitch.publish(false, true);

	footswitch.publish(true, true);
	control.publish(true, true);
	control.publish(false, true);

	assert.deepEqual(wanted, [true], 'releasing one source dropped a key another was holding');

	footswitch.publish(false, true);
	assert.deepEqual(wanted, [true, false]);
});

// **A source that is not live is not in the OR**, whatever it last published. This is the
// unplugged headset: by the time it is true the source is gone and can send nothing, so
// liveness has to be a property of the source rather than an event it emits.
test('a source that is not live wants nothing, whatever it last said', () => {
	const wanted = [];
	const reading = levels({ onIntent: (wants) => wanted.push(wants) });
	const footswitch = reading.add('a footswitch');

	footswitch.publish(true, false);

	assert.deepEqual(wanted, [], 'a dead source keyed');
});

// **A source that dies while it is held drops the key**, said of the seam's own rule rather
// than of the one source that has it today. There is no `gone` to call: a source that has left
// publishes that it is not live, which is both what is true and what takes it out of the OR.
test('a source that stops being live drops a key it was holding', () => {
	const wanted = [];
	const dropped = [];
	const reading = levels({
		onIntent: (wants) => wanted.push(wants),
		onDropped: (named) => dropped.push(named)
	});
	const footswitch = reading.add('a footswitch');
	footswitch.publish(true, true);
	assert.deepEqual(wanted, [true]);

	footswitch.publish(true, false);

	assert.deepEqual(wanted, [true, false], 'a source that vanished left the key held');
	assert.deepEqual(dropped, ['a footswitch'], 'nothing said which source took the key with it');
});

// **The forced unkey is said only when something was taken.** A source dying with the key up
// has changed nothing an operator can hear, and one dying while a second source is still
// holding the key has taken nothing off the air either — telling them their key dropped when
// it did not is the same lie as the reverse (ADR-0016).
test('a source dying says so only when the key actually dropped', () => {
	const dropped = [];
	const reading = levels({ onIntent: () => {}, onDropped: (named) => dropped.push(named) });
	const control = reading.add('the key control');
	const footswitch = reading.add('a footswitch');
	control.publish(true, true);
	footswitch.publish(true, true);

	footswitch.publish(true, false);
	assert.deepEqual(dropped, [], 'a key another source is still holding was reported as dropped');

	control.publish(false, true);
	assert.deepEqual(dropped, [], 'a released key was reported as a source dying');
});

// **A source never knows which emission mode it serves** (ADR-0021, ADR-0022). Mode logic
// lives above the seam, in `modes.js`; a source that decided its own could latch by accident,
// which would make an open mic the failure mode of a hardware fault.
test('nothing under the seam mentions a mode', async () => {
	const { readFileSync, readdirSync } = await import('node:fs');
	const { fileURLToPath } = await import('node:url');
	const under = fileURLToPath(new URL('../src/lib/input/', import.meta.url));

	const files = readdirSync(under, { recursive: true, withFileTypes: true })
		.filter((entry) => entry.isFile())
		.map((entry) => `${entry.parentPath}/${entry.name}`);

	for (const path of files) {
		const source = readFileSync(path, 'utf8');
		// The words appear in prose saying they are somebody else's, so what is checked is the
		// code: a source cannot branch on a mode it has no way to name.
		const code = source.replaceAll(/\/\/.*$/gm, '').replaceAll(/\/\*[\s\S]*?\*\//g, '');
		for (const mode of ['momentary', 'latch']) {
			assert.doesNotMatch(
				code,
				new RegExp(mode, 'i'),
				`${path} knows about ${mode} — mode logic lives above the seam`
			);
		}
	}
});

// **The console cannot invent a source.** Registering one is not on the seam's answer, which
// is what keeps *the wrapper may only ever add a source* (ADR-0020) a claim about the files
// under `input/` rather than about whatever the console happened to register.
test('the seam hands out no way to register a source', () => {
	const { input } = watching();

	assert.deepEqual(Object.keys(input), ['controls', 'bound', 'available', 'rebind', 'stop']);
});

// **Each name is read separately** (ADR-0022). One reading with every source in it would be
// one level for two keys, and two bindings collapsing into one level is the derived latch
// this seam exists to make impossible.
test('a press on one key is not a press on the other', () => {
	const there = aWindow();
	const { wanted, input } = watching({
		bindings: { one: A_KEY, other: { code: 'KeyF' } },
		on: there.on
	});
	input.available(true);

	there.press(key('Backquote'));

	assert.deepEqual(wanted, [['one', true]], 'a press reached a key it was not bound to');
});

test('a bound key keys while it is down', () => {
	const there = aWindow();
	const { wanted, input } = watching({ on: there.on });
	input.available(true);

	there.press(key('Backquote'));
	there.release(key('Backquote'));

	assert.deepEqual(answers(wanted), [true, false]);
});

test('a key that is not the binding does nothing', () => {
	const there = aWindow();
	const { wanted, input } = watching({ on: there.on });
	input.available(true);

	there.press(key('KeyG'));

	assert.deepEqual(wanted, []);
});

// **A press is matched exactly**, which is the whole of two bindings living on one key
// (ADR-0022). A modifier held is a different binding rather than the same one with something
// extra, in both directions.
test('a press matches the modifiers exactly', () => {
	const there = aWindow();
	const { wanted, input } = watching({ on: there.on });
	input.available(true);

	there.press(key('Backquote', { shift: true }));
	assert.deepEqual(wanted, [], 'a modified press reached an unmodified binding');

	const shifted = aWindow();
	const other = watching({ bindings: { anything: THE_SAME_KEY_SHIFTED }, on: shifted.on });
	other.input.available(true);

	shifted.press(key('Backquote'));
	assert.deepEqual(other.wanted, [], 'an unmodified press reached a modified binding');
});

// **A release is matched on the key alone.** An operator who presses a modifier while already
// holding the key would otherwise deliver a release that matched nothing — and a release that
// matches nothing is an open mic, which is the one outcome every rule here is aimed at.
test('a release is taken however the modifiers stand', () => {
	const there = aWindow();
	const { wanted, input } = watching({ on: there.on });
	input.available(true);

	there.press(key('Backquote'));
	there.release(key('Backquote', { shift: true }));

	assert.deepEqual(answers(wanted), [true, false], 'the key was left held by a shifted release');
});

// **Never while focus is in a text field or on an interactive control** (ADR-0022). The
// console must not place a focusable control where an operator's hands rest, and a key that
// typed a backtick into a form and keyed at the same time would be the reason why.
test('a press with focus in a text field or on a control does not key', async (t) => {
	for (const where of ['input', 'textarea', 'select', 'button', 'a[href]', '[contenteditable]']) {
		await t.test(where, () => {
			const there = aWindow();
			const { wanted, input } = watching({ on: there.on });
			input.available(true);

			there.press(key('Backquote', { target: focusedOn([where]) }));

			assert.deepEqual(wanted, [], `a key pressed with focus on ${where} keyed`);
		});
	}
});

// The refusal is on the press alone. Focus can move under a held key — a click lands
// somewhere while the other hand is talking — and a release nobody accepted is an open mic.
test('a release is taken wherever focus has got to', () => {
	const there = aWindow();
	const { wanted, input } = watching({ on: there.on });
	input.available(true);
	there.press(key('Backquote'));

	there.release(key('Backquote', { target: focusedOn(['button']) }));

	assert.deepEqual(answers(wanted), [true, false], 'a release was refused and left the key held');
});

// **A window that loses focus drops the level** (ADR-0021). Holding the key and switching to
// another application never delivers the release, and under an event-shaped seam that is a
// hung transmission that nothing later corrects.
test('a window that loses focus drops a key held in it', () => {
	const there = aWindow();
	const { wanted, input } = watching({ on: there.on });
	input.available(true);
	there.press(key('Backquote'));

	there.leave();

	assert.deepEqual(answers(wanted), [true, false], 'the key was left held in an unfocused window');
});

// **Autorepeat may not raise a level that is low.** A key still physically held when the
// window comes back delivers repeats and no fresh press, so a level raised by one would be a
// transmission starting at a moment nobody chose — v1 §7's rule for a key held across an
// outage, arriving from the one case in a browser that produces it.
test('a repeat does not key a window that has come back', () => {
	const there = aWindow();
	const { wanted, input } = watching({ on: there.on });
	input.available(true);
	there.press(key('Backquote'));
	there.leave();

	there.press(key('Backquote', { repeat: true }));
	assert.deepEqual(answers(wanted), [true, false], 'autorepeat keyed a key nobody pressed');

	// And letting go and pressing it again is what brings it back, which is the rest of the
	// same rule: LOW, then HIGH.
	there.release(key('Backquote'));
	there.press(key('Backquote'));
	assert.deepEqual(answers(wanted), [true, false, true]);
});

test('a repeat under a key that is already held changes nothing', () => {
	const there = aWindow();
	const { wanted, input } = watching({ on: there.on });
	input.available(true);

	there.press(key('Backquote'));
	there.press(key('Backquote', { repeat: true }));
	there.release(key('Backquote'));

	assert.deepEqual(answers(wanted), [true, false]);
});

// **PTT keys are live only under an assumed role** (ADR-0022), and only where there is an
// audio path to key over (ADR-0042). Both arrive as the same one answer, because they are one
// fact about the console rather than two about the key.
test('a key is inert until keying stands at all', () => {
	const there = aWindow();
	const { wanted } = watching({ on: there.on });

	there.press(key('Backquote'));

	assert.deepEqual(wanted, [], 'a key keyed with no role and no audio path');
});

test('a key held when keying is withdrawn drops, and says which source', () => {
	const there = aWindow();
	const { wanted, dropped, input } = watching({ on: there.on });
	input.available(true);
	there.press(key('Backquote'));

	input.available(false);

	assert.deepEqual(answers(wanted), [true, false]);
	assert.deepEqual(dropped, [['anything', 'the keyboard']]);
});

test('a key held across a withdrawal does not resume when keying comes back', () => {
	const there = aWindow();
	const { wanted, input } = watching({ on: there.on });
	input.available(true);
	there.press(key('Backquote'));
	input.available(false);

	input.available(true);
	there.press(key('Backquote', { repeat: true }));

	assert.deepEqual(answers(wanted), [true, false], 'the key came back without a hand on it');
});

// **A role given up is not a role you can key.** The console stops the seam when it goes, and
// what that has to mean is that the listeners are gone rather than merely ignored.
test('a stopped seam is listening to nothing', () => {
	const there = aWindow();
	const { wanted, input } = watching({ on: there.on });
	input.available(true);
	assert.ok(there.listening() > 0);

	input.stop();
	there.press(key('Backquote'));

	assert.equal(there.listening(), 0);
	assert.deepEqual(wanted, []);
});

// **The console is rendered on the server at build time**, where there is no window to listen
// on. A source with no events reaching it is one that is not live, rather than one that
// quietly keys nothing: the console may not draw a way to talk that cannot work (ADR-0016).
test('with nothing to listen on, the keyboard is never live', () => {
	const { wanted, input } = watching({ on: undefined });
	input.available(true);

	assert.deepEqual(wanted, [], 'a keyboard with no window to listen on keyed');
});

test('the keys as they stand are what the seam was given', () => {
	const { input } = watching({ bindings: { one: A_KEY, other: THE_SAME_KEY_SHIFTED } });

	assert.deepEqual(input.bound(), { one: A_KEY, other: THE_SAME_KEY_SHIFTED });
});

// **Bindings are the user's** (ADR-0021), so rebinding is an ordinary act rather than a
// deployment setting — and the two refusals are the seam's, because they are facts about how
// a key behaves rather than about how a console looks.
test('a rebind takes, and the old key stops keying', () => {
	const there = aWindow();
	const { wanted, input } = watching({ on: there.on });
	input.available(true);

	assert.equal(input.rebind('anything', { code: 'KeyF' }), null);

	there.press(key('Backquote'));
	assert.deepEqual(wanted, [], 'the key that was rebound away still keys');

	there.press(key('KeyF'));
	assert.deepEqual(answers(wanted), [true]);
	assert.deepEqual(input.bound(), { anything: { code: 'KeyF' } });
});

test('a rebind under a held key does not leave it held', () => {
	const there = aWindow();
	const { wanted, input } = watching({ on: there.on });
	input.available(true);
	there.press(key('Backquote'));

	input.rebind('anything', { code: 'KeyF' });

	assert.deepEqual(answers(wanted), [true, false], 'the old key was still holding the new one');
});

test('Space and Caps Lock are refused, and say why', async (t) => {
	for (const [code, why] of [
		['Space', /activates whatever control has focus/],
		['CapsLock', /release/]
	]) {
		await t.test(code, () => {
			const { input } = watching();

			const no = input.rebind('anything', { code });

			assert.match(no ?? '', why);
			assert.deepEqual(input.bound(), { anything: A_KEY }, `${code} was bound anyway`);
		});
	}
});

// A modifier is derived state and derived state loses releases (ADR-0022). It is the thing
// held beside a key rather than the key itself.
test('a modifier on its own is refused', () => {
	const { input } = watching();

	assert.match(input.rebind('anything', { code: 'ShiftLeft' }) ?? '', /modifier/i);
});

// Two names on one key is one press meaning two things — which, with the modes above, is a
// press that both opens and closes. That is the derived latch ADR-0022 refuses, arriving by
// the back door of a rebind.
test('a key already bound to something else is refused', () => {
	const { input } = watching({ bindings: { one: A_KEY, other: { code: 'KeyF' } } });

	assert.match(input.rebind('other', A_KEY) ?? '', /already in use/);
	assert.equal(input.rebind('other', THE_SAME_KEY_SHIFTED), null, 'a modifier apart is one key');
	assert.equal(input.rebind('one', A_KEY), null, 'a name clashed with itself');
});

test('a binding is read off a press, and said the way somebody would say it', () => {
	assert.deepEqual(fromEvent(key('Backquote', { shift: true })), {
		code: 'Backquote',
		shift: true,
		ctrl: false,
		alt: false,
		meta: false
	});

	assert.equal(describe(A_KEY), '`');
	assert.equal(describe(THE_SAME_KEY_SHIFTED), 'Shift + `');
	assert.equal(describe({ code: 'KeyF', ctrl: true }), 'Ctrl + F');
	assert.equal(refusal(A_KEY), null);
});

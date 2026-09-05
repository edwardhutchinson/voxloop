// The Input seam: a level and a liveness flag, and the OR over them.
//
// ADR-0021's whole argument is about what happens when something goes wrong, and every case
// it names is here: a release that never arrives, a source that dies while it is held, a
// second source that is not live. There is no browser in these tests because there is nothing
// browser-shaped in the seam — a source publishes two booleans, and that is the point of it.
//
// `input-seam.test.js` beside this one asks the other question, which is whether the build
// refuses anybody reaching past this interface.

import assert from 'node:assert/strict';
import test from 'node:test';

import { keying } from '../src/lib/input/index.js';
import { levels } from '../src/lib/input/level.js';

/** An Input with a record of everything it has said about intent. */
function watching() {
	const wanted = [];

	return { wanted, input: keying({ onIntent: (wants) => wanted.push(wants) }) };
}

test('a control held down wants to emit, and a control released does not', () => {
	const { wanted, input } = watching();
	input.onScreen.present(true);

	input.onScreen.down();
	input.onScreen.up();

	assert.deepEqual(wanted, [true, false]);
});

// **A control that is not on screen is a source that is not there.** Emission is withdrawn on
// a lost audio path (ADR-0042), so the key control goes — and a source that is not live is not
// in the OR, whatever a pointer happens to be doing over the space it left.
test('a control that is not on screen cannot key', () => {
	const { wanted, input } = watching();

	input.onScreen.down();

	assert.deepEqual(wanted, [], 'a control nobody can see keyed');
});

// **The failure the level exists to prevent.** A key control that vanishes under a held
// pointer delivers no release: under an event-shaped interface the transmission hangs, the
// microphone stays open, and the server goes on telling everybody that a session with no audio
// path is transmitting. Here the source stops being live, leaves the OR, and the key drops.
test('a control that goes while it is held drops the key', () => {
	const { wanted, input } = watching();
	input.onScreen.present(true);
	input.onScreen.down();
	assert.deepEqual(wanted, [true]);

	input.onScreen.present(false);

	assert.deepEqual(wanted, [true, false], 'a control that vanished left the key held');
});

// **And it does not come back on its own.** v1 §7's rule for the other end of an outage is
// that a source which was high across a withdrawal contributes nothing until it goes low and
// high again; a control that returned still holding what it held would be a transmission
// starting at a moment nobody chose.
test('a control that comes back is not still holding what it held', () => {
	const { wanted, input } = watching();
	input.onScreen.present(true);
	input.onScreen.down();
	input.onScreen.present(false);

	input.onScreen.present(true);

	assert.deepEqual(wanted, [true, false], 'the key came back without a hand on it');
});

// **The answer moves when it moves.** A level is sampled rather than counted, so a caller
// told the same thing twice would be a caller that had to remember what it was last told in
// order to act on it — which is how a signal ends up sent per sample.
test('saying the same thing twice is said once', () => {
	const { wanted, input } = watching();
	input.onScreen.present(true);

	input.onScreen.down();
	input.onScreen.down();
	input.onScreen.up();
	input.onScreen.up();

	assert.deepEqual(wanted, [true, false]);
});

// **Sources are additive and the client ORs the live ones** (ADR-0021). One source is what v1
// ships, and the OR is asserted with a second because that is the promise the Tauri wrapper
// is owed: it adds a source and changes nothing else (ADR-0020).
test('any live source wanting to emit is enough', () => {
	const wanted = [];
	// Two sources, registered the way a source inside the seam registers. This is the reading
	// itself rather than the assembled seam, because v1 ships one source and the promise the
	// wrapper is owed is about what happens when there are two (ADR-0020).
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
	const reading = levels({ onIntent: (wants) => wanted.push(wants) });
	const footswitch = reading.add('a footswitch');
	footswitch.publish(true, true);
	assert.deepEqual(wanted, [true]);

	footswitch.publish(true, false);

	assert.deepEqual(wanted, [true, false], 'a source that vanished left the key held');
});

// **A source never knows which emission mode it serves** (ADR-0021, ADR-0022). Mode logic is
// #42's and lives above this line; a source that decided its own could latch by accident,
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

	assert.deepEqual(Object.keys(input), ['onScreen']);
});

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

	input.onScreen.down();
	input.onScreen.up();

	assert.deepEqual(wanted, [true, false]);
});

// **The answer moves when it moves.** A level is sampled rather than counted, so a caller
// told the same thing twice would be a caller that had to remember what it was last told in
// order to act on it — which is how a signal ends up sent per sample.
test('saying the same thing twice is said once', () => {
	const { wanted, input } = watching();

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

// **A source that dies while keyed forces an unkey.** The failure this prevents is the whole
// reason the seam is a level: under an event-shaped interface there is no release to deliver,
// and the transmission hangs.
test('a source that dies while it is held drops the key', () => {
	const wanted = [];
	const reading = levels({ onIntent: (wants) => wanted.push(wants) });
	const footswitch = reading.add('a footswitch');
	footswitch.publish(true, true);
	assert.deepEqual(wanted, [true]);

	footswitch.gone();

	assert.deepEqual(wanted, [true, false], 'a source that vanished left the key held');
});

// The console asks Input which sources are live so it can say **why** keying is unavailable,
// rather than drawing a control that does nothing (ADR-0016).
test('Input says which sources are live, by name', () => {
	const { input } = watching();

	assert.deepEqual(input.live(), ['the key control']);

	input.onScreen.gone();
	assert.deepEqual(input.live(), []);
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

	assert.deepEqual(Object.keys(input).sort(), ['live', 'onScreen']);
});

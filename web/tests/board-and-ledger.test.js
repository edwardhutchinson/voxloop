// Two views of one loop list (ADR-0032), and the properties that make them one list rather
// than two lists that happen to agree today.
//
// **A state that renders in only one view is a bug** from this ticket onward, and that is
// not something a reviewer can hold in their head as the document grows a field a ticket.
// So every question here is asked of both views at once: does each loop in the document
// appear, in the same order, and does the transmit bar say the same words in both.
//
// The one thing not asked here is what a click does, because at this point nothing is
// clickable — subscription is #39 — and because it is what a browser does rather than what
// a component renders.

import assert from 'node:assert/strict';
import test from 'node:test';
import { join } from 'node:path';
import { read, src } from './console.js';
import { rendered } from './render.js';

const lib = join(src, 'lib');
const views = ['Board.svelte', 'Ledger.svelte'];

// Deliberately neither alphabetical nor sorted by id: the base loop order is administered
// rather than derived (ADR-0053), so a view quietly sorting what it was handed has to fail.
const inReach = [
	{ id: 'l-3', name: 'THERMAL', permission: 'control' },
	{ id: 'l-1', name: 'FLIGHT', permission: 'emit' },
	{ id: 'l-2', name: 'GNC', permission: 'monitor' }
];

const namesOf = (loops) => loops.map((reachable) => reachable.name);

/** Where each of these names first appears, in the order the page has them. */
const asShown = (body, names) =>
	[...names].sort((one, other) => body.indexOf(one) - body.indexOf(other));

const eachView = (props) => Promise.all(views.map((view) => rendered(view, props)));

// The media path as the presence document carries it (ADR-0042). Every question below is
// asked with one of these rather than with nothing, because the document always has one and
// a view rendered without it would be a view no session ever sees.
const carrying = { loops: inReach, mediaPath: 'connected' };

test('both views render every loop in the document', async () => {
	for (const [at, body] of (await eachView({ loops: inReach })).entries()) {
		for (const name of namesOf(inReach)) {
			assert.match(body, new RegExp(name), `${views[at]} left out ${name}`);
		}
	}
});

test('both views hold the loops in one order', async () => {
	for (const [at, body] of (await eachView({ loops: inReach })).entries()) {
		assert.deepEqual(
			asShown(body, namesOf(inReach)),
			namesOf(inReach),
			`${views[at]} shows the loops in an order of its own`
		);
	}
});

// One order lives above both views and is handed to each of them, so there is nowhere for a
// second one to be kept. Reordering it — #55 makes it personal — reorders both by
// construction rather than by two implementations agreeing.
test('neither view has an order of its own to keep', async () => {
	for (const view of views) {
		assert.doesNotMatch(
			read(join(lib, view)),
			/\.(sort|toSorted|reverse|toReversed)\(/,
			`${view} orders the loops itself — the order is one thing, held above both views`
		);
	}
});

test('a loop that leaves reach leaves both views', async () => {
	const [left] = namesOf(inReach);
	const stillThere = inReach.slice(1);

	for (const [at, body] of (await eachView({ loops: stillThere })).entries()) {
		assert.doesNotMatch(body, new RegExp(left), `${views[at]} still shows ${left}`);
		for (const name of namesOf(stillThere)) assert.match(body, new RegExp(name));
	}
});

test('an empty reach is a view with no loops rather than no view', async () => {
	for (const [at, body] of (await eachView({ ...carrying, loops: [] })).entries()) {
		assert.ok(
			body.includes(await rendered('TransmitBar.svelte', { mediaPath: 'connected' })),
			`${views[at]} lost its bar`
		);
	}
});

// A card cannot hold a sentence and the ledger is where a sentence lives (ADR-0032). The
// permission is the only state either view carries at this point, so it is where the division
// of labour is established: the rung as a word on the board, what it confers in the ledger.
test('the board says a word where the ledger says a sentence', async () => {
	const [board, ledger] = await eachView({ loops: inReach });

	for (const { permission } of inReach) assert.match(board, new RegExp(permission));
	assert.match(ledger, /speak on it/);
});

// Every state the bar has, in both views, rather than one of them: the bar is the whole of
// VoxLoop's compensation for emitting to several places at once, and a board and a ledger
// disagreeing about whether an operator can be heard would be worse than neither saying
// anything.
test('the transmit bar is in both views, worded identically, in every state it has', async () => {
	for (const mediaPath of ['connected', 'impaired', 'lost']) {
		const bar = await rendered('TransmitBar.svelte', { mediaPath });

		// The same bytes, because it is the same component: two views cannot word one bar
		// differently if neither of them writes the wording.
		for (const [at, body] of (await eachView({ loops: inReach, mediaPath })).entries()) {
			assert.ok(body.includes(bar), `${views[at]} does not carry the ${mediaPath} bar`);
		}
	}
});

// **Emission has two independent withdrawal conditions and the bar must say which**
// (ADR-0042, v1 §6): a lost state channel and a lost audio path are different problems with
// different fixes, and one wording for both sends an operator to look at the wrong thing.
test('the bar tells a lost audio path apart from a lost connection, in both views', async () => {
	for (const [at, body] of (await eachView({ loops: inReach, mediaPath: 'lost' })).entries()) {
		assert.match(body, /audio path/, `${views[at]} does not say what is missing`);
		assert.match(body, /will not emit/, `${views[at]} does not say emission is withdrawn`);
	}
});

// Three rungs, three things to say. `impaired` is a transient fault that routinely clears
// itself and emission stands through it, so a bar that read it the same way as `lost` would
// cut audio for a reroute that heals — which is the whole reason the middle rung exists.
test('each rung of the media path says something the others do not', async () => {
	const said = await Promise.all(
		['connected', 'impaired', 'lost'].map((mediaPath) =>
			rendered('TransmitBar.svelte', { mediaPath })
		)
	);

	assert.equal(new Set(said).size, 3, 'two rungs of the media path read alike');
	assert.match(said[1], /still stands/);
	assert.match(said[2], /will not emit/);
});

// The document always carries one of the three, so anything else is not a state the console
// has been told about — and a bar that cannot tell what the audio path is doing has no
// business offering a key control over it.
test('a media path the console has no reading of withdraws emission', async () => {
	assert.equal(
		await rendered('TransmitBar.svelte', { mediaPath: undefined }),
		await rendered('TransmitBar.svelte', { mediaPath: 'lost' })
	);
});

test('the transmit bar cannot be scrolled away in either view', async () => {
	for (const view of views) {
		// It is positioned by the view that holds it, because ADR-0034 puts it in a different
		// place in each; what neither may do is let it scroll off with the loops.
		assert.match(
			read(join(lib, view)),
			/\.transmit\s*\{[^}]*position:\s*(fixed|sticky)/,
			`${view} lets the transmit bar scroll away`
		);
	}
});

// Optimistic rendering is banned outright (ADR-0016), and the shape that enforces it is that
// nothing rendering the document has anywhere to put a state of its own: what is on screen
// came out of the last document and can be nothing else. It is also what makes switching
// views lose nothing — there is nothing held in either of them to lose.
test('nothing that renders the presence document keeps state of its own', async () => {
	for (const held of [...views, 'TransmitBar.svelte']) {
		const source = read(join(lib, held));

		for (const rune of ['$state(', '$effect(']) {
			assert.ok(
				!source.includes(rune),
				`${held} keeps ${rune} of its own — the document is the only thing it may render`
			);
		}
	}
});

test('the operating console remembers which view is on screen, and nothing else', async () => {
	const source = read(join(lib, 'Console.svelte'));

	assert.equal(
		source.match(/\$state\(/g)?.length,
		1,
		'the console keeps state beyond which view is showing — every other fact is the server’s'
	);
});

test('the console offers both views and opens on the board', async () => {
	const body = await rendered('Console.svelte', {
		presence: {
			session: 'a-session',
			role: { id: 'r-1', name: 'Flight Director' },
			media_path: 'connected',
			loops: inReach
		}
	});

	assert.match(body, /Board/);
	assert.match(body, /Ledger/);
	// The board is the view a control room reads at a glance, and it is what the operator
	// wanted; which view somebody lands in becomes theirs with #55.
	assert.ok(body.includes(await rendered('Board.svelte', carrying)));
});

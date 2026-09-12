// Two views of one loop list (ADR-0032), and the properties that make them one list rather
// than two lists that happen to agree today.
//
// **A state that renders in only one view is a bug** from this ticket onward, and that is
// not something a reviewer can hold in their head as the document grows a field a ticket.
// So every question here is asked of both views at once: does each loop in the document
// appear, in the same order, and does the transmit bar say the same words in both.
//
// What a click *does* is not asked here, because that is what a browser does rather than
// what a component renders. What is asked is where the click target sits and who decides
// what a click means, because both are structural and both are how the two views are kept
// from coming to disagree.

import assert from 'node:assert/strict';
import test from 'node:test';
import { join } from 'node:path';
import { read, src } from './console.js';
import { rendered } from './render.js';

const lib = join(src, 'lib');
const views = ['Board.svelte', 'Ledger.svelte'];

// Deliberately neither alphabetical nor sorted by id: the base loop order is administered
// rather than derived (ADR-0053), so a view quietly sorting what it was handed has to fail.
//
// Two of the three are being monitored and one is not, because **subscription is distinct
// from permission** (v1 §5): every loop here is one the role may monitor, and that says
// nothing about which of them it currently is.
//
// `FLIGHT` is armed and not monitored, which is a **blind arm** — legal, because arming is
// independent of subscription (ADR-0013), and the state the console has to name in words. It
// is also being spoken on, which is what makes it the loop that proves an indicator reaches a
// console that is not hearing the loop.
//
// `THERMAL` is monitored and muted, and `GNC` is monitored and turned down to 40% (#44). The
// other loops are at unity, which is where every loop starts (v1 §10).
//
// **Loop health is per monitored loop** (#46): `THERMAL`'s beacon is arriving, muted or not,
// and `GNC`'s is not — it is the loop that sounds exactly like a quiet one and must not look
// like one. `FLIGHT` is not monitored, so there is no beacon to count and no health at all.
//
// **Staffing state is per loop and the mark is per console** (#48). This role staffs
// `THERMAL`, which is staffed, and `FLIGHT`, which is not on this console at all — the mark's
// second state, and the actionable one. `FLIGHT` is also the loop whose occupants are away
// for differing reasons, which is the count the ledger spells out. `GNC` has no staffing
// roles, so it has no staffing state: blank, and not a fourth word (ADR-0056).
const inReach = [
	{
		id: 'l-3',
		name: 'THERMAL',
		permission: 'control',
		subscribed: true,
		armed: false,
		talking: false,
		muted: true,
		volume: 100,
		health: 'receiving',
		staffing: { state: 'staffed', away: [] },
		staffs: true
	},
	{
		id: 'l-1',
		name: 'FLIGHT',
		permission: 'emit',
		subscribed: false,
		armed: true,
		talking: true,
		muted: false,
		volume: 100,
		health: null,
		staffing: {
			state: 'away',
			away: [
				{ reason: 'not-subscribed', occupants: 2 },
				{ reason: 'muted', occupants: 1 }
			]
		},
		staffs: true
	},
	{
		id: 'l-2',
		name: 'GNC',
		permission: 'monitor',
		subscribed: true,
		armed: false,
		talking: false,
		muted: false,
		volume: 40,
		health: 'not-receiving',
		staffing: null,
		staffs: false
	}
];

const namesOf = (loops) => loops.map((reachable) => reachable.name);

/**
 * A view without its transmit bar: the loop list, and only that.
 *
 * The bar names loops too — it carries the armed set in words (ADR-0034) — and it is above the
 * rows in one view and below the cards in the other, so a question about the order **of the
 * list** has to ask it of the list. Everything else here reads the whole body, because the bar
 * being in it is usually the point.
 */
const theLoopList = (body) =>
	body.replace(/<section aria-label="Transmit bar">[\s\S]*?<\/section>/, '');

/** Where each of these names first appears, in the order the page has them. */
const asShown = (body, names) =>
	[...names].sort((one, other) => body.indexOf(one) - body.indexOf(other));

const eachView = (props) => Promise.all(views.map((view) => rendered(view, props)));

// A view as a session with a working audio path sees it. The media path is passed rather
// than left out wherever a question is not about it, because the document always carries one
// (ADR-0042) and a view rendered without it is a view no session ever sees.
const theBar = {
	mediaPath: 'connected',
	connection: 'confirmed',
	armedOn: ['FLIGHT'],
	keyed: false,
	mayKey: true
};

const carrying = { loops: inReach, bar: theBar };

/**
 * A view carrying a bar in whatever state the test is about.
 *
 * The bar reaches a view as one value it has no name for any part of (ADR-0034), so a test
 * that wants the bar in a particular state says so here rather than by handing a view a prop
 * the view could not have read.
 */
const inAView = (bar, loops = inReach) => ({ loops, bar: { ...theBar, ...bar } });

/**
 * The whole console, on a healthy channel.
 *
 * Where this tab stands with the signalling channel is the one state on the page the server
 * did not say (ADR-0018), so it is a prop of the console like the document is — and a test
 * that left it out would be testing a console with no reading of its own channel.
 */
const theConsole = {
	presence: {
		session: 'a-session',
		role: { id: 'r-1', name: 'Flight Director' },
		media_path: 'connected',
		connection: 'confirmed',
		loops: inReach
	},
	connection: { state: 'confirmed', since: 0, aLatchStands: true }
};

test('both views render every loop in the document', async () => {
	for (const [at, body] of (await eachView(carrying)).entries()) {
		for (const name of namesOf(inReach)) {
			assert.match(body, new RegExp(name), `${views[at]} left out ${name}`);
		}
	}
});

test('both views hold the loops in one order', async () => {
	for (const [at, body] of (await eachView(carrying)).entries()) {
		assert.deepEqual(
			asShown(theLoopList(body), namesOf(inReach)),
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

	for (const [at, body] of (await eachView(inAView({}, stillThere))).entries()) {
		assert.doesNotMatch(body, new RegExp(left), `${views[at]} still shows ${left}`);
		for (const name of namesOf(stillThere)) assert.match(body, new RegExp(name));
	}
});

test('an empty reach is a view with no loops rather than no view', async () => {
	for (const [at, body] of (await eachView(inAView({ armedOn: [] }, []))).entries()) {
		assert.ok(
			body.includes(await rendered('TransmitBar.svelte', { ...theBar, armedOn: [] })),
			`${views[at]} lost its bar`
		);
	}
});

// A card cannot hold a sentence and the ledger is where a sentence lives (ADR-0032). The
// permission is the only state either view carries at this point, so it is where the division
// of labour is established: the rung as a word on the board, what it confers in the ledger.
test('the board says a word where the ledger says a sentence', async () => {
	const [board, ledger] = await eachView(carrying);

	for (const { permission } of inReach) assert.match(board, new RegExp(permission));
	assert.match(ledger, /speak on it/);
});

// Every state the bar has, in both views, rather than one of them: the bar is the whole of
// VoxLoop's compensation for emitting to several places at once, and a board and a ledger
// disagreeing about whether an operator can be heard would be worse than neither saying
// anything.
test('the transmit bar is in both views, worded identically, in every state it has', async () => {
	for (const mediaPath of ['connected', 'impaired', 'lost']) {
		const carried = { ...theBar, mediaPath, mayKey: mediaPath !== 'lost' };
		const bar = await rendered('TransmitBar.svelte', carried);

		// The same bytes, because it is the same component: two views cannot word one bar
		// differently if neither of them writes the wording.
		for (const [at, body] of (await eachView(inAView(carried))).entries()) {
			assert.ok(body.includes(bar), `${views[at]} does not carry the ${mediaPath} bar`);
		}
	}
});

// **Emission has two independent withdrawal conditions and the bar must say which**
// (ADR-0042, v1 §6): a lost state channel and a lost audio path are different problems with
// different fixes, and one wording for both sends an operator to look at the wrong thing.
test('the bar tells a lost audio path apart from a lost connection, in both views', async () => {
	for (const [at, body] of (
		await eachView(inAView({ mediaPath: 'lost', mayKey: false }))
	).entries()) {
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
			rendered('TransmitBar.svelte', { mediaPath, mayKey: mediaPath !== 'lost' })
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
	// And the console reads it the same way, because there is one derivation above both views
	// rather than one in each — a rung nobody has a reading of leaves emission withdrawn.
	assert.match(
		read(join(lib, 'Console.svelte')),
		/media_path === 'connected' \|\| presence\.media_path === 'impaired'/,
		'the console decides whether emission stands somewhere other than from the ladder'
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

// The console itself keeps these things, and each is named here so that one more has to be
// argued for in a diff a reviewer reads. Which view is showing, and which loop's volume the
// operator has opened, are facts about the reader; whether this desk's audio output has moved
// under the operator is a fact about the hardware in front of them, which no server can see
// (ADR-0017);
// the latch, the source that went while it was held, and a latch taken down by something
// other than the operator are facts about the input on this desk, knowable here and true the
// moment they are said (ADR-0016, ADR-0021, ADR-0018). Nothing about the world is among them
// — that all arrives in the presence document, and where this tab stands with the channel
// arrives as a prop from the one place that measures it.
test('the operating console keeps only what is not the server’s to say', async () => {
	const source = read(join(lib, 'Console.svelte'));

	const kept = [...source.matchAll(/let (\w+) = \$state\(/g)].map(([, named]) => named);

	assert.deepEqual(
		kept.toSorted(),
		['bound', 'dropped', 'latchDropped', 'latched', 'moved', 'showing', 'volumeOpenFor'],
		'the console keeps a state of its own — every fact about the world is the server’s'
	);
});

// **A state that renders in only one view is a bug** (v1 §8), and subscription is the first
// state after the rung to prove it. The board carries it as a word and the ledger as a
// sentence, which is the division of labour ADR-0032 keeps both views for.
test('both views say whether each loop is being monitored', async () => {
	const [board, ledger] = await eachView(carrying);

	// Anchored on the tags, because `Not monitoring` contains `monitoring`: an unanchored
	// match for the monitored state is satisfied by the unmonitored one and cannot fail.
	assert.match(board, />Not monitoring</, 'the board does not say a loop is unmonitored');
	assert.match(board, />Monitoring</, 'the board does not say a loop is monitored');
	assert.match(ledger, /You are hearing this loop\./);
	assert.match(ledger, /You are not hearing this loop\./);
});

// Colour is never the only thing carrying a state, and neither is a border or an attribute:
// a view whose two states differ only in CSS says nothing to somebody reading it in a
// photograph, in high contrast, or out loud.
test('neither view carries the subscription in anything but words', async () => {
	const every = (subscribed) => inReach.map((reachable) => ({ ...reachable, subscribed }));
	const monitored = await eachView(inAView({}, every(true)));
	const not = await eachView(inAView({}, every(false)));
	// The markup with every attribute value taken out: what is left is what a person reads.
	const words = (body) => body.replaceAll(/="[^"]*"/g, '');

	for (const [at, body] of monitored.entries()) {
		assert.notEqual(
			words(body),
			words(not[at]),
			`${views[at]} reads identically whether or not the loops are monitored`
		);
	}
});

// **Clicking the card body toggles monitoring, and arm, mute and cog must not propagate that
// click** (v1 §8, ADR-0032). The rule is kept structurally rather than by remembering to stop
// propagation in two later tickets: the click target is a `<button>` inside the card rather
// than the card itself, and a `<button>` cannot contain another control — so anything #41 and
// #44 add to a card is a sibling of the body and never inside it.
test('the board toggles from a control inside the card, not from the card', async () => {
	const source = read(join(lib, 'Board.svelte'));
	const card = source.slice(source.indexOf('<li'), source.indexOf('</li>'));

	assert.doesNotMatch(
		card.slice(0, card.indexOf('>')),
		/onclick/,
		'the card itself is the click target, so every control added to it will propagate'
	);
	assert.match(card, /<button[^>]*onclick=/, 'the card body is not a control');

	const body = await rendered('Board.svelte', carrying);
	assert.match(body, /<button[^>]*aria-pressed="true"/, 'a monitored card is not marked as on');
	assert.match(
		body,
		/<button[^>]*aria-pressed="false"/,
		'an unmonitored card is not marked as off'
	);
});

// **Two acts on the wire, one decision, held above both views** (ADR-0016). Which of
// subscribe and unsubscribe a click is comes from the document, so a view deciding for
// itself would be reasoning from a state the server has not confirmed — and two views
// deciding separately is how they come to disagree.
test('neither view decides which act a click is', async () => {
	for (const view of views) {
		assert.doesNotMatch(
			read(join(lib, view)),
			/\bunsubscribe\b/i,
			`${view} picks the act itself — the document is what says which one a click is`
		);
	}
});

test('the console hands both views the same toggle', async () => {
	const source = read(join(lib, 'Console.svelte'));

	assert.equal(
		source.match(/onToggle=\{toggle\}/g)?.length,
		2,
		'the two views are not handed one toggle'
	);
	// The branch is on the document's own field, and both acts are named here and nowhere
	// else — which is what makes *which act is this* one decision rather than two.
	assert.match(source, /\.subscribed\b/, 'the toggle does not read the document');
	for (const act of ['onSubscribe', 'onUnsubscribe']) {
		assert.match(source, new RegExp(`${act}\\(`), `the console never calls ${act}`);
	}
});

// ---- Arming, keying and the talking indicator (#41) ---------------------------------------

// **A state that renders in only one view is a bug** (v1 §8), and the arm is the state this
// ticket adds. The board says it as a word and the ledger as a sentence, which is the division
// of labour ADR-0032 keeps both views for.
test('both views say whether each loop is armed', async () => {
	const [board, ledger] = await eachView(carrying);

	assert.match(board, />\s*Not armed\s*</, 'the board does not say a loop is unarmed');
	assert.match(board, />\s*Armed/, 'the board does not say a loop is armed');
	assert.match(ledger, /Your voice goes here/);
	assert.match(ledger, /Your voice does not go here\./);
});

// **A blind arm is named in words** (v1 §4, §8): armed and not monitored is legal, and the
// console compensating for it is the whole of what makes emitting blind safe to allow.
test('both views name a blind arm in words', async () => {
	const [board, ledger] = await eachView(carrying);

	assert.match(board, /Armed, not hearing it/, 'the board does not name the blind arm');
	assert.match(
		ledger,
		/Your voice goes here and you are not hearing it\./,
		'the ledger does not name the blind arm'
	);
});

// **Reach is the grid and only the grid.** A role that may hear a loop and not speak on it is
// offered nothing to press, rather than a control that is refused when it is used — a console
// may not misrepresent what a person can do (ADR-0016).
test('neither view offers an arm on a loop this role may only monitor', async () => {
	const monitorOnly = [{ ...inReach[2] }];
	const [board, ledger] = await eachView(inAView({ armedOn: [] }, monitorOnly));

	assert.doesNotMatch(board, />\s*Arm\s*</, 'the board offers an arm on a loop it may not emit on');
	assert.doesNotMatch(
		ledger,
		/>\s*Arm\s*</,
		'the ledger offers an arm on a loop it may not emit on'
	);
	assert.match(ledger, /This role may not speak on this loop\./);
});

// **The console shows that a loop is being spoken on and never who** (ADR-0033). It is one
// component, so both views carry the same indicator by construction rather than by two
// implementations agreeing — and there is nothing in it that could name anybody.
test('the talking indicator is the same indicator in both views and names nobody', async () => {
	const indicator = await rendered('Talking.svelte');
	const [board, ledger] = await eachView(carrying);

	for (const [at, body] of [board, ledger].entries()) {
		assert.ok(body.includes(indicator), `${views[at]} does not carry the talking indicator`);
	}

	// One loop is being spoken on and two are not, so the indicator appears once in each.
	for (const [at, body] of [board, ledger].entries()) {
		assert.equal(
			body.split(indicator).length - 1,
			1,
			`${views[at]} marks the wrong number of loops as being spoken on`
		);
	}

	// It says so in a word as well as in the glyph, because colour and motion are never what
	// carries a state.
	assert.match(indicator, /Talking/);
});

// It reaches a console that is not monitoring the loop, which is what makes it the
// compensation v1 §4 asks for: the operator arming blind can still see they are about to talk
// over somebody.
test('a loop being spoken on is marked whether or not this console is hearing it', async () => {
	const indicator = await rendered('Talking.svelte');
	const blind = inReach.map((reachable) => ({ ...reachable, subscribed: false }));

	for (const [at, body] of (await eachView(inAView({}, blind))).entries()) {
		assert.ok(body.includes(indicator), `${views[at]} shows the mark only where a loop is heard`);
	}
});

// **The armed set in words, and the same words in both views** (ADR-0034). It is the half of
// the bar an operator acts on: the second before keying is spent reading where their voice is
// about to go, and a count does not answer that.
test('the transmit bar carries the armed set in words, identically in both views', async () => {
	const bar = await rendered('TransmitBar.svelte', {
		mediaPath: 'connected',
		mayKey: true,
		armedOn: ['FLIGHT', 'SIM']
	});

	assert.match(bar, /Armed on FLIGHT and SIM\./);

	for (const [at, body] of (await eachView(inAView({ armedOn: ['FLIGHT', 'SIM'] }))).entries()) {
		assert.ok(body.includes(bar), `${views[at]} does not carry the armed set as the other does`);
	}
});

// **At zero armed the key control renders differently rather than going away** (v1 §8). A
// revocation can empty the arm set under somebody mid-sentence, and taking the control out of
// their hand is a bigger lie than showing them that it reaches nobody.
test('the key control says so when it reaches nobody, and still keys', async () => {
	const nothing = await rendered('TransmitBar.svelte', {
		mediaPath: 'connected',
		mayKey: true,
		armedOn: []
	});
	const something = await rendered('TransmitBar.svelte', {
		mediaPath: 'connected',
		mayKey: true,
		armedOn: ['FLIGHT']
	});

	assert.match(nothing, /Armed on nothing\./);
	assert.match(nothing, /reaching nobody/);
	assert.notEqual(nothing, something, 'the key control reads the same at zero armed');
	// It is still there, and it is still a control.
	assert.match(nothing, /<button class="key[ "]/);
});

// **The transmitting lamp is lit by the server's acknowledgement, never by the button going
// down** (ADR-0008). The bar keeps no state of its own — the test above says so — so the only
// thing that can light it is the document's `keyed`, and this is that being true.
test('the transmitting lamp is the document’s answer and nothing else', async () => {
	const lit = await rendered('TransmitBar.svelte', {
		mediaPath: 'connected',
		mayKey: true,
		armedOn: ['FLIGHT'],
		keyed: true
	});
	const unlit = await rendered('TransmitBar.svelte', {
		mediaPath: 'connected',
		mayKey: true,
		armedOn: ['FLIGHT'],
		keyed: false
	});

	// `Keyed`, not `Transmitting`: the glossary avoids the second for the act (CONTEXT.md,
	// Keying), and the console speaks the glossary's language.
	assert.match(lit, />\s*Keyed\s*</);
	assert.match(unlit, />\s*Not keyed\s*</);
	assert.match(lit, /<button[^>]*aria-pressed="true"/);
	assert.match(unlit, /<button[^>]*aria-pressed="false"/);

	// And the source has nowhere to pre-light it from: there is no local key state in the bar
	// and no handler that sets one.
	const source = read(join(lib, 'TransmitBar.svelte'));
	assert.ok(!source.includes('$state('), 'the transmit bar keeps a key state of its own');
});

// Emission is withdrawn on a lost audio path (ADR-0042), so there is no key control over one.
test('a lost audio path leaves no key control to press', async () => {
	const lost = await rendered('TransmitBar.svelte', {
		mediaPath: 'lost',
		mayKey: false,
		armedOn: ['FLIGHT']
	});

	assert.doesNotMatch(lost, /<button/, 'a console with no audio path was offered a key control');
	assert.match(lost, /will not emit/);
	// **The armed set stands whatever the audio path is doing** (ADR-0034). The bar answers
	// *who am I about to talk to*, and an operator whose path has just dropped is owed that
	// answer more than anybody: it is what they are coming back to.
	assert.match(lost, /Armed on FLIGHT\./, 'a withdrawn path took the armed set with it');
});

// **The control going is the source dying** (ADR-0021), and this is what makes that true: the
// console tells Input whether the control is on screen, from the same answer the bar draws it
// from. Without it a path that dropped under a held pointer would deliver no release, and the
// key would hang — the open mic the level was chosen to prevent.
test('the console tells Input when keying does not stand', async () => {
	const source = read(join(lib, 'Console.svelte'));

	assert.match(
		source,
		/keys\.available\(mayKey\)/,
		'a source can go under a held key without Input hearing about it'
	);
	// One derivation, and it reaches the bar in the one value both views carry — so the control
	// the operator sees and the source Input reads can never disagree about whether it is
	// there, and neither view has a name for it to get wrong.
	assert.equal(source.match(/\bmayKey,/g)?.length, 1, 'the two views are not handed one answer');
});

// **Two acts rather than one toggle, and the decision is held above both views**, exactly as
// it is for monitoring: which of arm and disarm a press is comes from the document, and two
// views deciding separately is how they come to disagree.
test('neither view decides whether a press is an arm or a disarm', async () => {
	for (const view of views) {
		assert.doesNotMatch(
			read(join(lib, view)),
			/\bonDisarm\b/,
			`${view} picks the act itself — the document is what says which one a press is`
		);
	}

	const source = read(join(lib, 'Console.svelte'));
	assert.equal(
		source.match(/onArm=\{arming\}/g)?.length,
		2,
		'the two views are not handed one arm'
	);
	assert.match(source, /\.armed\b/, 'the arm decision does not read the document');
	for (const act of ['onArm', 'onDisarm']) {
		assert.match(source, new RegExp(`${act}\\(`), `the console never calls ${act}`);
	}
});

// **Input is a seam** (ADR-0021, ADR-0061), and the console is the only thing above it. A view
// reaching a source directly would be the seam becoming a directory, which the lint rule
// refuses — and a view that ORs its own sources would be mode logic below the line.
test('the views know nothing about where a key press comes from', async () => {
	for (const view of views) {
		const source = read(join(lib, view));

		assert.doesNotMatch(
			source,
			/\$lib\/input/,
			`${view} reaches Input itself — the console is the only thing above that seam`
		);
		// Nor the modes above it: a view that knew which of the two a button served would be
		// a view that could decide, and deciding is what makes a latch derived (ADR-0022).
		assert.doesNotMatch(
			source,
			/modes\.js/,
			`${view} knows which emission mode a control serves — that is settled above it`
		);
	}

	// The console reads the modes and the modes read the seam, and there is no other way in
	// to either (ADR-0061).
	assert.match(
		read(join(lib, 'Console.svelte')),
		/from '\.\/modes\.js'/,
		'the console does not go through the modes'
	);
	assert.match(
		read(join(lib, 'modes.js')),
		/from '\.\/input\/index\.js'/,
		'the modes do not go through the Input seam'
	);
});

test('the console offers both views and opens on the board', async () => {
	const body = await rendered('Console.svelte', theConsole);

	assert.match(body, /Board/);
	assert.match(body, /Ledger/);
	// The board is the view a control room reads at a glance, and it is what the operator
	// wanted; which view somebody lands in becomes theirs with #55.
	assert.ok(body.includes(await rendered('Board.svelte', carrying)));
});

// ---- The emission modes (#42) --------------------------------------------------------------

// **Two modes and no third** (v1 §4), and two controls because they are two bindings
// (ADR-0022). Latch is never reached by how the key control was pressed, so the console
// offers a second control rather than a gesture — and a state that renders in only one view
// is a bug, so both of them carry it.
test('both views offer both ways to talk, and each names its act', async () => {
	for (const [at, body] of (await eachView(carrying)).entries()) {
		assert.match(body, />\s*Key\s*</, `${views[at]} offers no key control`);
		assert.match(body, />\s*Latch\s*</, `${views[at]} offers no latch`);
	}
});

// The button names the act and the sentence says what is true, the way every control on the
// console does: neither has to be read as the other, and the state is never carried by
// `aria-pressed` alone.
test('a latched key says so in words in both views', async () => {
	const latched = await eachView(inAView({ latched: true }));
	const not = await eachView(carrying);

	for (const [at, body] of latched.entries()) {
		assert.match(
			body,
			/You have latched the key open\./,
			`${views[at]} does not say it is latched`
		);
		assert.match(body, />\s*Unlatch\s*</, `${views[at]} offers no way out of a latch`);
		assert.doesNotMatch(not[at], /You have latched the key open\./);
	}
});

// **The latch is the console's own state and the lamp is the server's** (ADR-0008, ADR-0016).
// Latching lights nothing: the lamp is `keyed` out of the presence document, and a bar that
// pre-lit it from a local press would be exactly the optimistic rendering the standing
// requirement bans.
test('latching lights no lamp', async () => {
	const body = await rendered('TransmitBar.svelte', {
		mediaPath: 'connected',
		mayKey: true,
		armedOn: ['FLIGHT'],
		latched: true,
		keyed: false
	});

	assert.match(body, />\s*Not keyed\s*</, 'the lamp lit itself off a local latch');
});

// **A source that dies while keyed forces an unkey and says so locally** (ADR-0021), and it
// names the source: *the key control went* and *your keyboard went* send an operator to look
// at two different things.
test('both views say when a source went while the key was held', async () => {
	const dropped = await eachView(inAView({ dropped: 'the keyboard' }));

	for (const [at, body] of dropped.entries()) {
		assert.match(
			body,
			/still holding the keyboard/,
			`${views[at]} says nothing about a key that dropped under its own operator`
		);
	}
});

// The case that produces it is the audio path going under a held key, which takes the key
// control away with it (ADR-0042). A notice drawn beside that control would be a notice
// nobody in that case ever reads.
test('a dropped key is said where there is no longer a key control', async () => {
	const body = await rendered('TransmitBar.svelte', {
		mediaPath: 'lost',
		mayKey: false,
		armedOn: ['FLIGHT'],
		dropped: 'the key control'
	});

	assert.match(body, /still holding the key control/);
	assert.match(body, /will not emit/, 'the withdrawal it explains went unsaid');
});

// **The console must not place a focusable control where an operator's hands rest** (v1 §4).
// A key pressed with focus on a control is refused (ADR-0022), so a key control that took
// focus when it was clicked would leave the operator's keyboard binding dead until they
// clicked elsewhere — on the two controls where that matters most, and with nothing on screen
// to explain it.
test('the keying controls do not take focus when they are pressed', async () => {
	const source = read(join(lib, 'TransmitBar.svelte'));

	const pressed = [...source.matchAll(/onpointerdown=\{(\w+)\}/g)].map(([, named]) => named);
	assert.equal(pressed.length, 3, 'the bar has grown a keying control this does not know about');

	for (const named of pressed) {
		assert.match(
			source,
			new RegExp(`const ${named} = \\(event\\) => \\{\\s*event\\.preventDefault\\(\\);`),
			`${named} lets focus land on the control an operator's hand rests on`
		);
	}
});

// **Bindings are the user's** (ADR-0021) — a keybinding is not reach — and the defaults are
// ADR-0022's: `` ` `` for the key you hold, `` Shift+` `` for the latch. Space and `CapsLock`
// are refused, and the refusal is the seam's rather than this page's.
test('the console says which keys talk, and what each of them does', async () => {
	const body = await rendered('Console.svelte', theConsole);

	assert.match(body, /Momentary/);
	assert.match(body, /Latched/);
	assert.match(body, /Shift \+ `/, 'the latch key is not on the page');
	// The third binding (ADR-0046), in the same list and changed the same way.
	assert.match(body, /Priority/);
	assert.match(body, /Ctrl \+ `/, 'the priority key is not on the page');
	assert.match(body, /Change/, 'the keys are shown and cannot be changed');
});

// ---- #43: connection state and the emission predicate ---------------------------------------

// A view whose transmit bar stands on a healthy channel, so a test about one rung of the
// signalling ladder is not also a test about the audio path.
// The bar on one rung of the signalling ladder, with a healthy audio path underneath — so a
// test about one rung is not also a test about the other axis.
const onARung = (connection) => ({
	...theBar,
	connection,
	mayKey: connection !== 'disconnected'
});

// A sentence as somebody reads it, rather than as the markup wraps it. The wording is what
// these are about, and a line break inside it is the formatter's business rather than the
// operator's.
const asRead = (body) => body.replace(/\s+/g, ' ');

// **The stale banner and the disconnected banner render in both views** (v1 §6, ADR-0018).
// The bar is the strip both views carry and never scroll away (ADR-0034), so what each rung
// costs an operator is said there — where they are standing when they need it.
test('both views say where the console stands with the signalling channel', async () => {
	for (const connection of ['unconfirmed', 'disconnected']) {
		const bar = await rendered('TransmitBar.svelte', onARung(connection));

		for (const [at, body] of (await eachView(inAView(onARung(connection)))).entries()) {
			assert.ok(body.includes(bar), `${views[at]} does not carry the ${connection} bar`);
		}
	}
});

// **The bar has to say which** of the two withdrawal conditions applies (ADR-0018, ADR-0042):
// *nobody can be told what you are doing* and *nobody can hear you* are different problems
// with different fixes, and one wording for both sends an operator to look at the wrong thing.
test('a lost channel and a lost audio path are not worded alike', async () => {
	const noChannel = await rendered('TransmitBar.svelte', onARung('disconnected'));
	const noAudio = await rendered('TransmitBar.svelte', {
		...theBar,
		mediaPath: 'lost',
		mayKey: false
	});

	assert.match(asRead(noChannel), /connection to VoxLoop rather than the audio/);
	assert.match(asRead(noAudio), /audio rather than the connection to VoxLoop/);
	assert.notEqual(noChannel, noAudio);
});

// **Push-to-talk stays live at `unconfirmed`** (ADR-0018). *We cannot confirm your
// transmission right now* is a materially different statement from *we know you are
// disconnected*, and cutting somebody off mid-word for a half-second blip is the failure the
// middle rung exists to prevent. The latch is what does not survive it, so the bar says so.
test('the middle rung keeps the key control and says the latch will not be held', async () => {
	const unconfirmed = await rendered('TransmitBar.svelte', onARung('unconfirmed'));

	assert.match(unconfirmed, />\s*Key\s*</, 'the key control went at a rung that keeps it');
	assert.match(asRead(unconfirmed), /cannot confirm what you are doing/);
	assert.match(asRead(unconfirmed), /latched key will not be held open/);
});

// Every rung says something the others do not, or an operator reading one of them learns
// nothing about which it is.
test('each rung of the signalling ladder says something the others do not', async () => {
	const said = await Promise.all(
		['confirmed', 'unconfirmed', 'disconnected'].map((connection) =>
			rendered('TransmitBar.svelte', onARung(connection))
		)
	);

	assert.equal(new Set(said).size, 3, 'two rungs of the signalling ladder read alike');
});

// **The one user-facing message in the product that does not originate at the server**
// (ADR-0018). It says what it cost rather than what caused it — the rung above it is what
// names the cause — and it is in both views because an operator who believes they are still
// transmitting is the failure the whole rule exists to remove.
test('both views say when a latch was dropped for the operator', async () => {
	for (const [at, body] of (
		await eachView(inAView({ ...onARung('unconfirmed'), latchDropped: true }))
	).entries()) {
		assert.match(
			asRead(body),
			/latched key was dropped, so you are not transmitting/,
			`${views[at]} does not say the latch went, or what it cost`
		);
	}
});

// **Emission has two independent withdrawal conditions and the console reads both** (ADR-0018,
// ADR-0042). There is one derivation above both views rather than one in each, so a rung
// nobody has a reading of leaves emission withdrawn whichever ladder it is on.
test('the console withdraws emission at the bottom of either ladder', async () => {
	const source = read(join(lib, 'Console.svelte'));

	assert.match(
		source,
		/const mayKey = \$derived\(anAudioPath && aStateChannel\)/,
		'the console decides whether emission stands from something other than the two ladders'
	);
	assert.match(
		source,
		/const aStateChannel = \$derived\(standing !== DISCONNECTED\)/,
		'a session with no signalling channel was left an emission path'
	);
	// **Both ends of the channel, merged pessimistically** — green needs both, red needs one.
	// A console reading only its own clock goes on offering a key control over a fan-out the
	// server has already closed, which is the one failure a single reading cannot see.
	assert.match(
		source,
		/worse\(connection\.state, presence\.connection\)/,
		'the console reads one end of the channel and calls it the answer'
	);
});

// **Frozen and marked stale with a running age** (ADR-0018). Blanking was rejected as its own
// lie — an empty console implies *nothing is happening*, when everything may be — so what
// makes the freeze honest is the number beside it moving.
test('the console marks its own state stale, with a running age', async () => {
	const body = await rendered('Console.svelte', {
		...theConsole,
		connection: { state: 'unconfirmed', since: 7400, aLatchStands: false }
	});

	assert.match(asRead(body), /VoxLoop was last confirmed 7 s ago/);
	// The loops are still there. A console that blanked would be telling an operator nothing
	// is happening at the moment everything may be.
	for (const name of namesOf(inReach)) assert.match(body, new RegExp(name));
});

test('the console says the connection was lost once it is past the threshold', async () => {
	const body = await rendered('Console.svelte', {
		...theConsole,
		connection: { state: 'disconnected', since: 13_000, aLatchStands: false }
	});

	assert.match(asRead(body), /The connection to VoxLoop was lost 13 s ago/);
	for (const name of namesOf(inReach)) assert.match(body, new RegExp(name));
});

// **The half of the failure a console cannot see for itself.** Its answers are being lost
// while VoxLoop's heartbeats still arrive, so its own clock reads `confirmed` — and VoxLoop
// has reached the disconnect threshold and closed the fan-out. Without the document's reading
// the console would go on offering a key control over a route that no longer exists, which is
// ADR-0008's residual arriving as a feature.
test('the server’s reading withdraws emission even where this tab’s clock is happy', async () => {
	const body = await rendered('Console.svelte', {
		presence: { ...theConsole.presence, connection: 'disconnected' },
		connection: { state: 'confirmed', since: 0, aLatchStands: true }
	});

	assert.match(asRead(body), /VoxLoop is not hearing this console, so it will not emit/);
	assert.match(asRead(body), /What is on screen is current/);
	assert.doesNotMatch(body, />\s*Key\s*</, 'the key control outlived a closed fan-out');
});

// The two failures want different sentences: *your console is blind* and *your console is
// unheard* send an operator to look at different things, and only the first of them is a
// console that has stopped being told anything.
test('a console that cannot hear VoxLoop and one VoxLoop cannot hear are not worded alike', async () => {
	const blind = await rendered('Console.svelte', {
		...theConsole,
		connection: { state: 'disconnected', since: 13_000, aLatchStands: false }
	});
	const unheard = await rendered('Console.svelte', {
		presence: { ...theConsole.presence, connection: 'disconnected' },
		connection: { state: 'confirmed', since: 0, aLatchStands: true }
	});

	assert.match(asRead(blind), /The connection to VoxLoop was lost 13 s ago/);
	assert.doesNotMatch(asRead(unheard), /was lost/);
});

// ---- Mute, per-loop volume and the cog (#44) ----------------------------------------------

/** One card of the board, from its opening tag to its close, found by the loop's name. */
const theCard = (board, name) => {
	const opened = board.lastIndexOf('<li', board.indexOf(`>${name}<`));

	return board.slice(opened, board.indexOf('</li>', opened));
};

/** One row of the ledger, found the same way. */
const theRow = (ledger, name) => {
	const opened = ledger.lastIndexOf('<tr', ledger.indexOf(`>${name}<`));

	return ledger.slice(opened, ledger.indexOf('</tr>', opened));
};

// **A state that renders in only one view is a bug** (v1 §8), and mute is one. It is a word
// on the card and a sentence in the row — and the sentence says the thing a word cannot: the
// loop is still monitored, and nobody else is affected.
test('both views say a muted loop is muted, in words', async () => {
	const [board, ledger] = await eachView(carrying);

	assert.match(theCard(board, 'THERMAL'), />\s*Muted\s*</, 'the card does not say it is muted');
	assert.match(theCard(board, 'THERMAL'), />Monitoring</, 'the card lost the subscription');
	assert.match(
		theRow(ledger, 'THERMAL'),
		/You have muted this loop\. It is still monitored, and nobody else is affected\./
	);
	assert.doesNotMatch(theCard(board, 'GNC'), />\s*Muted\s*</);
	assert.doesNotMatch(theRow(ledger, 'GNC'), /muted/);
});

// **A mute presupposes a subscription** (ADR-0049), so a loop nobody is monitoring offers no
// mute: there is nothing to silence on it, and a control that did nothing would be the console
// misrepresenting what a press does.
test('neither view offers a mute on a loop it is not monitoring', async () => {
	const [board, ledger] = await eachView(carrying);

	for (const [at, found] of [theCard(board, 'FLIGHT'), theRow(ledger, 'FLIGHT')].entries()) {
		assert.doesNotMatch(found, />\s*(Mute|Unmute)\s*</, `${views[at]} offers a mute on FLIGHT`);
	}
	assert.match(theCard(board, 'GNC'), />\s*Mute\s*</);
	assert.match(theRow(ledger, 'GNC'), />\s*Mute\s*</);
	assert.match(theCard(board, 'THERMAL'), />\s*Unmute\s*</);
	assert.match(theRow(ledger, 'THERMAL'), />\s*Unmute\s*</);
});

// **Per-loop volume is the one attenuation in VoxLoop that nothing warns anybody about** (v1
// §4), so the operator who turned a loop down is at least shown it, in both views. A loop at
// unity says nothing on the card, because a card cannot carry a word that is true of almost
// every loop on it.
test('both views say a loop is turned down, and the card says nothing of one at unity', async () => {
	const [board, ledger] = await eachView(carrying);

	assert.match(theCard(board, 'GNC'), />\s*40%\s*</);
	assert.match(theRow(ledger, 'GNC'), /Plays at 40% of full volume\./);
	assert.doesNotMatch(theCard(board, 'THERMAL'), /%/);
	assert.match(theRow(ledger, 'THERMAL'), /Plays at full volume\./);
});

// **Behind a cog on the card and the row** (v1 §8, ADR-0034), and **not nudgeable from the
// main surface**: the cog is the only way to the volume, on every loop in both views, and
// neither view holds a control that sets one.
test('both views put a cog on every loop and hold no volume control of their own', async () => {
	for (const [at, body] of (await eachView(carrying)).entries()) {
		for (const name of namesOf(inReach)) {
			assert.match(
				body,
				new RegExp(`<button[^>]*aria-label="Volume for ${name}"`),
				`${views[at]} has no cog on ${name}`
			);
		}
		assert.doesNotMatch(body, /<input/, `${views[at]} holds a control on the main surface`);
		assert.doesNotMatch(read(join(lib, views[at])), /type="range"/);
	}
});

// **Arm, mute and cog must not propagate the card body's click** (v1 §8). The body is one
// `<button>` and a button holds no control, so the mute and the cog are its siblings — and
// neither of them is inside the thing a click on the card is read as.
test('the mute and the cog sit beside the card body, never inside it', async () => {
	const board = await rendered('Board.svelte', carrying);
	const body = theCard(board, 'GNC').match(/<button class="body[^"]*"[^>]*>[\s\S]*?<\/button>/)[0];

	assert.doesNotMatch(body, /Mute|Volume for/, 'a control is inside the card body');
});

// **Which of mute and unmute a press is comes from the document**, decided once above both
// views, the way the toggle and the arm are — so the two views cannot come to disagree about
// what a press on the same loop means.
test('neither view decides whether a press mutes or unmutes', async () => {
	for (const view of views) {
		assert.doesNotMatch(read(join(lib, view)), /onUnmute|onSetVolume/, `${view} picks the act`);
	}

	const source = read(join(lib, 'Console.svelte'));
	assert.equal(
		source.match(/onMute=\{muting\}/g)?.length,
		2,
		'the two views are not handed one mute'
	);
	assert.equal(
		source.match(/onCog=\{openTheVolume\}/g)?.length,
		2,
		'the two views are not handed one cog'
	);
	assert.match(source, /\.muted\b/, 'the mute does not read the document');
});

// **A modal scoped to that loop and holding only volume in v1** (v1 §8). One control, for one
// loop, at the level the document says — and the loop's name on it, so there is no doubt which
// loop a change lands on.
test('the volume modal holds one control, for one loop, at the level the document says', async () => {
	const body = await rendered('LoopVolume.svelte', { loop: inReach[2] });

	assert.equal(
		body.match(/<(input|select|textarea)/g)?.length,
		1,
		'the modal holds more than volume'
	);
	assert.match(body, /<input[^>]*type="range"/);
	assert.match(body, /<input[^>]*value="40"/);
	assert.match(body, /GNC/);
	assert.match(asRead(body), /40% of full volume/);
	assert.match(asRead(body), /Only you hear this/);
});

// The modal is rendered above both views rather than inside either, so it is one modal and it
// reads the loop out of the document it is given — which is what keeps it from showing a level
// the server has not confirmed (ADR-0016).
test('the console opens one volume modal, above both views, off the document', async () => {
	const source = read(join(lib, 'Console.svelte'));

	assert.equal(source.match(/<LoopVolume\b/g)?.length, 1);
	for (const view of views) {
		assert.doesNotMatch(read(join(lib, view)), /LoopVolume/, `${view} holds a modal of its own`);
	}
	assert.doesNotMatch(read(join(lib, 'LoopVolume.svelte')), /\$state\(/, 'the modal keeps a level');
});

// ---- #45: priority ----------------------------------------------------------------------------

// `FLIGHT` carries a priority transmission: somebody keyed priority on it. It is armed and not
// monitored here, so it is also the loop that proves the mark reaches a console not hearing it.
const withPriorityOn = (id, loops = inReach) =>
	loops.map((reachable) =>
		reachable.id === id ? { ...reachable, talking: true, priority: true } : reachable
	);

// **The talking indicator has exactly one variant, and it is not attribution** (v1 §8). One
// component in both views, and there is still nothing in it that could name anybody: the prop
// says what kind of transmission is on the loop and nothing about whose.
test('the priority mark is the talking indicator’s one variant, the same in both views', async () => {
	const mark = await rendered('Talking.svelte', { priority: true });
	const plain = await rendered('Talking.svelte');

	assert.notEqual(mark, plain, 'a priority transmission reads like any other');
	// In a word, because colour and motion are never what carries a state.
	assert.match(mark, /Priority/);
	assert.doesNotMatch(plain, /Priority/);

	for (const [at, body] of (await eachView(inAView({}, withPriorityOn('l-1')))).entries()) {
		assert.equal(body.split(mark).length - 1, 1, `${views[at]} does not mark the priority loop`);
		assert.ok(
			!body.includes(plain),
			`${views[at]} marks a priority loop with the plain indicator as well`
		);
	}
});

// **Marked wherever it lands** (ADR-0059): on a loop this console is not hearing, on one it has
// at full volume where the gain changed nothing, and on one it has muted — where there is no
// audio at all and the mark is the whole of what arrives.
test('the priority mark shows on a muted loop, an unheard one and one at full volume', async () => {
	const mark = await rendered('Talking.svelte', { priority: true });

	for (const id of ['l-3', 'l-1', 'l-2']) {
		for (const [at, body] of (await eachView(inAView({}, withPriorityOn(id)))).entries()) {
			assert.ok(body.includes(mark), `${views[at]} hid the priority mark on ${id}`);
		}
	}
});

// It names nobody (ADR-0033). The component takes a flag, and the only prop it has is that one.
test('the priority mark names nobody', async () => {
	const source = read(join(lib, 'Talking.svelte'));
	const props = source.match(/let \{([^}]*)\} = \$props\(\)/)?.[1] ?? '';

	assert.deepEqual(
		props
			.split(',')
			.map((prop) => prop.split('=')[0].trim())
			.filter(Boolean),
		['priority'],
		'the talking indicator takes something that could name a talker'
	);
});

// **The third binding has an on-screen control too**, because a source is a way to reach a
// level and the bar is a source (ADR-0021). Both views carry it, because they carry one bar.
test('both views offer the priority control beside the other two', async () => {
	for (const [at, body] of (await eachView(carrying)).entries()) {
		assert.match(body, />\s*Priority\s*</, `${views[at]} offers no priority control`);
	}
});

// **The lamp is the server's answer**, priority included (ADR-0008). An elevated transmission
// shows as elevated on the key control's lamp with no new surface (ADR-0046), and it is lit by
// the document's `priority` and nothing held here.
test('the lamp says a transmission is at priority when the document does', async () => {
	const at = (keyed, priority) => rendered('TransmitBar.svelte', { ...theBar, keyed, priority });

	assert.match(await at(true, true), />\s*Keyed at priority\s*</);
	assert.match(await at(true, false), />\s*Keyed\s*</);
	assert.match(await at(false, false), />\s*Not keyed\s*</);

	for (const [where, body] of (
		await eachView(inAView({ keyed: true, priority: true }))
	).entries()) {
		assert.match(body, /Keyed at priority/, `${views[where]} does not say it is elevated`);
	}
});

// **Priority never latches** (v1 §4), so the control has nothing on the way down that could
// hold it — it is a key you hold, drawn like the key control rather than like the latch.
test('the priority control is held, and has no latched state to show', async () => {
	const source = read(join(lib, 'TransmitBar.svelte'));

	assert.match(source, /onpointerup=\{onPriorityUp\}/);
	assert.match(source, /onpointerleave=\{onPriorityUp\}/);
	assert.doesNotMatch(source, /Unpriority|unprioritise/i);
});

// ---- #46: the loop beacon and loop health ---------------------------------------------------

// **A quiet loop and an unreachable loop sound identical, so they must never look identical**
// (v1 §6). A loop whose beacon is not arriving says so in both views, in words — a word on the
// card and a sentence in the row — and never in colour alone.
test('both views say a monitored loop is not being received, in words', async () => {
	const [board, ledger] = await eachView(carrying);

	assert.match(theCard(board, 'GNC'), />\s*Not receiving\s*</);
	assert.match(
		theRow(ledger, 'GNC'),
		/Its beacon is not arriving, so you may not hear this loop even when somebody talks on it\./
	);
});

// **The card carries no word for the ordinary case**, as it carries none for a loop at unity: a
// word true of nearly every monitored card is one nobody reads, and the card is read at a
// glance. The ledger is the reading view and says it on every monitored row — including a
// muted one, because a mute is not an unsubscribe and the beacon keeps arriving.
test('the ledger says a loop is reaching this console, and the card says nothing of it', async () => {
	const [board, ledger] = await eachView(carrying);

	assert.match(theRow(ledger, 'THERMAL'), /Its beacon is arriving, so this loop reaches you\./);
	assert.doesNotMatch(theCard(board, 'THERMAL'), /receiving|Checking/i);
});

// A loop just taken up has not proved it reaches anybody yet. That is a measurement not yet
// taken rather than a failure, and it is worded as one in both views.
test('both views say a loop just taken up is being checked', async () => {
	const checking = inReach.map((reachable) =>
		reachable.name === 'GNC' ? { ...reachable, health: 'checking' } : reachable
	);
	const [board, ledger] = await eachView({ loops: checking, bar: theBar });

	assert.match(theCard(board, 'GNC'), />\s*Checking\s*</);
	assert.match(theRow(ledger, 'GNC'), /Checking that this loop reaches you\./);
});

// A loop nobody here monitors has no beacon being counted, so neither view says anything about
// whether it is received — a word there would be the console inventing a measurement.
test('neither view says anything of health on a loop it is not monitoring', async () => {
	const [board, ledger] = await eachView(carrying);

	for (const [at, found] of [theCard(board, 'FLIGHT'), theRow(ledger, 'FLIGHT')].entries()) {
		assert.doesNotMatch(found, /receiving|reaches you|Checking|beacon/i, `${views[at]} on FLIGHT`);
	}
});

// **Colour is never the only thing carrying a state** (`styling.md`), and loss is marked in the
// warning colour through a class that names what it is rather than how it looks.
test('a loop not being received is marked by what it is, in both views', async () => {
	const [board, ledger] = await eachView(carrying);

	assert.match(theCard(board, 'GNC'), /class="[^"]*\bunreceived\b/);
	assert.match(theRow(ledger, 'GNC'), /class="[^"]*\bunreceived\b/);
	assert.doesNotMatch(theCard(board, 'THERMAL'), /unreceived/);
});

// **The beacon proves audio reached the browser, not the operator's ears** (ADR-0017), so the
// console asks them: a check tone at assume, offered before anything else is relied on.
test('a console just assumed asks the operator to confirm they can hear a check tone', async () => {
	const check = await rendered('OutputCheck.svelte');

	assert.match(check, /<button[^>]*>\s*Play the check tone\s*<\/button>/);
	assert.match(check, /Check that you can hear VoxLoop/);
});

// **Any change to the output is surfaced loudly** (ADR-0017): a default swapped under the
// operator is named, an output unplugged is said, and both come with the way to check again.
test('an output that moved is said aloud, with the way to check again', async () => {
	const swapped = await rendered('OutputCheck.svelte', {
		moved: { swapped: 'Default - Speakers' }
	});
	const unplugged = await rendered('OutputCheck.svelte', { moved: { removed: true } });

	for (const said of [swapped, unplugged]) {
		assert.match(said, /role="alert"/);
		assert.match(said, /Play the check tone/);
	}
	assert.match(swapped, /Default - Speakers/);
	assert.match(unplugged, /An audio output was unplugged/);
});

// One check, above both views rather than inside either, because it is about this desk's
// hardware and not about any loop.
test('the console holds one output check, above both views', async () => {
	const source = read(join(lib, 'Console.svelte'));

	assert.equal(source.match(/<OutputCheck\b/g)?.length, 1);
	for (const view of views) {
		assert.doesNotMatch(read(join(lib, view)), /OutputCheck/, `${view} holds a check of its own`);
	}
});

// ---- Off console (#47) ----------------------------------------------------------------

// The one claim in the product, and the one the console is forbidden from drawing like a
// fact (ADR-0016). Fourteen minutes is the age ADR-0016 itself uses as the example of what
// an honest rendering says.
const away = { last_active_seconds: 14 * 60 };

const theConsoleOffConsole = {
	...theConsole,
	presence: { ...theConsole.presence, off_console: away }
};

test('the console offers a way to say you are off console, and a way back', async () => {
	const on = await rendered('Console.svelte', theConsole);
	const off = await rendered('Console.svelte', theConsoleOffConsole);

	assert.match(on, />Off console</, 'a console with no way to say you are stepping away');
	assert.match(off, />I am back on console</, 'an assertion with no way out of it');
});

// **The claim and the age of its evidence are one value** (v1 §6), so the age moves with the
// document and there is no rendering in which the claim appears without it.
test('an assertion is never shown without how long ago its claimant was last active', async () => {
	const said = await Promise.all(
		[9, 14 * 60, 3 * 3600 + 20 * 60].map((last_active_seconds) =>
			rendered('OffConsole.svelte', { asserted: { last_active_seconds } })
		)
	);

	assert.match(said[0], /Last active 9 s ago/);
	assert.match(said[1], /Last active 14 min ago/);
	assert.match(said[2], /Last active 3 h 20 min ago/);
	// **A stale assertion is still shown, with its age** — nothing rounds three hours away,
	// hides it, or resolves the ambiguity on the reader's behalf.
	assert.match(said[2], /You said you are off console/);
});

// **The console may not render the asserted and the observed alike** (ADR-0016). The words
// carry it first — *you said*, which nothing the server observed would ever be written as —
// and the outline carries it second, because colour is never the only thing carrying a state.
test('an assertion is not drawn like anything the server observed', async () => {
	const body = await rendered('OffConsole.svelte', { asserted: away });
	const source = read(join(lib, 'OffConsole.svelte'));

	assert.match(body, /You said/, 'the claim is not marked as a claim in words');
	assert.match(asRead(body), /not something it has seen/);
	assert.match(source, /border:[^;]*dashed/, 'the claim wears the outline of an observed state');
	// The stale and lost marks are what an observed state that wants looking at reads like,
	// and this is not one of them: an operator's claim about themselves is not a fault.
	assert.doesNotMatch(source, /--warning/);
});

// It is about the person in the chair rather than about any loop, so it sits above both views
// like the stale mark and the output check — on screen whichever view is showing, and worded
// once because there is one of it.
test('the assertion is above both views rather than inside either', async () => {
	const body = await rendered('Console.svelte', theConsoleOffConsole);

	assert.ok(
		body.includes(await rendered('OffConsole.svelte', { asserted: away })),
		'the console does not carry the assertion'
	);
	for (const view of views) {
		assert.doesNotMatch(
			read(join(lib, view)),
			/OffConsole/,
			`${view} draws the assertion itself — there is one of it, above both views`
		);
	}
});

// ---- Staffing state (#48) -----------------------------------------------------------

// **A word on the board and a sentence in the ledger** (v1 §8). The division of labour the
// two views exist for, applied to the state that most needs it: a card cannot hold
// `away — 2 not subscribed, 1 muted`, and it does not have to.
test('staffing state is a word on the board and a sentence in the ledger', async () => {
	const [board, ledger] = await eachView(carrying);

	assert.match(board, /Staffed/, 'the board does not carry the word');
	assert.match(board, /Away/, 'the board does not carry the word');
	assert.doesNotMatch(
		board,
		/not subscribed/,
		'the board carries the reason a card has no room for'
	);
	assert.match(ledger, /Away — 2 not subscribed, 1 muted\./);
	assert.match(ledger, /an occupant of a role that staffs this loop is hearing it/);
});

// **Where occupants are away for different reasons the ledger counts them**, and it ranks
// nothing: no reason wins by precedence across people, because a mute is one click from
// hearing and so is a subscription (ADR-0065).
test('the ledger counts the reasons and puts none of them first', async () => {
	const [, ledger] = await eachView(carrying);

	assert.match(ledger, /2 not subscribed, 1 muted/);
	assert.doesNotMatch(ledger, /Away — muted it/, 'one reason was picked as the winner');
});

// **The absence of a staffing state is not a fourth state.** A loop with no staffing roles
// renders blank where the word goes: `vacant` there would say nobody is behind a loop two
// people may be talking on right now (ADR-0056).
test('a loop with no staffing roles renders blank in both views', async () => {
	const nothingStaffs = [inReach[2]];

	for (const [at, body] of (await eachView(inAView({}, nothingStaffs))).entries()) {
		assert.doesNotMatch(body, /Vacant|vacant/, `${views[at]} read no staffing roles as vacant`);
		assert.doesNotMatch(body, /Staffed|staffed/, `${views[at]} said something about staffing`);
		assert.doesNotMatch(body, /Away —/, `${views[at]} said something about staffing`);
		assert.match(body, /GNC/, `${views[at]} lost the loop`);
	}
});

// Losing the last staffing role is that blank arriving mid-session, and the console does not
// treat it as an error — it is the same render as a loop that never had one (ADR-0056).
test('a loop that loses its last staffing role renders as one that never had one', async () => {
	const [held] = inReach;
	const lost = { ...held, staffing: null, staffs: false };

	for (const [at, view] of views.entries()) {
		assert.equal(
			await rendered(view, inAView({}, [lost])),
			await rendered(view, inAView({}, [{ ...lost, staffing: undefined }])),
			`${views[at]} renders a loop that lost its staffing state differently`
		);
	}
});

// **The loops this role staffs are marked in both views, in two states** (v1 §8), and the
// mark is on the loop whether or not anything is wrong: showing only the second state would
// make it an alarm rather than a fact about the operator's own console.
test('the staffing mark is in both views, in both of its states', async () => {
	for (const [at, body] of (await eachView(carrying)).entries()) {
		assert.match(body, /You staff this/, `${views[at]} does not mark the loops this role staffs`);
		// One word for the condition in both views, and it is this console's own word for
		// the subscription rather than the `away` reason's, which is about somebody else's.
		assert.match(
			body,
			/You staff this(,| loop and you are) not monitoring it/,
			`${views[at]} does not tell the two states of the mark apart`
		);
	}
});

test('a loop this role does not staff carries no mark in either view', async () => {
	const unmarked = [inReach[2]];

	for (const [at, body] of (await eachView(inAView({}, unmarked))).entries()) {
		assert.doesNotMatch(body, /You staff/, `${views[at]} marked a loop this role does not staff`);
	}
});

// The mark is the console's fact and the staffing state is the loop's, so a loop this role
// staffs and is hearing carries the mark with nothing wrong anywhere.
test('the mark stands on a loop that is staffed and subscribed', async () => {
	const fine = [inReach[0]];

	for (const [at, body] of (await eachView(inAView({}, fine))).entries()) {
		assert.match(body, /You staff this/, `${views[at]} dropped the mark when nothing was wrong`);
	}
});

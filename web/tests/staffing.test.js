// Staffing state as the console says it: the word, the sentence, and the lobby that reads
// the sentence rather than the word.
//
// The two views are checked together in `board-and-ledger.test.js`, because a state that
// renders in only one of them is a bug. What is here is the wording itself — the collapse,
// the counts, and the absence — and the third surface that says it, which is the lobby.

import assert from 'node:assert/strict';
import test from 'node:test';
import { join } from 'node:path';
import { read, src } from './console.js';
import { rendered } from './render.js';
import { theSentence, theWord } from '../src/lib/staffing.js';

const lib = join(src, 'lib');

const away = (...reasons) => ({
	state: 'away',
	away: reasons.map(([reason, occupants]) => ({ reason, occupants }))
});

// **A loop with no staffing roles has no staffing state** (ADR-0056), and it renders blank
// rather than as a fourth word: `n/a` and `unstaffed` both read as states, and this is the
// absence of one.
test('a loop with no staffing roles says nothing at all', () => {
	for (const nothing of [null, undefined]) {
		assert.equal(theWord(nothing), '');
		assert.equal(theSentence(nothing), '');
	}
});

test('each state is a word of its own', () => {
	assert.equal(theWord({ state: 'staffed' }), 'Staffed');
	assert.equal(theWord({ state: 'away' }), 'Away');
	assert.equal(theWord({ state: 'vacant' }), 'Vacant');
});

// The server is the only thing entitled to judge a loop staffed, so a state this build has
// no name for is said as the word the document used rather than dropped.
test('a state this console has no name for is said as the word it arrived as', () => {
	assert.equal(theWord({ state: 'something-later' }), 'something-later');
	assert.match(theSentence({ state: 'something-later' }), /something-later/);
});

// **It collapses to the plain sentence when the occupants agree** (v1 §8): `1 muted` is a
// count nobody needed, and the five reasons each read as a sentence rather than as a label.
test('one reason is the plain sentence, whoever many occupants hold it', () => {
	assert.equal(theSentence(away(['muted', 1])), 'Away — muted it.');
	assert.equal(theSentence(away(['off-console', 1])), 'Away — off console.');
	assert.equal(theSentence(away(['not-subscribed', 3])), 'Away — not subscribed to it.');
	assert.equal(theSentence(away(['not-receiving', 1])), 'Away — not receiving it.');
	assert.equal(theSentence(away(['unreachable', 2])), 'Away — unreachable.');
});

// **Where occupants are away for different reasons the reason is counted** — and the counts
// rank nothing, because no ordering across people is defensible (ADR-0065).
test('differing reasons are counted, in the order they arrive', () => {
	assert.equal(
		theSentence(away(['unreachable', 1], ['not-subscribed', 2], ['muted', 1])),
		'Away — 1 unreachable, 2 not subscribed, 1 muted.'
	);
});

test('staffed and vacant each say what they mean', () => {
	assert.match(theSentence({ state: 'staffed' }), /is hearing it/);
	assert.match(theSentence({ state: 'vacant' }), /nobody occupies a role that staffs this loop/);
});

// One implementation, three readers. The ledger and the lobby saying the same thing about
// the same field is a property of there being one function, rather than of two of them
// agreeing today.
test('neither the ledger nor the lobby words staffing state itself', () => {
	for (const view of ['Ledger.svelte', 'Lobby.svelte']) {
		assert.match(
			read(join(lib, view)),
			/import \{ theSentence \} from '\.\/staffing\.js'/,
			`${view} does not read the sentence from the one place that holds it`
		);
	}
});

const aLobby = {
	roles: [
		{
			id: 'r-1',
			name: 'Flight Director',
			max_occupants: 1,
			occupants: ['gene'],
			staffs: [
				{ id: 'l-1', name: 'FLIGHT', staffing: away(['not-subscribed', 2]) },
				{ id: 'l-2', name: 'GNC', staffing: { state: 'staffed', away: [] } }
			]
		},
		{ id: 'r-2', name: 'Observer', max_occupants: null, occupants: [], staffs: [] }
	]
};

// **The lobby carries the reason in full, ledger-style** (v1 §2). It is read once and
// deliberately, by somebody about to be in a position to fix what it says, and
// `away — not subscribed to it` tells them the seat's loops need setting up before they
// take it.
test('the lobby says which loops a seat answers for and how they stand', async () => {
	const body = await rendered('Lobby.svelte', { lobby: aLobby });

	assert.match(body, /FLIGHT/);
	assert.match(body, /Away — not subscribed to it\./);
	assert.match(body, /GNC/);
	assert.match(body, /is hearing it/);
});

// A seat that answers for no loop says so: *this role staffs nothing* is an answer, and an
// empty cell is not.
test('a role that staffs nothing says so rather than leaving a blank', async () => {
	const body = await rendered('Lobby.svelte', { lobby: aLobby });

	assert.match(body, /None/);
});

// The lobby holds no authority and shows no console, so what it lists is the loops the seat
// answers for and not the loops it can reach.
test('the lobby lists no loops beyond the ones each role staffs', async () => {
	const body = await rendered('Lobby.svelte', {
		lobby: { roles: [{ ...aLobby.roles[0], staffs: [] }] }
	});

	assert.doesNotMatch(body, /FLIGHT|GNC/);
});

// **No banner announces an unsubscribed staffed loop** (v1 §8). It was designed and dropped:
// it would be the first server-generated advisory in the product, needing a slot, a lifetime
// and a dismissal rule, to say what the card already shows. The structural guarantee is that
// the console above the two views never reads either staffing field — there is nothing there
// for a banner to be built from, and the mark is on the card where the fix is.
test('the console raises nothing about staffing above the two views', () => {
	assert.doesNotMatch(
		read(join(lib, 'Console.svelte')),
		/\.(staffing|staffs)\b/,
		'the console reads a staffing field above the views — the mark belongs on the card'
	);
});

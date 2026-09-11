// The Audio module's one decision: how loud to play a talker.
//
// **Loudest-wins** (v1 §5, ADR-0007). The downlink is one stream per audible talker, so a
// talker reaching this operator on several loops arrives once — and plays at the loudest
// volume among the loops it is heard on. Volume is an attenuation control: a transmission also
// going to a loop the operator kept up is one they have already said they want to hear, and
// quietest-wins would let a suppressed loop silence it.
//
// It is asked of the function rather than of a playing stream, because the rule is the part
// of the Audio module that is VoxLoop's; the rest of it is the browser doing what it is told.

import assert from 'node:assert/strict';
import test from 'node:test';

import { loudest, theGain } from '../src/lib/audio.js';

// Three loops as the presence document carries them: one at unity, one turned down, one
// turned all the way down.
const loops = [
	{ id: 'l-flight', volume: 100, muted: false },
	{ id: 'l-sim', volume: 20, muted: false },
	{ id: 'l-thermal', volume: 0, muted: false }
];

test('a talker heard on one loop plays at that loop’s volume', () => {
	assert.equal(loudest(['l-sim'], loops), 0.2);
	assert.equal(loudest(['l-flight'], loops), 1);
});

test('a talker heard on several loops plays at the loudest of them', () => {
	assert.equal(loudest(['l-sim', 'l-flight'], loops), 1);
	assert.equal(loudest(['l-thermal', 'l-sim'], loops), 0.2);
});

// **A muted loop is not an applicable one** (v1 §5). The server stops carrying a talker on a
// loop the operator muted, and until that lands the document already says the loop is muted —
// so the mute is heard as soon as it is shown, and never lets a muted loop's volume win.
test('a muted loop is not one a talker is heard on', () => {
	const muted = loops.map((held) => (held.id === 'l-flight' ? { ...held, muted: true } : held));

	assert.equal(loudest(['l-flight', 'l-sim'], muted), 0.2);
	assert.equal(loudest(['l-flight'], muted), 0);
});

// A loop the document does not name yet is one a carriage arrived on before the document that
// says what it is set to. It is played at unity rather than guessed down, because the failure
// this rule is written against is a transmission the operator wanted going unheard.
test('a loop the document has not described yet plays at unity', () => {
	assert.equal(loudest(['l-new'], loops), 1);
	assert.equal(loudest([], loops), 1);
});

// ---- #45: priority ----------------------------------------------------------------------------

// **A priority transmission plays at full gain whatever the loop is set to** (v1 §4, ADR-0045).
// It is read off the same document as the volume, as the loop's priority mark, because the
// mark and the gain are one fact arriving (ADR-0059).
const marked = (id, on = loops) =>
	on.map((held) => (held.id === id ? { ...held, priority: true } : held));

test('a talker on a loop carrying a priority transmission plays at full gain', () => {
	assert.equal(theGain(['l-sim'], marked('l-sim')), 1);
	assert.equal(theGain(['l-thermal'], marked('l-thermal')), 1, 'a loop turned right down hid it');
});

test('with no priority anywhere, the gain is loudest-wins', () => {
	assert.equal(theGain(['l-sim'], loops), 0.2);
	assert.equal(theGain(['l-thermal', 'l-sim'], loops), 0.2);
	assert.equal(theGain(['l-new'], loops), 1);
});

// **Priority bypasses loudest-wins rather than competing with it** (ADR-0045): there is no
// rule run over the other loops, and one marked loop among several is enough.
test('priority on one of several loops a talker is heard on wins outright', () => {
	assert.equal(theGain(['l-thermal', 'l-sim'], marked('l-sim')), 1);
});

// **Mute stays sovereign** (ADR-0045). A muted loop is not one a talker is heard on, so a
// priority mark on it raises nothing — the mark still shows, and the audio does not arrive.
test('priority does not defeat a mute', () => {
	const muted = marked('l-sim').map((held) =>
		held.id === 'l-sim' ? { ...held, muted: true } : held
	);

	assert.equal(theGain(['l-sim'], muted), 0);
	assert.equal(theGain(['l-sim', 'l-thermal'], muted), 0);
});

// **Nothing in VoxLoop ever ducks** (v1 §4). A priority transmission somewhere else changes
// nothing about a talker who is not on a marked loop: they play at exactly what they would have.
test('priority lowers no other talker', () => {
	const elsewhere = marked('l-flight');

	assert.equal(theGain(['l-sim'], elsewhere), 0.2);
	assert.equal(theGain(['l-thermal'], elsewhere), 0);
});

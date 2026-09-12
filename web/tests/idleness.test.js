// **VoxLoop never guesses whether a human is in the chair** (ADR-0016), and this is the file
// that keeps it true rather than remembered.
//
// Idle-based auto-away is rejected outright: an operator watching telemetry intently is idle
// at the keyboard and very much on console, so a console that demoted them would be inventing
// a state in the opposite direction from the one it was meant to fix. Off console is asserted
// by a person and cleared by a deliberate act, and every deliberate act is a message the
// server already receives — so the client has nothing to measure and no reason to watch.
//
// A sweep of every source file rather than of the one component, because the rule is about
// the product and not about a surface: the way this gets broken is somebody adding an idle
// timer to the frame, or a `mousemove` listener to a view, long after the component that owns
// the claim was written and reviewed.
//
// `blur` is excluded, and named here so the exclusion is a decision rather than a gap: the
// keyboard source releases a key held when the window goes, because no key-up is coming for
// it (`src/lib/input/sources/keyboard.js`). That is Input answering *is this key down*, which
// is not *is somebody there*, and it reaches nothing in this file's subject.

import assert from 'node:assert/strict';
import test from 'node:test';
import { named, read, under } from './console.js';

// Both spellings, because the console writes listeners two ways: `addEventListener('scroll')`
// in a module, and `onscroll={…}` as a Svelte attribute — with `on:scroll={…}`, Svelte's
// older form, caught as well so that a component written from a stale example does not slip
// through. A check for one of them would pass a component doing the other, which is the half
// of the rule that would go unnoticed.
const watching = [
	/'(mousemove|mouseover|mouseenter|pointermove|scroll|visibilitychange|focus)'/,
	/\bon:?(mousemove|mouseover|mouseenter|pointermove|scroll|visibilitychange|focus)\s*[={:]/
];

test('nothing in the console watches for activity', () => {
	for (const path of under(/\.(svelte|js)$/)) {
		for (const listening of watching) {
			assert.doesNotMatch(
				read(path),
				listening,
				`${named(path)} watches for activity — off console is asserted and never inferred`
			);
		}
	}
});

// The claim is made by a hand on a control. A clock anywhere near it is the rejected design
// arriving through the other door, whichever direction it moved the flag in.
test('nothing decides off console on a clock', () => {
	const source = read(under(/^OffConsole\.svelte$/)[0]);

	assert.doesNotMatch(source, /setTimeout|setInterval|Date\.now/);
});

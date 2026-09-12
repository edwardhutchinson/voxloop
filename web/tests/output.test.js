// The client watching its own output path (v1 §6, ADR-0017).
//
// **The beacon proves audio reached the browser and says nothing about whether it reached the
// operator's ears.** The likeliest real *"I can't hear Flight"* is a headset unplugged, or the
// operating system quietly repointing its default output — both invisible to the server and
// invisible to the beacon. So the console notices them itself, and these are the two rules it
// notices them by: which device is the default, and whether that has moved.

import assert from 'node:assert/strict';
import test from 'node:test';

import { outputsAsTheyStand, watchTheOutput, whatMoved } from '../src/lib/output.js';

// What `enumerateDevices` answers, cut to what is read from it. The `default` entry is the
// browser's name for whatever the operating system is sending sound to, and its label says
// which device that is.
const headset = { kind: 'audiooutput', deviceId: 'hs', label: 'Headset', groupId: 'g-hs' };
const speakers = { kind: 'audiooutput', deviceId: 'sp', label: 'Speakers', groupId: 'g-sp' };
const theDefault = (device) => ({
	kind: 'audiooutput',
	deviceId: 'default',
	label: `Default - ${device.label}`,
	groupId: device.groupId
});
const microphone = { kind: 'audioinput', deviceId: 'mic', label: 'Microphone', groupId: 'g-hs' };

test('what is read of the outputs is the default’s label and group, and every output there is', () => {
	assert.deepEqual(outputsAsTheyStand([microphone, theDefault(headset), headset, speakers]), {
		theDefault: { label: 'Default - Headset', group: 'g-hs' },
		outputs: ['hs', 'sp']
	});
});

test('nothing moved is nothing to say', () => {
	const before = outputsAsTheyStand([theDefault(headset), headset, speakers]);
	const after = outputsAsTheyStand([theDefault(headset), headset, speakers]);

	assert.equal(whatMoved(before, after), null);
});

// **The default swapped** is detected by comparing the label and the group of the `default`
// entry across change events (ADR-0017), because its id is `default` whatever it points at.
test('the default output moving to another device is a swap, and names where it went', () => {
	const before = outputsAsTheyStand([theDefault(headset), headset, speakers]);
	const after = outputsAsTheyStand([theDefault(speakers), headset, speakers]);

	assert.deepEqual(whatMoved(before, after), { swapped: 'Default - Speakers' });
});

test('an output that went away is one somebody may have been listening on', () => {
	const before = outputsAsTheyStand([theDefault(headset), headset, speakers]);
	const after = outputsAsTheyStand([theDefault(speakers), speakers]);

	assert.deepEqual(whatMoved(before, after), {
		swapped: 'Default - Speakers',
		removed: true
	});
});

test('an output arriving that changes nothing about the default is nothing to say', () => {
	const before = outputsAsTheyStand([theDefault(headset), headset]);
	const after = outputsAsTheyStand([theDefault(headset), headset, speakers]);

	assert.equal(whatMoved(before, after), null);
});

// A browser that has not been granted a device yet hands back entries with no labels. Nothing
// can be compared then, and a swap reported off two blank labels would be an alarm about
// nothing — so nothing is said until there is something to read.
test('outputs with no labels to read are not compared', () => {
	const blank = (device) => ({ ...device, label: '' });
	const before = outputsAsTheyStand([blank(theDefault(headset)), blank(headset)]);
	const after = outputsAsTheyStand([blank(theDefault(speakers)), blank(speakers)]);

	assert.equal(whatMoved(before, after), null);
});

// **A confirmed check tone at assume, and `devicechange` monitoring thereafter** (ADR-0017),
// in that order: what the operator heard the tone on is the path the watch is measured from,
// so nothing is compared until they have said they heard it.
test('nothing is reported until the operator has confirmed what they heard the tone on', async () => {
	let listed = [theDefault(headset), headset, speakers];
	const moved = [];
	const devices = {
		enumerateDevices: async () => listed,
		addEventListener: (_event, listener) => (devices.changed = listener),
		removeEventListener: () => {}
	};
	const watch = watchTheOutput({ devices, onMoved: (now) => moved.push(now) });

	listed = [theDefault(speakers), headset, speakers];
	await devices.changed();

	assert.deepEqual(moved, [], 'a change was reported against a baseline nobody had confirmed');

	await watch.settle();
	listed = [theDefault(headset), headset, speakers];
	await devices.changed();

	assert.deepEqual(moved, [{ swapped: 'Default - Headset' }]);
	watch.stop();
});

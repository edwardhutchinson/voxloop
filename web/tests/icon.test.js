// `Icon.svelte` is the console's whole icon mechanism, so what is tested here is the part a
// component author relies on without looking: an icon takes its size and colour from the
// text beside it, and it is invisible to a screen reader unless somebody says otherwise.
//
// Rendering it means compiling it, which `render.js` is: there is no component test runner
// in the console and a whole dependency tree is a lot to buy for one component.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { components, named, read, src } from './console.js';
import { rendered } from './render.js';

const lib = join(src, 'lib');
const source = read(join(lib, 'Icon.svelte'));

const drawn = (props) => rendered('Icon.svelte', props);

test('an icon inherits its size and its colour', async () => {
	assert.match(await drawn({ name: 'arrow-left' }), /stroke="currentColor"/);

	// Not `1rem`: `1em` is the font size of whatever the icon sits beside, so an icon in a
	// heading and an icon in a table row need no token and no variant between them.
	assert.match(source, /width:\s*1em/);
	assert.match(source, /height:\s*1em/);
});

test('an icon is hidden from a screen reader unless it is labelled', async () => {
	const beside = await drawn({ name: 'arrow-left' });
	assert.match(beside, /aria-hidden="true"/);
	assert.doesNotMatch(beside, /role="img"/);

	const alone = await drawn({ name: 'bell', label: 'Hail' });
	assert.match(alone, /role="img"/);
	assert.match(alone, /aria-label="Hail"/);
	assert.doesNotMatch(alone, /aria-hidden/);
});

test('an icon draws the shapes its entry holds', async () => {
	const settings = await drawn({ name: 'settings' });
	assert.match(settings, /<path d="M9.671/);
	assert.match(settings, /<circle cx="12" cy="12" r="3"/);
});

test('the licences travel with the copied path data', () => {
	const icons = read(join(lib, 'icons.js'));

	// Both licences require the notice appears in all copies, and this is a self-hosted
	// product that ships to customers.
	assert.match(icons, /ISC/);
	assert.match(icons, /Lucide Icons and Contributors/);
	assert.match(icons, /MIT/);
	assert.match(icons, /Cole Bemis/);
});

test('every icon is drawn out of shapes Icon.svelte knows how to render', async () => {
	const { icons } = await import(join(lib, 'icons.js'));
	const drawable = ['path', 'circle', 'rect'];

	for (const [name, shapes] of Object.entries(icons)) {
		assert.ok(shapes.length > 0, `${name} has no shapes`);

		for (const [shape, attributes] of shapes) {
			assert.ok(
				drawable.includes(shape),
				`${name} is drawn with <${shape}>, which Icon.svelte drops`
			);
			assert.ok(
				Object.keys(attributes).length > 0,
				`a <${shape}> in ${name} carries no attributes`
			);
		}
	}
});

test('every icon the console asks for is one icons.js holds', async () => {
	const { icons } = await import(join(lib, 'icons.js'));

	// A name written out, which is every call site there is. `Icon.svelte` renders an empty
	// `<svg>` for a name it does not hold, and a hole in a control room console is worth
	// catching here rather than by looking. A computed `name={…}` is out of this check's
	// reach, and is the reason for the note in `docs/agents/styling.md`.
	for (const path of components()) {
		for (const [, name] of read(path).matchAll(/<Icon\s[^>]*name="([^"]+)"/g)) {
			assert.ok(name in icons, `${named(path)} asks for the icon ${name}, which does not exist`);
		}
	}
});

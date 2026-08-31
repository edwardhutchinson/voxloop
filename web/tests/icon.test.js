// `Icon.svelte` is the console's whole icon mechanism, so what is tested here is the part a
// component author relies on without looking: an icon takes its size and colour from the
// text beside it, and it is invisible to a screen reader unless somebody says otherwise.
//
// Rendering it means compiling it, because there is no component test runner in the console
// and one whole dependency tree is a lot to buy for one component.

import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { compile } from 'svelte/compiler';
import { render } from 'svelte/server';
import { components, named, read, src } from './console.js';

const lib = join(src, 'lib');
const source = read(join(lib, 'Icon.svelte'));

let Icon;

before(async () => {
	// `.svelte-kit/` is the generated directory: git, Prettier and ESLint all ignore it.
	const built = join(src, '..', '.svelte-kit', 'tests');
	mkdirSync(built, { recursive: true });

	const { js } = compile(source, { generate: 'server', filename: 'Icon.svelte' });
	// The compiled component is written outside `src/`, so its one relative import has to be
	// pointed back at the file it means.
	writeFileSync(
		join(built, 'Icon.js'),
		js.code.replace("'./icons.js'", JSON.stringify(join(lib, 'icons.js')))
	);

	Icon = (await import(join(built, 'Icon.js'))).default;
});

const drawn = (props) => render(Icon, { props }).body;

test('an icon inherits its size and its colour', () => {
	assert.match(drawn({ name: 'arrow-left' }), /stroke="currentColor"/);

	// Not `1rem`: `1em` is the font size of whatever the icon sits beside, so an icon in a
	// heading and an icon in a table row need no token and no variant between them.
	assert.match(source, /width:\s*1em/);
	assert.match(source, /height:\s*1em/);
});

test('an icon is hidden from a screen reader unless it is labelled', () => {
	const beside = drawn({ name: 'arrow-left' });
	assert.match(beside, /aria-hidden="true"/);
	assert.doesNotMatch(beside, /role="img"/);

	const alone = drawn({ name: 'bell', label: 'Hail' });
	assert.match(alone, /role="img"/);
	assert.match(alone, /aria-label="Hail"/);
	assert.doesNotMatch(alone, /aria-hidden/);
});

test('an icon draws the shapes its entry holds', () => {
	const settings = drawn({ name: 'settings' });
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

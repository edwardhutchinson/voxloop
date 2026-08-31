// The admin console's pages have URLs (#76), and a URL is a promise: it is what somebody
// bookmarks, reloads and pastes into a chat. Two things can quietly break that promise and
// neither shows up as an error at runtime — a link written to a page that does not exist,
// and a page nobody can link to — so both are checked here rather than clicked through.
//
// The third check is the one the issue is really about: moving between these pages must not
// reload the document, because the signalling channel is one socket per tab opened at sign-in
// (ADR-0054) and a full navigation drops it.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync } from 'node:fs';
import { join } from 'node:path';
import { components, named, read, src, under } from './console.js';

const routes = join(src, 'routes');

// A directory holding a `+page.svelte` or a `+page.js` is a page. The second is a page that
// only decides where somebody meant to go — `/admin` and `/admin/roles/{id}` are both a
// record with pages under it rather than a page of their own.
function pages(dir = routes, at = '') {
	const held = readdirSync(dir, { withFileTypes: true });
	const here = held.some((entry) => entry.name === '+page.svelte' || entry.name === '+page.js');

	return [
		...(here ? [at || '/'] : []),
		...held
			.filter((entry) => entry.isDirectory())
			.flatMap((entry) => pages(join(dir, entry.name), `${at}/${entry.name}`))
	];
}

// Written out rather than derived, because this is the list #76 agreed to and a route that
// appears or disappears without the list changing is the thing worth being told about.
const agreed = [
	// The frame's own page. The lobby and the operating console deliberately have no URL:
	// which of the two a person is looking at is what the server says about their session,
	// never a place they navigated to.
	'/',
	'/admin',
	'/admin/grid',
	'/admin/loops',
	'/admin/loops/[id]',
	'/admin/roles',
	'/admin/roles/[id]',
	'/admin/roles/[id]/eligibility',
	'/admin/roles/[id]/reach',
	'/admin/users',
	'/admin/users/[id]'
];

test('the console has exactly the pages it agreed to have', () => {
	assert.deepEqual(pages().sort(), [...agreed].sort());
});

// Every path the console writes is an absolute one it wrote itself; `/api/…` is the server's
// surface and belongs to `server.js`. A parameter reaches a path as an interpolation — Svelte's
// `{…}` in markup, a template literal's `${…}` in a module — and stands for the segment it
// fills.
const written = /(['"`])(\/(?!api\/)[^'"`]*)\1/g;

const asRoute = (path) => path.replaceAll(/\$?\{[^}]*\}/g, '[id]').replace(/(.)\/$/, '$1');

test('every path the console writes names a page it has', () => {
	const has = new Set(pages());

	// Both halves of the console: the components, and the modules beside them. A `+page.js`
	// that redirects somewhere is naming a page as surely as an `href` is.
	for (const path of under(/\.(svelte|js)$/)) {
		for (const [, , wrote] of read(path).matchAll(written)) {
			assert.ok(
				has.has(asRoute(wrote)),
				`${named(path)} names the path ${wrote}, which is not a page the console has`
			);
		}
	}
});

test('nothing in the console navigates the document', () => {
	// A full navigation drops the signalling channel, and the console would look like a
	// server that had gone away. These are the ways one gets written by accident: SvelteKit's
	// own opt-out attribute, an assignment to the location, and a link out of the router.
	const reloads = [
		/data-sveltekit-reload/,
		/(window\.)?location(\.href)?\s*=[^=]/,
		/location\.(assign|replace)\(/,
		/<a[^>]*\starget=/
	];

	for (const path of components()) {
		const source = read(path);

		for (const spelling of reloads) {
			assert.doesNotMatch(
				source,
				spelling,
				`${named(path)} navigates the document — the signalling channel is one socket per tab and a reload drops it`
			);
		}
	}
});

// The styling standard, made checkable (ADR-0069). A standard nobody can check is prose,
// which is the argument #69 made for having a linter at all; these are the rules from
// `docs/agents/styling.md` that a machine can read.
//
// ESLint would be the obvious home, but the rules are about CSS inside `<style>` blocks and
// about one stylesheet's contents, and neither is something `eslint-plugin-svelte` sees.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { components, named, read, src } from './console.js';

const tokenFile = join(src, 'app.css');

// Comments first, then quoted strings: `content: ' ·unreviewed'` is a value that can hold a
// semicolon or a colon, and leaving it in makes the parser below read punctuation as CSS.
const stripped = (css) =>
	css.replaceAll(/\/\*[\s\S]*?\*\//g, '').replaceAll(/'[^']*'|"[^"]*"/g, "''");

// `<style lang="…">` is still a style block. Matching a bare `<style>` would make a whole
// component invisible to every rule in this file, which is the one failure a checker of
// checkable rules cannot have.
function styleOf(path) {
	const blocks = read(path).matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g);
	return stripped([...blocks].map((block) => block[1]).join('\n'));
}

// A component without its style block, and without its comments: what the markup rules below
// ask their question of. The comments go with the CSS because a component explains itself in
// prose, and a sentence saying what arrives *in:* a later ticket is not a motion directive.
function markupOf(path) {
	return read(path)
		.replaceAll(/<style[^>]*>[\s\S]*?<\/style>/g, '')
		.replaceAll(/<!--[\s\S]*?-->/g, '');
}

// Declarations only: everything after the innermost `{`, so a selector never reads as a
// property and an `@media` wrapper does not swallow the rule inside it.
function declarations(css) {
	return css.split('}').flatMap((chunk) => {
		const opened = chunk.lastIndexOf('{');
		if (opened === -1) return [];
		return chunk
			.slice(opened + 1)
			.split(';')
			.flatMap((declaration) => {
				const colon = declaration.indexOf(':');
				if (colon === -1) return [];
				return [[declaration.slice(0, colon).trim(), declaration.slice(colon + 1).trim()]];
			});
	});
}

// The scale, spelled out here as well as in `app.css`, because a token silently changing
// value is the failure this file exists to catch.
const tokens = {
	'--space-1': '0.25rem',
	'--space-2': '0.5rem',
	'--space-3': '0.75rem',
	'--space-4': '1rem',
	'--space-5': '1.5rem',
	'--space-6': '2rem',
	'--space-page-bottom': '6rem',
	'--type-1': '0.75rem',
	'--type-2': '0.85rem',
	'--type-3': '1rem',
	'--type-4': '1.1rem',
	'--type-5': '1.25rem',
	'--radius': '0.2rem',
	'--ground': '#14161a',
	'--raised': '#1d2026',
	'--ink': '#e8eaed',
	'--quiet': '#9aa1ad',
	'--rule': '#2f343d',
	'--refusal': '#e8a0a0',
	'--warning': '#e8c07a'
};

test('app.css defines the scale, and exactly the scale', () => {
	const defined = Object.fromEntries(
		declarations(stripped(read(tokenFile))).filter(([property]) => property.startsWith('--'))
	);

	assert.deepEqual(defined, tokens);
});

// Every CSS named colour, rather than the handful anybody would think to ban. A shorter list
// is a list somebody gets past with `crimson`.
const namedColours = new Set(
	`aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue
	blueviolet brown burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk
	crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgreen darkgrey darkkhaki
	darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen
	darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue
	dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite
	gold goldenrod gray green greenyellow grey honeydew hotpink indianred indigo ivory khaki
	lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan
	lightgoldenrodyellow lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen
	lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime limegreen linen
	magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen
	mediumslateblue mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream
	mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid
	palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum
	powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown
	seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen
	steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow
	yellowgreen`.split(/\s+/)
);

test('nothing outside app.css names a colour', () => {
	const written = /#[0-9a-f]{3,8}\b|\b(rgba?|hsla?|hwb|lab|lch|oklab|oklch|color|color-mix)\(/i;

	for (const path of components()) {
		for (const [property, value] of declarations(styleOf(path))) {
			const found = value.match(written);
			assert.equal(
				found,
				null,
				`${named(path)} writes the colour ${found?.[0]} — colours live in app.css and reach a component as a token`
			);

			for (const part of value.toLowerCase().split(/[\s,()/]+/)) {
				assert.ok(
					!namedColours.has(part),
					`${named(path)} sets ${property}: ${value} — colours live in app.css and reach a component as a token`
				);
			}
		}
	}
});

test('no literal margin, padding, gap, font-size, radius or offset outside the tokens', () => {
	// The offsets are here because they are spacing under another name, and `font` is here
	// because the shorthand carries a size: `font: 0.85rem/1.5 system-ui` is a font size that
	// the five properties the standard names would not have caught.
	const measured =
		/^(margin|padding|gap|row-gap|column-gap|font-size|border-radius|inset|top|right|bottom|left)(-|$)/;
	const shorthand = /^font$/;
	// `0` needs no unit and so needs no token; the keywords are not measurements.
	const unmeasured = /^(0|auto|inherit|initial|unset|revert|none|normal)$/;

	for (const path of [tokenFile, ...components()]) {
		const inAppCss = path === tokenFile;
		const css = inAppCss ? stripped(read(path)) : styleOf(path);

		for (const [property, value] of declarations(css)) {
			// `app.css` sets the root size the type scale is measured against, and a token for
			// the thing the tokens are relative to would be circular.
			if (!measured.test(property) && !(shorthand.test(property) && !inAppCss)) continue;

			for (const part of value.split(/\s+/)) {
				assert.ok(
					unmeasured.test(part) || part.startsWith('var(--'),
					`${named(path)} sets ${property}: ${value} — the scale in app.css holds these values`
				);
			}
		}
	}
});

test('every token a component uses is one app.css defines', () => {
	for (const path of components()) {
		for (const [, token] of styleOf(path).matchAll(/var\((--[a-z0-9-]+)\)/g)) {
			assert.ok(token in tokens, `${named(path)} uses ${token}, which app.css does not define`);
		}
	}
});

// v1 §8: motion is permitted in exactly one place, and that place is the talking indicator
// (ADR-0033) — one fixed rate, one fixed shape, reading unambiguously as on or off. Cognitive
// load is the thing being minimised, and a console that moves for any other reason spends the
// operator's attention on something that is not an operation. The indicator arrives with #41,
// and this is the check it will have to be written into rather than around.
test('the console renders no motion', () => {
	const moves = /^(animation|transition)(-|$)/;
	const directives = /\s(transition|in|out|animate):[a-z]/i;

	// `app.css` is in this one because the furniture is: a `transition` on the bare `button`
	// rule would put motion under every control in the console at once, which is the largest
	// version of what this refuses rather than an exception to it.
	for (const path of [tokenFile, ...components()]) {
		const css = path === tokenFile ? stripped(read(path)) : styleOf(path);

		for (const [property, value] of declarations(css)) {
			assert.ok(
				!moves.test(property),
				`${named(path)} sets ${property}: ${value} — motion is permitted in exactly one place, and it is the talking indicator`
			);
		}

		assert.ok(
			!css.includes('@keyframes'),
			`${named(path)} declares @keyframes — motion is permitted in exactly one place, and it is the talking indicator`
		);

		if (path === tokenFile) continue;

		assert.doesNotMatch(
			markupOf(path),
			directives,
			`${named(path)} carries a Svelte motion directive — motion is permitted in exactly one place, and it is the talking indicator`
		);
	}
});

test('a component declares nothing global', () => {
	for (const path of components()) {
		assert.ok(
			!styleOf(path).includes(':global('),
			`${named(path)} declares a global rule — shared furniture is a bare element selector in app.css`
		);
	}
});

test('no styling reaches an element past its style block', () => {
	// An inline `style` attribute is out of reach of every rule above, so it is refused
	// outright rather than checked. Nothing in the console has ever needed one.
	for (const path of components()) {
		assert.doesNotMatch(
			markupOf(path),
			/\sstyle=/,
			`${named(path)} carries an inline style attribute — styling belongs in a style block`
		);
	}
});

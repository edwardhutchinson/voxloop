// The Input seam, and whether the lint config actually holds it.
//
// ADR-0061 gives the client one enforced boundary and says a lint rule is "what turns that
// promise into a failing build". A rule nobody has watched fail is not that, which is why
// this file exists and why there is no equivalent for the rest of the config.
//
// It asks one question of every specifier, in every syntax that can carry one, because the
// seam's failure mode is not "the rule does nothing" — it is the rule agreeing with itself
// about `$lib/input/level.js` and disagreeing about the same path in backticks. A back door
// is as good as an open one.
//
// It lints source text at made-up paths rather than fixtures on disk, because the question
// is always *what does this rule say about a file living here*, and Input has no files yet.

import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { ESLint } from 'eslint';

import { inputSeam } from '../eslint.input-seam.js';

const web = fileURLToPath(new URL('..', import.meta.url));

const justTheSeam = new ESLint({ cwd: web, overrideConfigFile: true, baseConfig: inputSeam });

/** Every syntax that can name a module. */
const spellings = {
	'a static import': (it) => `import x from '${it}';`,
	'a re-export': (it) => `export { x } from '${it}';`,
	'a dynamic import': (it) => `export const later = () => import('${it}');`,
	'a dynamic import in backticks': (it) => `export const later = () => import(\`${it}\`);`
};

/** What the seam says about this specifier, spelled every way, from a file at `from`. */
async function verdicts(specifier, from = 'src/lib/Console.js') {
	const asked = Object.entries(spellings).map(async ([spelling, write]) => {
		const [result] = await justTheSeam.lintText(`${write(specifier)}\n`, { filePath: from });
		return [spelling, result.messages];
	});

	return Object.fromEntries(await Promise.all(asked));
}

const refuses = async (specifier) => {
	for (const [spelling, said] of Object.entries(await verdicts(specifier))) {
		assert.equal(said.length, 1, `${spelling} of '${specifier}' was not refused`);
		assert.match(said[0].message, /Input is a seam/, `${spelling} of '${specifier}'`);
	}
};

const allows = async (specifier, from) => {
	for (const [spelling, said] of Object.entries(await verdicts(specifier, from))) {
		assert.deepEqual(said, [], `${spelling} of '${specifier}' was refused and should not be`);
	}
};

test('nothing outside Input may reach past its interface', async (t) => {
	for (const specifier of [
		'$lib/input/level.js',
		'$lib/input/sources/keyboard.js',
		// An index one level down is still an internal one.
		'$lib/input/sources/index.js',
		// The trailing slash, and the relative walk, are the two ways round it.
		'$lib/input/',
		'./input/level.js',
		'../input/level.js',
		'../../input/level.js'
	]) {
		await t.test(specifier, () => refuses(specifier));
	}
});

test('the interface is the way in', async (t) => {
	for (const specifier of ['$lib/input', '$lib/input/index.js', './input/index.js']) {
		await t.test(specifier, () => allows(specifier));
	}
});

test('and everything else is somebody else’s business', async (t) => {
	for (const specifier of [
		// A package that happens to have an `input` directory is not this Input.
		'some-pkg/input/thing.js',
		// Nor is a sibling whose name merely starts the same way.
		'$lib/inputs/x.js',
		'$lib/server.js',
		'svelte'
	]) {
		await t.test(specifier, () => allows(specifier));
	}
});

test("Input's own files reach their own internals", async (t) => {
	for (const specifier of ['../level.js', '$lib/input/level.js']) {
		await t.test(specifier, () => allows(specifier, 'src/lib/input/sources/keyboard.js'));
	}
});

// Everything above tests the rule. This tests that the build is running it: `eslint.config.js`
// could stop composing the seam rule tomorrow and every assertion above would still pass.
test('and the console is linted with it', async () => {
	const asTheBuildRunsIt = new ESLint({ cwd: web });

	const [result] = await asTheBuildRunsIt.lintText(
		"<script>\n\timport { keyboard } from '$lib/input/sources/keyboard.js';\n</script>\n",
		{ filePath: 'src/lib/Console.svelte' }
	);

	// The full config has opinions about the rest of the snippet too — an unused import, for
	// one — so this asks whether the seam rule is among them, not whether it is alone.
	assert.ok(
		result.messages.some(({ ruleId }) => ruleId === 'no-restricted-imports'),
		`the seam rule did not run: ${JSON.stringify(result.messages)}`
	);
});

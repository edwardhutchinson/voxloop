// The Input seam, and whether the lint config actually holds it.
//
// ADR-0061 gives the client one enforced boundary and says a lint rule is "what turns that
// promise into a failing build". A rule nobody has watched fail is not that, which is why
// this file exists and why there is no equivalent for the rest of the config.
//
// It loads the seam rule on its own rather than the whole of `eslint.config.js`, and lints
// source text at a made-up path, because the question is always *what does this rule say
// about a file living here* — and the exemption for Input's own files is a question about a
// path that has no file behind it yet.

import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { ESLint } from 'eslint';

import { inputSeam } from '../eslint.input-seam.js';

const justTheSeam = new ESLint({
	cwd: fileURLToPath(new URL('..', import.meta.url)),
	overrideConfigFile: true,
	baseConfig: inputSeam
});

/** What the seam rule says about this source, were it to live at this path. */
async function complaints(source, at) {
	const [result] = await justTheSeam.lintText(source, { filePath: at });
	return result.messages.map(({ ruleId, message }) => ({ ruleId, message }));
}

const refused = (said, by) => {
	assert.equal(said.length, 1, `expected one complaint, got ${JSON.stringify(said)}`);
	assert.equal(said[0].ruleId, by);
	assert.match(said[0].message, /Input is a seam/);
};

test('a module outside Input may not reach past its interface', async () => {
	refused(
		await complaints(
			"import { keyboard } from '$lib/input/sources/keyboard.js';\n",
			'src/lib/Console.js'
		),
		'no-restricted-imports'
	);
});

test('nor may it, by taking the relative way round', async () => {
	refused(
		await complaints(
			"import { keyboard } from './input/sources/keyboard.js';\n",
			'src/lib/Console.js'
		),
		'no-restricted-imports'
	);
});

test('nor by importing it at runtime', async () => {
	// `no-restricted-imports` does not read `import()`, so this one is held by the syntax rule
	// standing beside it. The seam does not care which of them says no.
	refused(
		await complaints(
			"export const later = () => import('$lib/input/sources/keyboard.js');\n",
			'src/lib/Console.js'
		),
		'no-restricted-syntax'
	);
});

test('the interface is the way in', async () => {
	assert.deepEqual(
		await complaints("import { sources } from '$lib/input';\n", 'src/lib/Console.js'),
		[]
	);
	assert.deepEqual(
		await complaints("export const later = () => import('$lib/input');\n", 'src/lib/Console.js'),
		[]
	);
});

test("Input's own files reach their own internals", async () => {
	assert.deepEqual(
		await complaints("import { level } from '../level.js';\n", 'src/lib/input/sources/keyboard.js'),
		[]
	);
});

// Everything above tests the rule. This tests that the build is running it: `eslint.config.js`
// could stop composing the seam rule tomorrow and every assertion above would still pass.
test('and the console is linted with it', async () => {
	const asTheBuildRunsIt = new ESLint({ cwd: fileURLToPath(new URL('..', import.meta.url)) });

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

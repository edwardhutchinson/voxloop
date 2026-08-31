// The one boundary in the client that is a failing build rather than a review comment.
//
// ADR-0061 gives the server compiler-checked module privacy and the client nothing
// equivalent, so it picks one seam to enforce with a lint rule and holds the other three by
// convention. Input is that seam, because ADR-0020 makes a written promise about it — the
// Tauri wrapper may only ever *add a source* to Input — and a promise nothing checks is the
// kind that quietly stops being true.
//
// The whole of the rule is: `$lib/input` is the way in, and nothing underneath it is.
// Import a source, a level reading or a binding table directly and Input is no longer a
// seam, it is a directory.
//
// It lives in its own file so the test can load the rule the build actually uses rather
// than a copy of it that can drift.

const wayIn = '$lib/input';

/** Every spelling of "somewhere under Input", including the relative way round. */
const underneath = [`${wayIn}/**`, '**/input/**'];

/** …except the interface itself, which is what everyone is supposed to import. */
const exceptTheWayIn = [`!${wayIn}`, `!${wayIn}/index.js`, '!**/input/index.js'];

const message =
	'Input is a seam (ADR-0061): import it as `$lib/input`. ' +
	'Reaching past its interface is how the Tauri wrapper stops being able to just add a source.';

// `no-restricted-imports` reads `import` and `export ... from` and stops there, so on its own
// it leaves `import()` as an unlocked back door into exactly what it is guarding. The second
// rule is the same sentence said about the other syntax, not a second policy: anything under
// Input that is not its interface.
const underneathAtRuntime = /(^\$lib|\/)input\/(?!index\.js$)/;

export const inputSeam = [
	{
		rules: {
			'no-restricted-imports': [
				'error',
				{ patterns: [{ group: [...underneath, ...exceptTheWayIn], message }] }
			],
			'no-restricted-syntax': [
				'error',
				{ selector: `ImportExpression > Literal[value=${underneathAtRuntime}]`, message }
			]
		}
	},
	{
		// Input's own files are inside the seam, so none of this applies to them.
		files: ['src/lib/input/**'],
		rules: { 'no-restricted-imports': 'off', 'no-restricted-syntax': 'off' }
	}
];

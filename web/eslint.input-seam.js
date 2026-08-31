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

/**
 * Somewhere under Input that is not its interface.
 *
 * One expression, used by both rules below, because the two of them are one policy said
 * about two syntaxes. Writing it twice — globs for one, a regex for the other — is how they
 * came to disagree about `$lib/input/` in an earlier draft of this file.
 *
 * It is anchored to the ways the console can name its own module: `$lib/input/…` and the
 * relative walk to the same place. A bare specifier is a package, so `some-pkg/input/x.js`
 * is somebody else's directory called input and none of Input's business.
 */
const underneath = String.raw`^(?:\$lib|\.\.?(?:/\.\.)*)/input/(?!index\.js$).*$`;

const message =
	'Input is a seam (ADR-0061): import it as `$lib/input`. ' +
	'Reaching past its interface is how the Tauri wrapper stops being able to just add a source.';

// `no-restricted-imports` reads `import` and `export ... from` and stops there, which leaves
// `import()` an unlocked back door into exactly what is being guarded. Hence the second
// rule — and hence two selectors, because a specifier in backticks is a TemplateLiteral and
// walks straight past a rule that only knows about Literal.
//
// A selector ends its regex at the first bare `/`, and this one is full of path separators,
// so they are escaped on the way in. Same expression, spelled for the reader that gets it.
const inASelector = underneath.replaceAll('/', String.raw`\/`);

const atRuntime = [
	`ImportExpression > Literal[value=/${inASelector}/]`,
	`ImportExpression > TemplateLiteral > TemplateElement[value.raw=/${inASelector}/]`
];

export const inputSeam = [
	{
		rules: {
			'no-restricted-imports': ['error', { patterns: [{ regex: underneath, message }] }],
			'no-restricted-syntax': ['error', ...atRuntime.map((selector) => ({ selector, message }))]
		}
	},
	{
		// Input's own files are inside the seam, so none of this applies to them.
		files: ['src/lib/input/**'],
		rules: { 'no-restricted-imports': 'off', 'no-restricted-syntax': 'off' }
	}
];

// The console's lint config. Deliberately small: the point is the mechanism, and a house
// style argued rule by rule is a different conversation from having a linter at all.
//
// The one rule here that carries a promise rather than a preference is the Input seam, and
// it lives in `eslint.input-seam.js` beside its test.

import js from '@eslint/js';
import globals from 'globals';
import svelte from 'eslint-plugin-svelte';

import { inputSeam } from './eslint.input-seam.js';

export default [
	// `dist/` is the built bundle and `.svelte-kit/` is generated; neither is written here.
	{ ignores: ['dist/', '.svelte-kit/'] },

	js.configs.recommended,
	...svelte.configs.recommended,

	{ languageOptions: { globals: globals.browser } },

	// The build and test tooling runs in Node, not the browser.
	{
		files: ['*.js', 'tests/**/*.js'],
		languageOptions: { globals: globals.node }
	},

	...inputSeam
];

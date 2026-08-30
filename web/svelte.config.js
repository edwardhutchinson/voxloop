import adapter from '@sveltejs/adapter-static';

/** @type {import('@sveltejs/kit').Config} */
export default {
	kit: {
		// ADR-0037: the release build of the Rust binary embeds `dist/`, and no Node runtime
		// is deployed. `dist/` is never committed.
		adapter: adapter({
			pages: 'dist',
			assets: 'dist',
			fallback: 'index.html',
			precompress: false,
			strict: true
		})
	}
};

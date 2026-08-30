import { sveltekit } from '@sveltejs/kit/vite';

export default {
	plugins: [sveltekit()],
	server: {
		// Development is two processes: this dev server, proxying to the Rust binary. Release
		// is one artefact. The binary's certificate is whatever the developer generated, so
		// the proxy does not verify it.
		proxy: {
			'/api': { target: 'https://localhost:8443', secure: false }
		}
	}
};

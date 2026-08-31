import { sveltekit } from '@sveltejs/kit/vite';

export default {
	plugins: [sveltekit()],
	server: {
		// Development is two processes: this dev server, proxying to the Rust binary. Release
		// is one artefact. The binary's certificate is whatever the developer generated, so
		// the proxy does not verify it.
		proxy: {
			// `ws` because the signalling channel is under `/api` too, and a proxy that only
			// forwards requests leaves the console with no live state at all — which looks
			// like the server being broken rather than the proxy being half-configured.
			'/api': { target: 'https://localhost:8443', secure: false, ws: true }
		}
	}
};

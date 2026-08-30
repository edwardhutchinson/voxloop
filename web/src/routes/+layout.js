// The console is a single-page application after sign-in, and there is no search engine to
// satisfy: everything it renders arrives over the signalling WebSocket (ADR-0037).
export const ssr = false;
export const prerender = false;

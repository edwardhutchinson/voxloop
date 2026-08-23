# The client ships as static assets embedded at release; no Node runs in the deployment

[ADR-0020](./0020-the-browser-is-the-client.md) makes one SvelteKit bundle the client on every platform. This ADR fixes how that bundle reaches a customer: **SvelteKit builds with `adapter-static`, and the release build of the Rust binary embeds the result. No Node runtime is deployed.**

## SSR buys VoxLoop nothing and costs a runtime

`adapter-node` would give server-side rendering and a second server process. VoxLoop is an authenticated single-organisation console with no search engine to satisfy and no anonymous first paint to optimise; after sign-in its entire content arrives over the signalling WebSocket as [ADR-0019](./0019-presence-is-one-versioned-document-scoped-to-reach.md)'s presence document. SSR would render a login form fractionally sooner in exchange for putting a Node runtime and an npm dependency tree inside a customer network that may be air-gapped, to be deployed, supervised and patched by their ops team forever.

Node therefore stays a **build-time** dependency, on the developer's machine and in CI, and never ships. Combined with [ADR-0038](./0038-sqlite-behind-domain-shaped-repositories.md) and [ADR-0040](./0040-one-binary-one-unit-four-moving-parts.md), that is what holds the deployment to four moving parts.

## The embed is a Cargo feature, and `dist` is not committed

Embedding assets unconditionally would mean a bare `cargo build` requires Node and a completed `npm install` — friction for the developer and worse friction for a coding agent that only wants to build the backend. The obvious fix is to commit the built output. It was rejected on two costs:

**Unbounded repository growth.** Vite content-hashes asset filenames, so every UI change *adds* files rather than replacing them and git retains every generation forever. That is not noise that can be pruned later without rewriting history.

**Silent staleness ships the wrong console.** Edit a component, forget to rebuild, commit — and the binary handed to a customer contains the previous UI. Nothing fails and no test catches it; the artefact simply disagrees with its source. On a product whose standing rule is that displayed state must be factual, a build output that can quietly contradict the code is a poor thing to keep in the tree.

So the embed sits behind a **Cargo feature, off by default and on for release**. A bare `cargo build` never needs `dist` at all, because the only build that embeds is the release build, which happens in one place with Node present.

This also matches how the work is actually done. **Development is two processes** — Vite's dev server with hot reload, proxying API and WebSocket to the Rust binary — and **release is one artefact**. Neither needs build output in version control.

## Consequences

- **A release build has an ordering requirement**: `npm run build` before `cargo build --release --features <embed>`. It belongs in CI and in the release documentation, because it is the one step whose omission produces a binary that runs and is wrong.
- **The frontend cannot be rebuilt from a source checkout without Node**, deliberately. Reproducing a release means reproducing the toolchain, which is a release-engineering obligation and belongs with the deployment and packaging work.
- **Svelte and SvelteKit are confirmed with nothing challenging them.** `adapter-static` is a first-class adapter, and the client is a single-page application after sign-in regardless.
- **If build output is ever committed after all**, it needs a CI check that rebuilds and fails on any difference. That closes the staleness hole and leaves only the growth cost, knowingly accepted rather than stumbled into.

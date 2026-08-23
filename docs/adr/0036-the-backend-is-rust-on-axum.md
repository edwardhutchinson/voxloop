# The backend is Rust on axum, decided on its merits rather than inherited

[ADR-0006](./0006-mediasoup-carries-the-audio.md) chose mediasoup's first-party Rust API and recorded "the backend is Rust" as a *consequence*. That is a decision made sideways, and [#13](https://github.com/edwardhutchinson/voxloop/issues/13) is the last cheap moment to take it deliberately. It is confirmed: **the signalling, permission, presence and configuration server is Rust, built on axum and tokio.**

## The binding risk that would justify reopening is not real

The case for reopening was that mediasoup's documentation, examples and demo are Node-first, so the Rust crate might be a lagging binding maintained on someone's goodwill. It is not. As of 23 August 2026 the `mediasoup` crate is at **0.27.0** and npm `mediasoup` at **3.26.0**, both published on 18 August 2026 — the Rust crate is currently one release *ahead*. Releases have tracked each other weekly through July and August. ADR-0006's claim of same-cadence first-party support holds empirically rather than aspirationally.

TypeScript's real advantage was velocity — one language shared with the client, a larger corpus for coding agents, and mediasoup's own tutorials. It loses to what this codebase actually is. The code this ticket governs is permission evaluation, connection-state ladders, emission enforcement and the presence document, where the failure mode is an open microphone nobody was told about. Exhaustive matching over `none | monitor | emit | control`, over three connection states, and over observed-versus-asserted provenance is the class of bug Rust removes by construction, and [ADR-0016](./0016-displayed-state-is-observed-or-asserted.md) has already made "the console displayed something that was never true" the defining defect of this product.

## The tax is real and is written down rather than discovered

The same weekly release cadence costs more in Rust than in Node. npm's `3.x` minor bumps are non-breaking under semver; cargo treats every `0.x` minor bump as breaking. ADR-0006 was written against 0.25.2 and five releases have landed since. So the crate is **pinned to an exact version**, and upgrades are scheduled work with a changelog read attached — never `cargo update`.

The other cost is honest too: every *ordinary* thing — migrations, configuration, HTTP plumbing — is somewhat more effortful in Rust than in TypeScript. If velocity later becomes the binding constraint, the place to spend it is a thinner admin console, not a second language in the deployment.

## axum, and mediasoup constrains nothing here

mediasoup imposes **no async runtime**. Its dependency tree is `async-executor`, `futures-lite`, `async-channel` and `async-lock`, with no `tokio` anywhere; it runs its own executor on its own threads and exposes plain futures. Any Rust web framework composes with it.

The one visible signal pointing elsewhere is that mediasoup's own Rust example uses actix-web with `actix-web-actors` — and it is a weak signal, since that crate has had no release since **August 2024** and has been superseded by `actix-ws`. The example demonstrates mediasoup, not a framework recommendation.

axum wins on ecosystem mass: tokio, hyper and tower underneath, maintained by the tokio team, and far the largest body of published examples and training data. For a solo developer working with coding agents that is a direct velocity argument, which is the balance [#13](https://github.com/edwardhutchinson/voxloop/issues/13) asked to strike.

## Consequences

- **mediasoup's callbacks fire on mediasoup's threads, not tokio's.** Its event listeners are synchronous closures invoked from its own executor, so the bridge into the axum application is channels, never a blocking call or a borrowed tokio handle. Getting this wrong produces cross-runtime stalls that appear only under load, which is the worst time to find them.
- **The crate is pinned exactly**, and a mediasoup upgrade is a task with a changelog review, not a dependency refresh.
- **`tracing` is the logging facade**, chosen now because it is unavoidable and because structured spans are what make the map's still-open observability work cheap rather than a retrofit. [ADR-0025](./0025-credentials-are-administered-because-there-is-no-email.md)'s first-start bootstrap code goes down it, which is what makes log-read access equivalent to administrator at that moment.
- **One binary crate with module boundaries per seam**, not a Cargo workspace. Crate boundaries would enforce the seams at compile time and cost a solo developer real friction for it; nothing has a second consumer yet. Split when one appears — this is deliberately the reversible choice in this ADR and needs no revisiting ceremony.

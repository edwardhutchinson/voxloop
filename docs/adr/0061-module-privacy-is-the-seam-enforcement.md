# Module privacy is the seam enforcement, and there is still no workspace

[ADR-0036](./0036-the-backend-is-rust-on-axum.md) chose one binary crate with module boundaries per seam over a Cargo workspace, because crate boundaries "would enforce the seams at compile time and cost a solo developer real friction for it", and called itself deliberately reversible. [#26](https://github.com/edwardhutchinson/voxloop/issues/26) reopened it: iterating on isolated parts is precisely the argument that would buy that friction.

**It holds, on a fact ADR-0036 never stated: Rust already enforces this inside one crate.** An item private to a module is invisible to a sibling module whether or not they share a crate. The enforcement is free and the compiler checks it, provided the discipline is explicit.

**Each module makes exactly its interface `pub(crate)`. Everything else stays private to the module.**

A workspace buys one thing on top of that. It makes a violation harder to commit by accident, because widening `pub(crate)` is a one-word edit while adding a crate dependency is a `Cargo.toml` change somebody notices. That is a weak argument against build times and a manifest per module. ADR-0036's condition for splitting stands unchanged: when a module gains a consumer outside the binary.

## The client is enforced in one place only

TypeScript has no equivalent and SvelteKit checks nothing. The available mechanism is a lint rule restricting imports, and it applies to **Input alone**.

That is not a shortfall. Input is the only seam in VoxLoop with real variation, and [ADR-0020](./0020-the-browser-is-the-client.md) makes a written promise about it: the Tauri wrapper may only ever add a source to the seam, and needing a second code path anywhere else means the one-client story has broken. A lint rule is what turns that promise into a failing build. Audio, Console and Session get the same seam rule by convention, because nothing is swapped through them and enforcing a boundary nothing crosses is cost without return.

## Consequences

- **The two halves of the system are enforced at different strengths, deliberately.** The server gets compiler-checked privacy because it is free. The client gets one lint rule because that is where its only real seam is.
- **Widening `pub(crate)` is the thing to watch.** It is the cheapest edit in the codebase and it is how a seam quietly stops existing, the same failure [ADR-0024](./0024-identity-is-a-replaceable-front-door.md) named when it warned that calling the password check directly is how the identity seam disappears.
- **A module's tests live with the module and go through its interface.** If a test needs something private, either the interface is the wrong shape or the test is reaching past the seam that makes the module worth having.

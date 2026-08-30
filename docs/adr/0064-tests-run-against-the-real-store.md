# Tests run against the real store, and there is no in-memory repository

A fake exists at exactly two seams, **Media plane** and **Synthesis**. Every other module is tested against its real implementation.

**An in-memory repository is ruled out by name**, in the same spirit as [ADR-0038](./0038-sqlite-behind-domain-shaped-repositories.md) ruling out `sqlx::Any`: both are the plausible-looking thing that quietly removes the guarantee you were buying. A hand-written in-memory store does not enforce foreign keys, does not have SQLite's type affinity, does not serialise writers the way WAL mode does, and does not fail on the constraint that would have caught the bug. The failure mode is the worst available. The tests pass, the product is broken, and nobody looks at the repository layer because it is green.

SQLite makes avoiding this cheap in a way most databases do not. The store is one file, migrations are embedded in the binary and run at startup, and the whole persistent set is small and cold. A test opens a temporary database, migrates it, and throws it away.

**The two that get fakes earn it by being slow, external and nondeterministic.** Media plane is a C++ subprocess negotiating ICE and DTLS. Synthesis is another subprocess carrying a neural model file, the largest artefact in the deployment. Neither belongs in a fast test loop, and by [ADR-0062](./0062-the-call-graph-is-acyclic-and-effects-modules-are-sinks.md) both are sinks, so their fakes are recorders rather than simulations. Identity is a password hash, State authority is already in memory, Configuration is a file. None of those is worth faking.

## Consequences

- **The repository seam will never have a second adapter.** Tests use the real store and Postgres may never arrive, so ADR-0038's carefully domain-shaped traits are a seam nothing is ever swapped through, which is exactly what [ADR-0021](./0021-ptt-input-is-a-level-with-liveness.md) warned would not fit when somebody needs it. Accepted knowingly. **Nobody should build a fake repository thinking they are fixing an oversight**: it is the alternative that was rejected, for the reason above.
- **Identity and the recording tap sit in the same position**, one adapter and no double. [`docs/spec/modules.md`](../spec/modules.md) marks all three rather than presenting every seam as equally exercised.
- **The Media plane fake carries the weight**, because [ADR-0063](./0063-the-media-plane-executes-routing-it-never-computes-it.md) puts every routing decision above it. It must record instructions faithfully and do nothing else; a fake that starts making decisions is the failure ADR-0063 exists to prevent.
- **Append-only has somewhere to be tested.** ADR-0038 requires that the audit log's append-only property be tested rather than merely intended, and testing against the real store is what makes that test mean anything.

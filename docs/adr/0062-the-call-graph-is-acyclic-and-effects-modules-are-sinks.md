# The call graph is acyclic, and the effects modules are sinks

Eleven modules with clean interfaces buy nothing if they form a mesh. A module you cannot build or test without the other ten is not isolated however good its interface looks, and isolation is the whole of what [#26](https://github.com/edwardhutchinson/voxloop/issues/26) asked for.

**No module may call a module that can call it back.** The graph is acyclic and [`docs/spec/modules.md`](../spec/modules.md) records the direction of every edge.

**Media plane and Synthesis are sinks.** They call nothing. When they have something to report they emit it on a channel, and something above them decides what it means.

Two existing decisions were instances of this without saying so. [ADR-0038](./0038-sqlite-behind-domain-shaped-repositories.md) and [ADR-0039](./0039-live-state-is-in-process-behind-one-state-authority.md) keep persistence and the state authority ignorant of each other, computing blast radius on the live side and passing it into the transaction as a value so neither seam has to know about the other. And [ADR-0036](./0036-the-backend-is-rust-on-axum.md) already requires a channel bridge out of mediasoup, because its callbacks fire on its own threads and a blocking call across runtimes stalls under load. The sink rule turns a threading workaround into a design rule that the chosen mechanism already satisfies.

**The sink rule is what makes the fakes work.** A module that calls nothing is trivially substitutable: its fake records what it was told and reports nothing back, so no test has to simulate a callback arriving at an awkward moment. That is why Media plane and Synthesis are exactly the two modules [ADR-0064](./0064-tests-run-against-the-real-store.md) fakes. The dependency rule and the testing decision are one decision seen from two ends.

## Consequences

- **A sink cannot refuse.** A caller never learns from the call itself whether the effect succeeded, because the answer comes back asynchronously or not at all. That is already true and already costly: [ADR-0029](./0029-an-announcement-is-an-ordinary-transmission.md) gives announcement callers a synchronous accept and nothing more, which is why a dead sidecar loses every announcement silently. The rule makes that shape general rather than an accident of one feature.
- **The health of a sink is observed, never returned.** [ADR-0040](./0040-one-binary-one-unit-four-moving-parts.md) has the binary supervise both subprocesses precisely so their health is known as a matter of course, and this is the interface-level statement of the same thing.
- **Transport is a caller, not a sink**, despite looking like an effects module. It receives requests, so it sits at the top of the graph rather than the bottom, and [ADR-0054](./0054-every-operation-declares-its-authorisation.md)'s route registration is the top edge of every server-side call chain.
- **A cycle is a design error, not a refactor.** Two modules that need each other are one module with an interface drawn in the wrong place, and the fix is to move the seam rather than to break the rule.

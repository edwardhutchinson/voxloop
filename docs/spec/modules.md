# Modules and seams

Every part of VoxLoop, what its interface promises, and who may call it. The rules behind the shape of this list are [ADR-0060](../adr/0060-a-seam-names-domain-operations.md) (what a seam is), [ADR-0061](../adr/0061-module-privacy-is-the-seam-enforcement.md) (how it is enforced), [ADR-0062](../adr/0062-the-call-graph-is-acyclic-and-effects-modules-are-sinks.md) (who may call whom) and [ADR-0064](../adr/0064-tests-run-against-the-real-store.md) (what is faked). This file is the enumeration.

**Read this before the ADRs.** Sixty ADRs record why VoxLoop is the way it is; this page is what it *is*.

**The rule, once.** A seam's interface names domain operations and domain types, and nothing on the caller's side can tell which adapter is behind it. If a type or an error crossing the interface names the mechanism, the seam has failed.

**Modules are not processes.** A deployment is one unit and one SQLite file, with the mediasoup worker linked into the binary and the sidecar beside it ([ADR-0040](../adr/0040-one-binary-one-unit-four-moving-parts.md), [ADR-0070](../adr/0070-the-mediasoup-worker-is-a-thread-of-this-process.md)). Seven of these modules are inside the Rust binary and four are inside the client bundle.

**Exercised** says what the seam actually has on the other side of it today. A seam with one adapter and no test double is a reserved space, not a proven boundary ([ADR-0021](../adr/0021-ptt-input-is-a-level-with-liveness.md)), and is marked as such rather than presented as an equal.

## Server modules

| Module | Interface promises | Calls | Exercised |
|---|---|---|---|
| **Transport** | HTTP routes, the signalling WebSocket, static assets. Every route registered with its authorisation requirement as a mandatory argument ([ADR-0054](../adr/0054-every-operation-declares-its-authorisation.md)), so an unruled operation fails the build. The only module that may name an HTTP status | Authorisation, Configuration, State authority, Identity, Media plane, Synthesis | Real. Top of the graph |
| **Authorisation** | Evaluates one of the six requirements against a caller. Answers permitted or refused, never *why* in a form the caller can act on | Configuration, State authority | Real |
| **Identity** | Resolves a credential to an internal user id, and nothing else ([ADR-0024](../adr/0024-identity-is-a-replaceable-front-door.md)). Nothing downstream learns how it was obtained | Configuration | **One adapter, no double.** Local passwords only; SSO is the reserved second |
| **Configuration** | Users, roles, loops, the (role, loop) grid, eligibility, service principals, the pronunciation dictionary, personalisation, the sign-ins and enrolment codes held against a user, and the audit log. Domain-shaped repository traits with the transaction handle passed in ([ADR-0038](../adr/0038-sqlite-behind-domain-shaped-repositories.md)) | — | **One adapter, no double.** Real SQLite in tests; Postgres is the reserved second and may never arrive |
| **State authority** | Every live fact: sessions, occupancy, subscriptions, arms, key state, connection state, media path state, loop health. Single writer, holds nothing durable ([ADR-0039](../adr/0039-live-state-is-in-process-behind-one-state-authority.md)). Computes the audience and the presence document as projections | — | Real, and already in memory |
| **Media plane** | Takes *these subscribers should hear this producer* and executes it ([ADR-0063](../adr/0063-the-media-plane-executes-routing-it-never-computes-it.md)). Never computes who. Owns mediasoup, transports, producers, consumers and the recording tap. One Worker, one Router, one shared `WebRtcServer` port, and the worker is a thread of this process rather than a child of it ([ADR-0070](../adr/0070-the-mediasoup-worker-is-a-thread-of-this-process.md)). A client's own media negotiation crosses it as an opaque value, so no mediasoup type leaves | **Sink.** Reports on a channel, and says things to one session on a channel handed *in* with that session's path | Fake and real. The fake is a recorder |
| **Synthesis** | Text in, audio into the media plane ([ADR-0030](../adr/0030-speech-synthesis-is-a-swappable-sidecar.md)). Accepts synchronously and synthesises later | **Sink.** Reports on a channel | Fake and real |

**The on-box CLI is not a module.** It is a second entry point into the same binary, sitting beside Transport at the top of the graph: it receives an invocation and calls Configuration, so nothing calls into it and no cycle is created ([ADR-0062](../adr/0062-the-call-graph-is-acyclic-and-effects-modules-are-sinks.md)). What makes it not a seam is that it promises nothing to anybody — it is **outside the authorisation model by design**, evaluating no requirement and resolving no principal, which is where [`api-surface.md`](./api-surface.md) enumerates it. It is the one caller of Configuration that Authorisation never stands in front of, and that is the whole of why shell access to the host is the highest privilege in the system.

**The sign-in clock is not a module either.** Ending a sign-in that has stood in the lobby for 24 hours with no deliberate act is a tick, a read from the state authority and a write through Configuration — so it sits beside Transport and the CLI at the top of the graph, calls down, and is called by nothing ([ADR-0062](../adr/0062-the-call-graph-is-acyclic-and-effects-modules-are-sinks.md)). It promises nothing to anybody, which is what keeps it off the list above. It is the second place the two state seams meet, and it meets them the same way blast radius does: the live side answers which sign-ins hold a session, and that answer is handed to the write as a value.

**Supervision is not a module.** The media plane calls nothing and reports on a channel ([ADR-0062](../adr/0062-the-call-graph-is-acyclic-and-effects-modules-are-sinks.md)), and something has to decide what a report means — which cannot be the sink. So it is a loop over that channel writing live state, sitting at the top of the graph beside Transport, the CLI and the sign-in clock, calling down and called by nothing. It promises nothing to anybody and has no interface to substitute, which is what keeps it off the list above.

**Persistence is not a module.** It is Configuration's seam. Nothing else in the system reaches a database.

**Audit is not a module either.** A grid edit and its audit entry commit in one transaction ([ADR-0038](../adr/0038-sqlite-behind-domain-shaped-repositories.md)), so the entry is written by the module that owns the write it records. A separate Audit module would be one you could forget to call.

**Blast radius is the only place the two state seams meet**, and they meet by passing a value. State authority computes it, the caller hands it to Configuration's transaction, and neither knows about the other ([ADR-0039](../adr/0039-live-state-is-in-process-behind-one-state-authority.md)).

## Client modules

| Module | Interface promises | Calls | Exercised |
|---|---|---|---|
| **Session** | The socket, sign-in, assume, resume by session id, gap events ([ADR-0041](../adr/0041-a-session-is-resumed-by-name.md)). Owns the presence document and hands it out; nothing else talks to the server | — | Real |
| **Input** | A source reports intent, liveness, and whether its reading is stale-high ([ADR-0021](../adr/0021-ptt-input-is-a-level-with-liveness.md)). Sources are additive and a source never knows which emission mode it serves | — | **The only real seam in VoxLoop.** One source today, the on-screen key control; the keyboard bindings are #42 and the Tauri native hotkey is #61 |
| **Audio** | Client-side mixing over per-talker streams, per-loop volume, loudest-wins overlap, priority at full gain ([ADR-0007](../adr/0007-the-client-emits-one-stream.md), [ADR-0045](../adr/0045-priority-defeats-attenuation-and-nothing-else.md)) | Session | Real. One uplink and one stream per audible talker; volume, loudest-wins and priority are #44 and #45 |
| **Console** | Board and ledger over one loop list, the transmit bar, the hail picker ([ADR-0032](../adr/0032-the-console-is-two-views-of-one-loop-list.md)). Renders server-pushed state and never optimistically | Session, Input, Audio | Real |

**Input is the only client seam with enforcement** — a lint rule on imports ([ADR-0061](../adr/0061-module-privacy-is-the-seam-enforcement.md)) — because [ADR-0020](../adr/0020-the-browser-is-the-client.md) promises the Tauri wrapper may only ever add a source to it. The other three hold the same rule by convention.

## What must never happen

- A cycle. Two modules that need each other are one module with the seam in the wrong place ([ADR-0062](../adr/0062-the-call-graph-is-acyclic-and-effects-modules-are-sinks.md)).
- A `mediasoup::Producer`, a `rusqlite::Row`, or an HTTP status crossing outward from a module that does not own it ([ADR-0060](../adr/0060-a-seam-names-domain-operations.md)). The client's media negotiation crosses the media plane's seam as a value nothing above it can read, which is what keeps that true of `DtlsParameters` and `RtpParameters` too.
- Routing computed inside Media plane ([ADR-0063](../adr/0063-the-media-plane-executes-routing-it-never-computes-it.md)).
- An in-memory repository fake ([ADR-0064](../adr/0064-tests-run-against-the-real-store.md)).
- A module reaching past a sibling's interface into its internals. The compiler stops this on the server and the lint rule stops it at Input ([ADR-0061](../adr/0061-module-privacy-is-the-seam-enforcement.md)); review stops it across the rest of the client.

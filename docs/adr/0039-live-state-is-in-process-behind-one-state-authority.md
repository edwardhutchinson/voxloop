# Live state is in-process behind one state authority, and a restart ends every session

Sessions, occupancy, subscriptions, arms, key state, connection state, beacon counts and audience projections live in **plain in-memory structures owned by the server process**. There is no second store, and there is no Redis.

## A restart genuinely ends every session, and persisting occupancy would be a lie

The instinct is to persist occupancy so that a restart does not empty the control room into the lobby mid-event. It is the wrong instinct here, and the reason is not squeamishness about complexity.

**The media plane cannot survive a restart at any price.** The mediasoup Worker is a child of the server process ([ADR-0006](./0006-mediasoup-carries-the-audio.md)) and every WebRtcTransport dies with it. Restored occupancy would therefore describe roles as occupied by sessions that demonstrably have no audio path — precisely the misrepresentation [ADR-0016](./0016-displayed-state-is-observed-or-asserted.md) exists to ban, and one that [ADR-0005](./0005-occupancy-means-listening-not-signed-in.md) would render as loops `staffed` by people who cannot hear a thing.

A restart is not distinguishable from [ADR-0018](./0018-no-signalling-channel-means-no-emission-path.md)'s `disconnected` threshold, and the honest post-restart staffing state of every loop is `vacant`. Users remain **signed in** — [ADR-0026](./0026-one-credential-and-the-media-path-carries-none.md)'s cookie is unaffected — and must **assume** their role again, which is the correct reading of [ADR-0023](./0023-sign-in-is-to-the-application-and-a-role-is-assumed.md): the outer act survives, the session does not.

This makes a backend restart an operational event with a real cost during an incident. That is a property of the system and belongs in the spec as one, not in a footnote.

## The state authority is a boundary, not a storage interface

All live state sits behind a single module — the **state authority** — exposing domain operations such as `assume`, `subscribe`, `set_arm`, `key`, `audience_for` and `presence_for`. Nothing outside it touches the underlying structures.

This costs nothing today because the module is wanted regardless: being the **single writer** is what makes [ADR-0019](./0019-presence-is-one-versioned-document-scoped-to-reach.md)'s monotonic per-session versioning and atomically-rendered documents possible at all. A projection assembled by several writers is exactly the torn state that ADR-0019 rejects.

**It is deliberately not modelled as a key-value store.** The presence document is not stored — ADR-0019 makes audience *computed, not stored* — it is a projection over subscriptions, mutes, connection states and beacon health. Flattening a projection into records today, in order to make a later key-value backend easier, would pay the whole cost immediately for an option that may never be exercised.

## Redis is recorded as hygiene, not as a migration path

The distinction matters, because the alternative is that someone later "completes the migration" and ships a system that is slower, has more to deploy air-gapped, and is no more capable. Neither candidate motivation survives inspection:

- **Surviving a restart** is unavailable for the reason above — the transports are gone, so there is nothing truthful to restore.
- **Running two server processes** is foreclosed by the media plane rather than by the state store. Sessions on process A hold transports on A's Worker, and B cannot fan out to them without `PipeTransport`. Shared state would be necessary and nowhere near sufficient; ADR-0006's growth path is sharding across Routers, which is a placement change, not a state-store change.

The boundary is therefore justified by correctness today, and a swappable backing store is a side effect of having drawn it properly.

## Consequences

- **The state authority is the single writer for everything live**, which is what lets presence versions be monotonic and documents atomic. Any code path that mutates live state elsewhere breaks ADR-0019 rather than merely bending this ADR.
- **Blast radius is a query on this module**, answered as a value and handed to [ADR-0038](./0038-sqlite-behind-domain-shaped-repositories.md)'s transaction before it opens. That is the only place the two seams meet, and they meet by passing data rather than by sharing a boundary.
- **A restart is an announced operational act.** Because it is indistinguishable from a total network loss to every client, upgrading the binary during an event is a decision with a cost, and the deployment work should say so.
- **A latched emission dies with the process**, and per ADR-0018 the client is the only thing that can tell its own operator. The local, server-independent announcement path that ADR-0018 already requires covers a restart with nothing added.
- **Nothing may assume live state is durable.** Any later feature that needs a fact to survive a restart must put it in [ADR-0038](./0038-sqlite-behind-domain-shaped-repositories.md)'s store deliberately, which is the correct place to have that argument.

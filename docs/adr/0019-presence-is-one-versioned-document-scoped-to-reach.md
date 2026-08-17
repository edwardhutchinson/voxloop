# Presence is one versioned document, scoped to reach, hiding nothing inside it

Three decisions about how presence state is carried, sliced and shared.

## One channel, and the signalling WebSocket is authoritative

All state rides the signalling WebSocket VoxLoop already owns for mediasoup. The media transport carries audio and nothing else.

They can disagree, in both directions. The rule is that **the WebSocket is authoritative for state; the media transport is authoritative only for connectivity health**, which it reports *into* the WebSocket from both ends — the server sees ICE and DTLS state, the client sees it from the other side. Health merges pessimistically: **a green reading needs both ends to agree, a red one needs only one**.

Where the WebSocket itself is down, no displayed state is trustworthy however healthy the audio sounds, and the session enters the degraded ladder in [ADR-0018](./0018-no-signalling-channel-means-no-emission-path.md).

## One versioned document, rendered atomically

Presence is pushed as a **single versioned document per session**, not as independent per-topic streams.

Independent streams were the obvious shape and they carry a subtle defect: arms as of one instant beside subscriptions as of another, each individually server-confirmed, the combination never true at any moment. That satisfies [ADR-0016](./0016-displayed-state-is-observed-or-asserted.md) to the letter while displaying something that was never the case — a torn state, which is a lie assembled entirely from facts.

A versioned document costs a little bandwidth and buys the property this whole area exists for: **what is on screen was simultaneously true.** It also gives [ADR-0018](./0018-no-signalling-channel-means-no-emission-path.md)'s stale marking a single honest age to display rather than one per widget, and it collapses resynchronisation to *"here is version N"* — which is why [#18](https://github.com/edwardhutchinson/voxloop/issues/18) gets materially easier rather than harder.

## Scoped to reach

A session receives presence only for loops its role holds at least `monitor` on.

Pushing everything was simpler and was rejected on boundaries rather than bandwidth. [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) makes the grid the single gate on reach; leaking the staffing state and subscriber lists of loops a role cannot touch erects a second, softer information boundary beside it that nobody configured and no administrator can see. One gate or none.

Note that `Observer`, seeded `monitor` on every loop present at install ([ADR-0015](./0015-the-admin-console-reads-one-row-at-a-time.md)), still sees nearly everything. That is correct — it is what the grid says — and it is worth noticing rather than fixing.

## Inside your reach, nothing is hidden

**"Who is currently listening to me" is fully visible, per (role, user), to anyone holding `monitor` on the loop** — and it distinguishes three states, not two: **hearing**, **present but not hearing** (muted, off console, unreachable, or not receiving the beacon), and **not subscribed**.

The three-way split is the whole point. A subscriber list that silently includes people who cannot hear you is precisely the misrepresentation this area exists to eliminate — it answers *who chose to listen* when the operator asked *who will hear me*. The corollary is that **before keying, the console shows the audience for the current arm set**, which is the brief's "who am I about to talk to" answered directly rather than inferred from a list.

This makes one person's mute visible to their colleagues, which is socially spicy and was accepted deliberately. It is mitigated by presentation, not by concealment: lead with *not hearing*, keep the reason available. Concealing it would mean the console knows something material about whether you will be heard and declines to say so. Privacy of this kind is not on offer in a single-organisation operations centre whose defining requirement is certainty about who hears you.

## Consequences

- **The document is the API.** Whatever the console needs to render must be in it, and anything in it is something the server has committed to keeping true — this is the surface [#12](https://github.com/edwardhutchinson/voxloop/issues/12) designs against.
- **Version numbers must be monotonic per session** and survive reconnection, or resynchronisation cannot tell a stale document from a fresh one.
- **The scoping is recomputed when the grid changes.** A cell edit that grants or revokes `monitor` changes what a live session may see, and the document must narrow or widen accordingly — mid-session, without a re-sign-in.
- **The blast-radius query [ADR-0015](./0015-the-admin-console-reads-one-row-at-a-time.md) depends on is satisfied by this state.** Live sessions, their subscriptions and their arms are all server-held and point-in-time queryable, because they are exactly what the documents are projected from.
- **Audience is computed, not stored.** It is a projection over subscriptions, mutes, connection states and beacon health, and it changes whenever any of those do — so it is the most volatile thing in the document and the most likely to need rate-limiting in practice.

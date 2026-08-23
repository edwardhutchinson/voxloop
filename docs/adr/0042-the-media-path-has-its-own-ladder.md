# The media path has its own ladder, and it withdraws emission too

[ADR-0018](./0018-no-signalling-channel-means-no-emission-path.md) rules the case where the audio path is fine and nobody can tell the client anything. The mirror case — **signalling fine, audio dead** — was unruled, and `CONTEXT.md` closes the door on handling it under the same name: *connection state* describes the state channel, never the audio path.

That gap is not academic. A session whose transport is down has a console showing a healthy, armed, keyable operator whose voice reaches nobody, and whose transmitting lamp never lights because [ADR-0008](./0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md) sources it from audio the server never receives. It is the same misrepresentation ADR-0018 exists to prevent, arriving through the other door.

**Therefore a session with no media path has no emission path**, expressed as a second three-valued ladder: **media path state**, one of `connected`, `impaired`, `lost`. PTT is withdrawn at `lost`.

Two asymmetries against ADR-0018 are worth having. This failure **is announceable**, because the signalling channel is up — so the operator is told exactly what is wrong, and per ADR-0008 the loop is told a transmission was *cut* rather than hearing a voice vanish mid-word. And the operator is present and reading a working console throughout, which is what licenses the posture in the last section.

## The client drives it; the server is the backstop

The two ends of the media path do not have the same shape, and the difference decides who drives.

The browser's `RTCPeerConnection` distinguishes a transient `disconnected`, which routinely self-heals in a second or two, from a terminal `failed`. mediasoup's server-side `iceState` has **no `failed` at all**: it goes `connected`/`completed` → `disconnected`, documented as *"was connected or completed but it has suddenly failed"*, and it is driven by ICE consent freshness — which takes on the order of **thirty seconds** to declare loss.

A server-authoritative ladder would therefore keep PTT live over a dead audio path for longer than ADR-0018's entire signalling ladder allows. So the **client drives it**: its `disconnected` maps to `impaired`, its `failed` to `lost`, reported into the WebSocket. mediasoup's `on_ice_state_change` and `on_dtls_state_change` are the **backstop** — the server's `iceState: disconnected` also forces `lost`, covering the wedged or lying client, which is the one direction the server is genuinely better at.

This is [ADR-0019](./0019-presence-is-one-versioned-document-scoped-to-reach.md)'s existing rule applied rather than a new one: health is reported from both ends into the WebSocket and merges pessimistically — green needs both ends to agree, red needs only one.

The `impaired` band exists for the same reason ADR-0018's `unconfirmed` band does. A binary reading would flap on every ICE consent hiccup and cut audio for a reroute that heals itself.

## Recovery order is fixed by the API, not chosen

mediasoup's `restart_ice()` returns fresh `IceParameters` **that must be signalled to the client**. So an ICE restart is impossible without a working signalling channel, and recovery is strictly ordered:

1. Signalling channel back to `confirmed`.
2. `restart_ice()` and re-signal — this keeps the transport, the producer and every consumer alive, and is by far the cheap path.
3. Full transport rebuild, only if that fails.

A media-only failure — WebSocket healthy throughout — skips straight to step 2 with nothing to wait for. A session that lost both recovers signalling first by necessity, which is also the right order: it restores the channel that can *tell the operator what is happening*.

## What a rebuild rebuilds, and what re-enables PTT

A rebuilt transport needs the session's producer ([ADR-0007](./0007-the-client-emits-one-stream.md): one uplink stream), a beacon consumer per subscribed loop ([ADR-0017](./0017-loop-health-is-measured-not-asserted.md)), and consumers for whoever is talking.

It is smaller than it looks. A per-talker consumer only exists while that talker is producing, so at any moment there are a handful, not one per subscriber — the rebuild is one producer, N beacons, and a short tail.

Order is **producer, then beacons, then talker consumers**, and **PTT waits only on the producer**. The full predicate for re-enabling emission, satisfying ADR-0018's requirement that reconnection restore emission capability explicitly rather than as a side effect, is:

> the presence document has arrived **and** the producer is created and server-confirmed.

Beacons are deliberately excluded. Waiting on twenty loops' health to recover before the loudest voice in the room can speak buys no safety — the operator's own uplink is what PTT gates, and ADR-0017 already renders loop health honestly while it is unknown.

## A permanently dead media path does not end the session

If the WebSocket stays confirmed while the media path stays `lost` — ICE restart failing, rebuild failing — nothing reaps that session. [ADR-0041](./0041-a-session-is-resumed-by-name.md)'s reconnection window is a *signalling* timer. The operator holds their role, hears nothing, cannot key, and every loop they staff reads `away`, indefinitely.

**That is accepted; no media-side window is added.** This is the one failure where the operator is present and reading a working console, so the server can tell them precisely what is wrong. Ending their session for them takes the decision from the person best placed to make it, possibly mid-fix. The occupancy cost is already covered: colleagues see the role held by someone unreachable with a running age, and ADR-0023's forced relinquish exists for when it matters.

It is also the posture this project has taken twice before — ADR-0016 makes an ambiguity visible and leaves the judgement with the human rather than resolving it by guessing.

## Consequences

- **`media path state` is a third axis on the console**, alongside connection state and per-loop health. All three can disagree, and each says something the others do not.
- **Emission has two independent withdrawal conditions now.** The transmit bar must say *which*, because "you cannot talk" for a lost state channel and for a lost audio path are different problems with different fixes.
- **A cut for media loss is announced to the loop**, unlike ADR-0018's silent case, so the protocol's cut reason set gains one value.
- **The `impaired` threshold is a client-side judgement about a browser state**, so it cannot be tuned server-side with ADR-0041's timers, and it will behave differently across browsers. ADR-0020 made the browser tier genuinely universal; this is where that costs something.
- **A page reload is now a routine transport rebuild** (ADR-0041 makes F5 a resume), so the rebuild path is exercised constantly rather than only in failures. That is a gift for reliability and should be kept that way.

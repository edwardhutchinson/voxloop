# The client emits one stream; the server routes it to loops

Audio moves in three layers, and only the middle one knows what a loop is.

- **Uplink** — a talker's client encodes once and sends **one** stream, whatever they are armed on. It behaves like a radio wave: the client transmits, it does not address.
- **Server** — VoxLoop fans that stream into each loop the talker is armed and keyed on. This is the only place loop identity exists in the media path.
- **Downlink** — each subscriber receives **one stream per audible talker**, mixed in their client at per-loop volume.

**A loop is therefore not a transport primitive at all.** It is a row in the (role, loop) permission matrix from [ADR-0002](./0002-permissions-attach-to-role-and-loop.md) and an entry in the server's routing table. Nothing in the media layer knows what a loop is; mediasoup Routers are a sharding unit only.

**Router-per-loop is rejected, and the reason is worth recording because it looks like an oversight.** A mediasoup `Transport` belongs to exactly one `Router`, so a user monitoring six loops would need six WebRtcTransports — six ICE and DTLS sessions per user. That is precisely the connection explosion that disqualified rooms-as-loops on LiveKit ([ADR-0006](./0006-mediasoup-carries-the-audio.md)), arriving through a different door.

**Publishing one stream per destination loop is rejected too**, though it was the first proposal. It makes the client encode N times, multiplies uplink bandwidth by N for exactly the remote VPN users who have least of it, and — worst — turns arming a loop into a renegotiation. Under server-side routing, arming and disarming are pure routing changes: the Producer already exists, so the change is instant and the client never negotiates.

**Server-side mixing is rejected**, which is the deliberate divergence from openvocs. Mixing per subscriber satisfies per-loop volume, which the [prior-art survey](https://github.com/edwardhutchinson/voxloop/issues/4) found is the universal operator mechanism, and openvocs has flown it for a decade. But it forecloses client-side ducking and interruptive priority permanently, forecloses per-stream processing that the map requires not be made expensive, and [#15](https://github.com/edwardhutchinson/voxloop/issues/15) found openvocs pays one OS process per connected user for it.

**The downlink is per talker, not per (talker, loop).** The finer split was proposed first and has a case: each stream would carry its loop context inherently, so media and displayed attribution could never disagree. It fails on the overlap case, which is exactly what multi-destination emission exists to produce. If Alice emits to Flight and Sim and Bob monitors both, per-(talker, loop) delivers Alice's voice to Bob **twice** — the same audio, two streams, two volumes — so Bob hears her doubled unless the client discards one, at which point the second stream was pure waste. It also doubles consumer count in precisely the situation the feature is for.

Giving that up costs per-loop volume its unambiguity, which is resolved by rule: **when a talker is emitting to several loops a subscriber monitors, the loudest applicable loop volume wins.** Loop volume is an attenuation control — an operator turns a loop down because it is chatter they do not need moment to moment — so if a transmission is also going to a loop they have kept up, they have already signalled they want to hear it. Quietest-wins would let a suppressed loop silence a transmission the operator cares about, which is the wrong failure direction in an operations centre. A designated "primary loop" would need a concept of primacy that neither the domain model nor any surveyed system has.

What this does **not** give up is per-loop separation in the architecture. That survives on the server-side fan-out and is what the map's recording constraint actually binds ([ADR-0009](./0009-recording-taps-plain-rtp-on-loopback.md)). [#15](https://github.com/edwardhutchinson/voxloop/issues/15) drew this distinction precisely: *does the client receive separate streams* and *does the architecture preserve them* are different questions, and only the second is load-bearing.

## Consequences

- **Arming and disarming a loop are instant and require no renegotiation.** This is the main practical win of server-side routing and it should not be traded away later without re-opening this ADR.
- **Loop attribution reaches the client through signalling, not through media**, so there is a window in which audio and displayed attribution can disagree. The console must render both from the same server-pushed state rather than racing a media event against a signalling event — the map's requirement that state shown always be factual lands here.
- **The console must attribute a transmission to every loop it is on**, not just the one that won the volume. Otherwise an operator sees Alice on Flight at full volume with no indication that Sim — which they had turned down — is what she is actually addressing.
- **Panning is available to the console for free.** With client-side mixing over mono sources ([ADR-0010](./0010-opus-mono-and-the-latency-budget.md)), placing loops in the stereo field costs nothing on the wire.
- **Consumer count scales with (subscriber × audible talker), not with loops.** Roughly 40 producers and 3,000–4,000 consumers at the pilot's shape; see [ADR-0006](./0006-mediasoup-carries-the-audio.md) for why that is affordable and what must be load-tested.
- **Ducking and interruptive priority are now possible but not yet decided.** The separate-streams downlink was justified partly on keeping them available; whether v1 has them, and what triggers them, is still open.

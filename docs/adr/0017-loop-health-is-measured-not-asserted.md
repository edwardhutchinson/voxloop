# Loop health is measured by a beacon, not asserted by the server

[ADR-0010](./0010-opus-mono-and-the-latency-budget.md) named the trap and did not close it: with DTX on, silence sends no packets, so a client cannot tell *"this loop is quiet"* from *"I am deaf to this loop"* by watching media arrive — and the two are indistinguishable to the ear as well. It ruled that loop health must come from server-pushed application state rather than media presence. That rules out the wrong answer without supplying a right one.

The available answer was **assertion**: the server states that your consumers for this loop exist and are unpaused, and the session transport is up. That is cheap and it is nearly always green, because it reports the server's *belief* about a media path rather than the path itself. VoxLoop instead measures it, by two mechanisms with different reach.

## The loop beacon

Each loop carries one synthetic low-rate producer — a **loop beacon** — emitting a packet every few seconds, consumed by every session subscribed to that loop and counted by the client. Loop health is then the arrival of those packets: an end-to-end measurement over the same transport, the same router and the same per-loop fan-out that real speech traverses.

It is affordable. At pilot scale — around 200 sessions subscribed to roughly six loops each — it is of the order of 240 packets per second in total, against the 15,000–30,000 that live speech produces ([ADR-0006](./0006-mediasoup-carries-the-audio.md)). It is noise.

**The beacon is silent in v1.** A faintly audible beacon — a barely-there noise floor announcing a live circuit, the way analogue radio has always done it — is genuinely attractive against the map's cognitive-load requirement, because it is perceived without looking at anything. It is also a way to make twenty loops hiss at an operator whose product aesthetic is *minimal and muted*. The mechanism is identical either way, so nothing is foreclosed: this is a rendering choice over the same packets, and [#12](https://github.com/edwardhutchinson/voxloop/issues/12) is where a screen can answer it better than an argument can.

## The client checks its own output path

The beacon proves audio reached the browser. It says nothing about whether it reached the operator's ears, and the most probable real-world *"I can't hear Flight"* is not a routing fault at all — it is a headset unplugged, or Windows silently repointing the default output device. Both are invisible to the server and invisible to the beacon.

So the client also watches its own output: a confirmed check tone at sign-in, `devicechange` monitoring thereafter, and detection of the default-device swap by comparing the label and group of the `default` entry across change events. Any change is surfaced loudly. *(The default-swap detection is asserted here at moderate confidence and should be confirmed on real hardware alongside [#17](https://github.com/edwardhutchinson/voxloop/issues/17).)*

## Beacon loss drops the loop to `away`

[ADR-0005](./0005-occupancy-means-listening-not-signed-in.md) holds that a loop nobody can hear is not staffed. Beacon loss is a fourth way of not hearing, alongside off console, muted and unreachable, so it counts the same way: a loop whose staffing-role occupants are all failing to receive it reads `away`, with beacon loss as the shown reason.

This is the real payoff. It upgrades staffing state from *"someone says they are listening"* to *"someone is demonstrably receiving"* — which is as close to factual as this can be made, and it is the direct answer to the brief's question of whether anyone is actually behind a loop before you key up.

It also fails safe in the wedged-client case: a client that has stopped functioning reports no beacons, the server sees no reports, and the loop reads `away` rather than `staffed`.

## What the beacon does not prove

The downlink is per talker ([ADR-0007](./0007-the-client-emits-one-stream.md)), so the beacon's consumer is a **different consumer** from the one carrying any given talker's voice. Beacon *loss* soundly proves the session is deaf to the loop, which is the direction the `away` rule leans on. The converse does not hold: **a beacon arriving does not prove you would hear Alice**, because her individual consumer could be broken while the loop's path is fine.

A beacon per (loop, talker) would close this and was rejected — forty times the mechanism, and it re-inflates precisely the idle-stream cost DTX exists to avoid. The claim the beacon supports is therefore *"the loop reaches this session"*, never *"this session will hear every talker"*.

**This limitation is recorded rather than closed**, because the fault it leaves behind is one where the console shows green and an operator hears nothing — the single failure this product can least afford, and the last thing that should be discovered rather than known.

## Consequences

- **The beacon must be excluded from the recording tap** ([ADR-0009](./0009-recording-taps-plain-rtp-on-loopback.md)) **and from `AudioLevelObserver`** ([ADR-0008](./0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md)), or it registers as a permanent talker on every loop and as a permanent signal in every recording.
- **Loop health is per (session, loop)**, not a property of the loop. Two sessions subscribed to the same loop can legitimately disagree, and both readings are correct.
- **A loop with no subscribers still runs its beacon.** Suppressing it would make the mechanism unavailable at exactly the moment someone subscribes.
- **The beacon covers server-side fan-out faults for free**, since it originates server-side per loop: if the loop's fan-out is broken, no subscriber sees the beacon.
- **A load test must include beacon traffic**, since it is the one packet source that is always on and therefore the floor beneath the idle-consumer assumption ADR-0006 leaves unverified.

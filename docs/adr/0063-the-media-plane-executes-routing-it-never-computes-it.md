# The media plane executes routing; it never computes it

The **Media plane** interface takes *these subscribers should hear this producer* and makes it so. It never decides who. **State authority** computes the audience, which [ADR-0039](./0039-live-state-is-in-process-behind-one-state-authority.md) already gives it an operation for, and hands the answer down.

[ADR-0007](./0007-the-client-emits-one-stream.md) said nothing in the media layer knows what a loop is, and left it as a description of mediasoup rather than a constraint on VoxLoop's own code. This makes it a type signature. No subscription, arm set or permission rung crosses into Media plane. A loop identifier may cross as an opaque label, since [ADR-0009](./0009-recording-taps-plain-rtp-on-loopback.md)'s tap is addressed per (talker, loop), but Media plane never reasons about it.

**Getting this the other way round would be quietly fatal.** [ADR-0064](./0064-tests-run-against-the-real-store.md) gives Media plane a fake, because the real adapter is a C++ subprocess negotiating ICE and DTLS. If the fan-out decision lived below the seam, that fake would have to reimplement VoxLoop's routing, and every test touching arming, subscription, reach or Cut would be testing the fake instead of the product. Routing is where an open microphone comes from, and [ADR-0016](./0016-displayed-state-is-observed-or-asserted.md) has already made *the console displayed something that was never true* the defining defect of this product. Testing that against a reimplementation is worse than not testing it, because it looks green.

Placed below the decision, the fake is a recorder. A test arms Alice on Flight and Sim, subscribes Bob to Flight, and asserts on the instructions Media plane was handed. No worker runs.

## Consequences

- **Every routing rule in VoxLoop is testable with no mediasoup running.** Arming and keying, loudest-wins overlap, Cut closing the fan-out, priority applying to the whole arm set, the recording tap attaching per (talker, loop). All of it becomes an assertion about instructions.
- **`audience_for` is load-bearing rather than a convenience.** ADR-0039 listed it as one operation among several. It is now the single computation Media plane depends on and the only place reach is decided.
- **Cut is a State authority act.** [ADR-0014](./0014-authority-acts-on-emission-are-transient.md) describes it as closing the server-side fan-out, which reads like a Media plane operation. It is not: the state authority withdraws the audience and the media plane executes the result, exactly like any other change of reach.
- **The recording tap sits below the seam with everything else**, so attaching a sink stays what ADR-0009 promised, no change to the media path and no load on the media server.
- **A `mediasoup::Producer` never leaves the module.** Whatever identifies a stream across the seam is VoxLoop's own value, per [ADR-0060](./0060-a-seam-names-domain-operations.md), or the fake cannot exist.

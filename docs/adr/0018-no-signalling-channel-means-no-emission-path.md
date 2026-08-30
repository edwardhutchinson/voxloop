# A session with no signalling channel has no emission path

State travels on the signalling WebSocket; audio travels on the media transport ([ADR-0019](./0019-presence-is-one-versioned-document-scoped-to-reach.md), [ADR-0006](./0006-mediasoup-carries-the-audio.md)). They fail independently, which produces the case this ADR exists for: **the client's audio path is fine and nobody can tell it anything.**

Under [ADR-0008](./0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md) every talking indicator anyone sees is a server broadcast. So an operator keying with no signalling channel transmits into a system where no listener's console will show them talking, no loop will attribute the transmission, and no operational authority holder can cut it. The audio arrives; the accountability does not.

**Therefore: a session with no signalling channel has no emission path.** The client disables push-to-talk, and the server closes the fan-out.

## Three connection states, not two

A single threshold would mean a VPN reroute mutes the Flight Director mid-sentence — trading a state-honesty problem for a new and worse availability problem. So a session's **connection state** has three values:

| | | |
|---|---|---|
| `confirmed` | heartbeats current | everything normal |
| `unconfirmed` | heartbeats missed | state frozen and marked stale, **PTT still live** |
| `disconnected` | past the threshold | PTT disabled, server closes fan-out, banner |

The `unconfirmed` band is what makes disabling PTT safe rather than fragile. Its honest reading is *"we cannot confirm your transmission right now"* — which is true, and is a materially different statement from *"we know you are disconnected"*.

Indicatively: `unconfirmed` after a few seconds, `disconnected` around ten. **The shape is settled here; the numbers belong to [#18](https://github.com/edwardhutchinson/voxloop/issues/18)**, tuned against the pilot's actual VPN. *(Set by [ADR-0041](./0041-a-session-is-resumed-by-name.md): 5s and 12s, startup settings with a bounded ceiling.)*

## In-flight emission: momentary survives, latched does not

An emission already open when the channel drops is a separate question from whether a new one may start.

**A momentary key survives to release**, bounded by the operator's thumb — or to the `disconnected` threshold, whichever comes first. A held button is a human continuously asserting intent, and cutting them off mid-word for a 500 ms blip is exactly the failure the `unconfirmed` band exists to prevent.

**A latched emission is dropped after roughly two seconds of `unconfirmed`.** A latch is an assertion made once, possibly minutes ago, and its entire safety story is *the console will show you that it is open* ([ADR-0013](./0013-arming-is-independent-of-subscription.md) refuses latched presets for the same reason). That story is void the instant the console cannot be trusted, so a latched transmission surviving a signalling drop is a hot mic that by definition nobody can be told about. The two-second grace keeps a brief blip from cutting a genuine latched transmission; past it, the latch dies.

This is the one place VoxLoop cuts audio without being able to announce it, which contradicts [ADR-0008](./0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md)'s rule that a cut transmission is announced to the loop. It is accepted because the alternative is an unannounced *open* mic, and because the announcement that can still be made is made: **the client tells its own operator locally**, which is the one message that cannot come from the server.

## The server enforces it too

Disabling PTT in the client is not sufficient. The situation that motivates the rule — the client has lost the server — is precisely the situation where the client may be wedged, and a wedged client's audio is still arriving at a perfectly healthy media transport.

So **the server closes the fan-out at the same threshold**, in the media plane, reusing the machinery ADR-0008 already built for revocation and [ADR-0014](./0014-authority-acts-on-emission-are-transient.md) for Cut. Without this, the rule would be a client-side courtesy and ADR-0008's stated residual — a client that keeps sending while claiming otherwise — would return in its worst form: audio reaching loops from a session nobody can attribute, indicate or cut.

## What the console shows while it is uncertain

Last-known state is **frozen and visibly marked stale with a running age**, under a banner announcing the loss. It is not blanked and the UI is not blocked.

Blanking was rejected as its own lie: an empty console implies *nothing is happening*, when in fact everything may be happening and we simply cannot see it. Blocking the UI was rejected because it disarms an operator at the exact moment things are going wrong. A frozen state that says *"last confirmed 12s ago"* is the only rendering that is true, and the running age is what stops it being mistaken for live.

## Consequences

- **The disconnect threshold is a safety parameter, not a display preference.** It sets how long a network hiccup can silence the loudest voice in the room, and it must be tuned on the pilot's network before v1 is called done.
- **Reconnection must restore emission capability explicitly**, not as a side effect: PTT is re-enabled only after the signalling channel is back *and* state has resynchronised. [#18](https://github.com/edwardhutchinson/voxloop/issues/18) owns what is restored and what is deliberately not. *(Answered by [ADR-0043](./0043-a-resume-restores-everything-except-the-key.md); the predicate is [ADR-0042](./0042-the-media-path-has-its-own-ladder.md)'s, which adds a second withdrawal condition this ADR did not anticipate.)*
- **The client needs a local, server-independent announcement path** for the dropped latch and for the disabled PTT. This is the only user-facing message in the product that does not originate at the server.
- **A latched emission is now interruptible by the network**, so the console must make a dropped latch unmissable — an operator who believes they are still transmitting is the failure this rule was meant to remove, arriving through the other door.
- **Two sessions can be cut for reasons the other cannot see.** A session dropped at the `disconnected` threshold vanishes from loops it was emitting to; per ADR-0008 the loop must be told a transmission was cut rather than ended, and the reason available to listeners is *signalling lost*, not *talker released*.

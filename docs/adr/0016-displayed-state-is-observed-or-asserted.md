# Displayed state is observed or asserted, and never guessed

The originating brief's sharpest non-functional requirement is that *"we don't really have much room for the UI presenting something that's not factual"*. That is not satisfiable by care alone — it needs a rule that makes "factual" checkable. This ADR is that rule, in three parts.

## Every state item is observed or asserted

**Observed state** is something the server has seen: a session's transport is connected, a producer is sending audio, a consumer exists, a subscription is held, a loop is armed. **Asserted state** is something a user has claimed: *I am off console*. VoxLoop cannot see a chair, so an asserted state is only ever as true as the moment it was asserted.

Both classifications are carried on the state itself, and **the console is forbidden from rendering them identically**. The classification is not a judgement call — it falls out of where the state came from — which is what makes the rule enforceable rather than aspirational.

## The console renders only what the server has confirmed

Generalising [ADR-0008](./0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md), which lit the transmitting lamp from the server's acknowledgement rather than from the operator's own button: **a local action may change local audio, never the displayed state of the world.** Clicking *subscribe* sends a request; nothing on screen moves until the server says it happened.

Optimistic rendering was rejected outright, and it is worth being explicit about why, because it is the near-universal UI convention and someone will reintroduce it for feel. Optimistic UI *is* the mechanism by which a console shows you subscribed to a loop you are not subscribed to. The cost is that every toggle visibly lags a round trip — a few milliseconds on the LAN, tens over the VPN — which is the cheapest honesty available in this product.

Keying remains the one special case, and it is not an exception to this rule but an instance of it: audio starts locally for latency, and the *lamp* still waits for the server.

## VoxLoop never guesses whether a human is in the chair

Off console is asserted and **never inferred**. Idle-based demotion — flipping a session to off console after N minutes without keyboard or mouse input — was considered and rejected: an operator watching telemetry intently is idle at the keyboard and very much on console, so auto-away is a fabrication in the opposite direction from the one it was meant to fix.

Instead, the console shows **when the assertion was last backed by evidence**: *"on console, last active 14 min ago"*. That is a fact; `away` would not have been. What counts as evidence is a deliberate act — keying, changing a subscription, changing an arm, answering a prompt — never mouse movement or scroll. It is free, because every deliberate act already arrives at the server as a signalling message.

## Off console is an assertion about the human, not about the audio

Declaring off console drops the staffing state of loops you staff to `away` ([ADR-0005](./0005-occupancy-means-listening-not-signed-in.md)) and changes nothing else. Subscriptions stand and audio keeps flowing, so the operator who steps away and hears something over their headset from three metres away still hears it, and returning is a click rather than a resynchronisation.

**Keying clears the flag**, because keying is an unambiguous assertion of presence. **Nothing else does** — not mouse movement, not scrolling, not focus — because clearing on incidental activity would be exactly the guessing this ADR forbids.

## Consequences

- **The console has no local model of the world it can diverge from.** It is a projection of server state; see [ADR-0019](./0019-presence-is-one-versioned-document-scoped-to-reach.md) for the shape that state arrives in.
- **"Last active" is an observed state item that must be recorded per session**, distinguishing deliberate acts from traffic. It is displayed against asserted state, which is the only state that needs it.
- **Anyone proposing optimistic rendering later is reopening this ADR**, not making a UI tweak.
- **A stale assertion is still shown, with its age.** The system does not resolve the ambiguity of an operator who walked away without saying so — it makes the ambiguity visible and leaves the judgement with the human. This is a deliberate refusal to be helpful.

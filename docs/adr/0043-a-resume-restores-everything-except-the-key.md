# A resume restores everything except the key

[ADR-0041](./0041-a-session-is-resumed-by-name.md) reattaches a client to a session that never ended. This ADR says what the operator gets back.

## Everything server-held returns, unconditionally

Occupancy, subscriptions, mutes, arms and the off-console assertion **all return**, with no reconfirmation and no ceremony. They are live state on a session [ADR-0023](./0023-sign-in-is-to-the-application-and-a-role-is-assumed.md) says survived, so "restoring" them is not an act at all — it is [ADR-0019](./0019-presence-is-one-versioned-document-scoped-to-reach.md)'s document being rendered again. The client has no local model to reconcile ([ADR-0016](./0016-displayed-state-is-observed-or-asserted.md)); it is a projection, and the projection resumes.

**A graduated restore was designed and dropped.** The idea was that a long outage should force the operator to re-confirm their arms before keying — silent restore for a blip, an explicit act after ninety seconds. It fails on inspection: **an arm can only change by the operator's own hand or by a permission revocation**, and a revocation removes the arm server-side anyway ([ADR-0008](./0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md) closes the fan-out mid-word). So a restored arm set is *exactly what that operator chose*, minus anything they have since lost. There is nothing for them to discover, and a re-arm gate would be friction in the precise moment things are going wrong — which is when outages happen.

The compensation is already built and always on: [ADR-0034](./0034-the-transmit-bar-is-always-visible-and-the-audience-is-a-count.md) keeps the transmit bar unscrolled with the armed set in words and the audience as counts, so the operator sees their destinations before they key, resume or no resume.

A **monitoring directive issued during the gap** needs no special handling and gets none. [ADR-0035](./0035-a-monitoring-directive-promotes-a-loop-it-does-not-police-it.md) applies a directive once per session; the session survived, so it applied server-side while the client was dark, and it simply appears on resume as an ordinary directed subscription — carrying its marking and its reason, as it always would.

## Key state never returns

**A resumed session is always unkeyed.**

A latched emission is already gone — ADR-0018 drops it after two seconds of `unconfirmed`, because a latch's entire safety story is that the console will show it to you, and that story is void the moment the console cannot be trusted. A momentary key cannot survive either, for a different reason: it is a human continuously asserting intent with their thumb, and the thumb is not observable across a gap. Whatever the server last saw is a guess by the time the channel returns.

### A key held across the outage is suppressed until it is released

This is the sharp edge, and it falls out of [ADR-0021](./0021-ptt-input-is-a-level-with-liveness.md) rather than fighting it. PTT input is a **level**, not events — the client ORs the live sources — so an operator still holding the button when the channel returns presents a HIGH level, and audio would begin the instant [ADR-0042](./0042-the-media-path-has-its-own-ladder.md)'s predicate is satisfied.

That is a hot mic with a randomly-timed start. The operator was told locally that PTT was dead ([ADR-0018](./0018-no-signalling-channel-means-no-emission-path.md)'s server-independent announcement path), so they cannot know the moment it goes live, and they may well have stopped speaking a minute ago.

**So a source that was HIGH across a PTT withdrawal is marked stale and contributes nothing to the OR until it transitions LOW then HIGH again.** It is a suppression on the level, not a translation into edges, so ADR-0021's model is intact — and the same rule covers the withdrawal edge at `disconnected`, making it one rule for both ends of the outage rather than two.

## A resume is the network's act, not the human's

The off-console assertion **stands untouched** across a resume, and a reconnect **does not refresh last-active**.

ADR-0016 admits only deliberate acts as evidence that someone is in the chair, and keying is the only thing that clears off console. A socket re-establishing itself while the operator is in the kitchen must not make their console claim they came back — that is the guessing ADR-0016 forbids, dressed as plumbing.

## Consequences

- **The audio comes back on its own; the ability to talk does not.** Subscriptions restore and the operator hears the room again as soon as consumers rebuild, but emission waits on ADR-0042's predicate and then on their thumb. That asymmetry is deliberate and the console must not blur it.
- **"Stale-high" is a state an input source carries**, so ADR-0021's seam gains a third thing a source's reading can mean, alongside intent and liveness. It is set by the client, never reported by the source.
- **Nothing in the resume path writes live state**, which keeps [ADR-0039](./0039-live-state-is-in-process-behind-one-state-authority.md)'s state authority the single writer: a resume reattaches a socket and projects a document, and the only mutation is the connection state itself.
- **A long outage and a short one are indistinguishable in what they restore**, so the only thing carrying the outage's length to the operator is [ADR-0044](./0044-resynchronisation-narrates-the-gap.md)'s narration. If that is ever weakened, this decision should be revisited with it.

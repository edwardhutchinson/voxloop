# Arming is independent of subscription

A user may emit to a loop they are not monitoring, and monitor a loop they are not armed on. **You subscribe to the loops you want to hear, and nothing else puts a loop in your ears.**

The alternative — arming a loop forces you to subscribe to it — was considered and rejected because it destroys the meaning of a subscription. [ADR-0005](./0005-occupancy-means-listening-not-signed-in.md) reads occupancy directly off subscriptions: a loop is `staffed` when an occupant of a staffing role is subscribed to it. If arming pushed entries into that set, loops would report themselves staffed because somebody was *talking at* them, which is precisely the failure ADR-0005 exists to prevent. Keeping the two independent leaves a subscription meaning "I chose to listen to this" — the only meaning occupancy can be computed from.

**The cost is that emitting blind is now legal.** An operator can arm a loop they cannot hear and talk over a conversation in progress. This is accepted, and compensated in the console rather than in the model: every armed loop shows whether **someone is currently transmitting on it**, which is free — the server is already the sole authority for talking indicators and `AudioLevelObserver` already runs in v1 ([ADR-0008](./0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md)). Arms on loops the user does not monitor are rendered distinctly from arms on loops they do, because those are materially different situations and the standing requirement is that state shown be factual.

**Muting is personal and costs occupancy.** A user may mute a subscribed loop to concentrate. The subscription stands — loop health and talking indicators keep arriving — but a loop nobody can actually hear is not staffed, so muting a loop you staff drops it to `away`, exactly as declaring yourself off console does, and the drop is shown to you as you mute it. Refusing the mute outright was rejected: it would disable the feature precisely where concentration matters most. A mute does **not** auto-expire — an unexpected un-mute mid-incident is its own hazard, and [ADR-0004](./0004-monitoring-directives-are-enforced-and-additive.md) set the precedent — so it is paid for with visibility instead, both on the muter's console and in the `away` state everyone else can see.

## Presets

A **preset** is a named set of loops that momentarily replaces the user's armed loops while keyed, reverting on unkey. It exists to serve two symmetric needs at once: a senior figure reaching every loop for an announcement, and an operator with wide standing arms who wants to reach *only* the support engineers for one question.

- **Named "preset", not "broadcast".** The mechanism narrows as often as it widens, and "broadcast" names only half of it — a term that means "to everyone" would be wrong half the time and dangerous the other half.
- **Reach is the grid and only the grid.** A preset reaches loops the role already has `emit` on; "all loops" means "all loops I may emit on". A preset that could reach past the grid would be a hidden bypass, which is the exact thing [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) exists to prevent. A role that must reach everything in an emergency is given `emit` on everything, where the grid *shows* it. For the same reason there is no gate on who may use presets — the grid is already the gate.
- **Exclusive, not additive.** While a preset is keyed, its set *is* the destination. Additive would leave "where is my voice going" ambiguous for a narrowing preset, which is the case that matters most.
- **Momentary only.** Latched presets are refused outright. A latched preset across twenty loops that someone walks away from is a site-wide hot mic, and momentary-only makes the revert automatic rather than something a user can forget.
- **Role presets and personal presets both exist**, rendered distinctly — the same posture [ADR-0002](./0002-permissions-attach-to-role-and-loop.md) took. Role presets are system administration; personal presets are personalisation, scoped to (user, role).
- **A preset silently narrows to the loops its user may emit on**, showing the excluded ones greyed. Role presets are shared across occupants with differing reach, so rejecting them outright would make role presets unusable.

## Consequences

- **A preset is not a profile.** Profiles — persistent, deliberately switched, covering subscriptions, volumes and layout — remain open work. A preset is momentary, covers arms only, and reverts on unkey. Overlapping content, different lifetime; they must not be merged.
- **Recording is unaffected.** [ADR-0009](./0009-recording-taps-plain-rtp-on-loopback.md) taps per (talker, loop); a preset across twelve loops creates twelve taps, mechanically identical to arming twelve loops.
- **Whether a preset's emission ducks or interrupts what is already on those loops is not settled here** — it is permitted to talk over, and the priority question belongs to [#19](https://github.com/edwardhutchinson/voxloop/issues/19).

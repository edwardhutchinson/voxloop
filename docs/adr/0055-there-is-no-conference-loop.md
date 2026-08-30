# There is no conference loop

VoxLoop has no concept of a conference loop, a breakout loop or a private loop. There is no loop kind, no mechanism, and no naming convention in the v1 spec. A site that wants a room for two people to have a word configures an ordinary loop and uses it that way, and VoxLoop neither knows nor cares that they have.

## The question, and why it was asked

[ADR-0001](./0001-the-loop-is-the-only-destination.md) rejects channels created on demand and says private conversation "happens on standing, pre-provisioned conference loops that operators move to". That sentence names a *use*, and the open question was whether VoxLoop has to implement anything to support it. [#24](https://github.com/edwardhutchinson/voxloop/issues/24) went looking for the mechanism. There isn't one, and the parts that would have to be built are more expensive than the problem.

**Everything breakout needs is already here.** Peel-off is *"come to Conference 3"*, which is a [hail](./0047-a-hail-is-a-monitoring-directive-without-the-authority.md) to a role or a named person. Recall is the other operator hailing them back, and it does not need to reach into the breakout at all: it targets the loop you want them on, which the recaller already holds `emit` or `control` on. Quiet is [mute](./0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md) and per-loop volume, both live-only and both one click. Nothing new is required to move two people onto a loop and back.

## The bind that killed the designed version

A breakout loop is only reachable by hailing, and ADR-0047 is explicit that a hail to a seat which cannot already hear the destination does not land at all. So a breakout loop is useful in proportion to how many roles hold `monitor` on it, and private in inverse proportion. Make it broad enough to pull anyone in and you have made a room a large part of the ops floor can sit on silently, with [ADR-0033](./0033-the-console-shows-that-someone-is-talking-never-who.md) guaranteeing the two talkers cannot see who else is there.

**VoxLoop therefore cannot promise a conversation is private, and v1 does not pretend to.** Closing that gap needs either a per-user grant, which [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) removed from the model entirely, or an occupancy list naming who is present on a loop, which [ADR-0048](./0048-the-hail-picker-is-the-only-place-the-console-names-a-person.md) refuses. Both are large doors to open for an ergonomic gain.

## Two mechanisms considered and rejected

**Invite by hail** — a hail to a breakout loop grants the target reach on it for the duration. This would be the first thing in VoxLoop to confer reach outside the grid, against ADR-0047's "grants nothing" and ADR-0011's flat removal of per-user exceptions. It is arguable: hold it as live state in the [state authority](./0039-live-state-is-in-process-behind-one-state-authority.md) rather than as configuration, dead when the session is, and it becomes the mirror of [Cut](./0014-authority-acts-on-emission-are-transient.md), which ADR-0011 itself moved out of the permission model as a transient act. The argument works. It also buys a second, session-scoped authority path alongside the grid, and the grid being the only answer to "who can hear me" is the property ADR-0011 spent the whole ticket protecting.

**Isolate** — an opt-in control that mutes every loop except the one you are on, cleared when you leave it. Cheap, self-inflicted and instantly reversible, so it avoids the hazard [ADR-0004](./0004-monitoring-directives-are-enforced-and-additive.md) rejected the exclusive directive for. It dies on a different point: [ADR-0045](./0045-priority-defeats-attenuation-and-nothing-else.md) says priority defeats attenuation and does not defeat mute, so an isolated operator is unreachable by the only interruptive mechanism the product has. Two people isolated in a breakout can be reached by a hail banner and by Cut, and by nothing else. Making priority defeat isolate would make it defeat mute, which ADR-0045 refused, and a private conversation you can be shouted into is not private.

## Deferred on purpose, to be informed by use

Sites will improvise with ordinary loops. What they actually do — how many rooms, who is permitted on them, whether anyone minds being overheard, whether the quiet problem is real — is the input a genuine design needs and nobody currently has. Building the mechanism first would be guessing at all four.

## Consequences

- **ADR-0001 stands unamended.** Its "standing, pre-provisioned conference loops" names an administrator's use of an ordinary loop, not a feature. *"Move to Conference 3"* is a subscription plus a hail, and both exist today.
- **The v1 spec says nothing about conference loops**, including no recommended naming convention. Guidance would be the thin end of the feature.
- **Anyone whose role holds `monitor` hears the conversation, and the talkers cannot see who.** The only tell is the [transmit bar's](./0034-the-transmit-bar-is-always-visible-and-the-audience-is-a-count.md) audience count reading `4 hearing` when you expected one. That is a number you have to look at, not a guarantee, and operator documentation should say so plainly rather than let anyone assume otherwise.
- **Subscribing to a loop still does nothing to the loops you already have.** The busy loop you walked away from is still in your ears until you mute it or turn it down yourself. This is the same additive-only rule as ADR-0004 and the exclusive form stays rejected in every guise, including the voluntary one.
- **Cut reaches a conversation on a loop the cutter holds nothing on.** Cut applies to the whole uplink (ADR-0014) and the caller names an authority loop they hold `control` on ([ADR-0054](./0054-every-operation-declares-its-authorisation.md)), so a control holder on any loop they share with the target closes that target's mic everywhere. They cannot hear the conversation; they can end its audio. Correct, and worth knowing before someone discovers it.
- **A loop provisioned for this purpose has no staffing roles**, so it would have read `vacant` permanently under [ADR-0005](./0005-occupancy-means-listening-not-signed-in.md). [ADR-0056](./0056-a-loop-with-no-staffing-roles-has-no-staffing-state.md) fixes that, and is the one thing this question actually produced.
- **If breakout is ever built, invite-by-hail is the part to think hardest about**, because it is the part that breaks a settled rule rather than merely adding to the product.

# A role default is a starting point, never a floor

A **role default** is the console an administrator sets for a role: its **subscription set**, its **default view**, and its **loop order**. A `(user, role)` pair with nothing saved starts from it. It is **applied once and never re-imposed**, so editing it does not reach users who have already personalised. Their route back to it is the reset in [ADR-0049](./0049-the-role-is-the-profile.md).

Prior art is strong here. [#4](https://github.com/edwardhutchinson/voxloop/issues/4) found that openvocs and Rohde & Schwarz both load the loop set and the tile layout per role at sign-in, which is the same object under a different name.

**Volumes are deliberately excluded.** Volume is the one item scoped per `(user, role, loop)`, and it is personal in a way the others are not. An administrator setting `FLIGHT` to 80% for everyone occupying a role is guessing at headsets and hearing, and [ADR-0034](./0034-the-transmit-bar-is-always-visible-and-the-audience-is-a-count.md) already put volume behind a cog on the grounds that it is not a live operational control. Every loop starts at unity and the operator moves it.

## No enforced minimums

Subscriptions a role always gets that the user may not remove are **rejected**. [ADR-0035](./0035-a-monitoring-directive-promotes-a-loop-it-does-not-police-it.md) removed VoxLoop's only compulsion mechanism on a finding that generalises past monitoring directives: an operator who cannot drop a loop can still mute it, zero its volume, or walk away from the console, and the mechanism reports success in all three cases.

An enforced minimum buys the same appearance of a guarantee at the same price. It also buys it *worse*, because a directive at least sits behind a `control` gate and an enforced minimum would sit behind ordinary role configuration. The honest name for what an administrator wants here is "the role's default console", not "the loops this role must hear".

## No time-of-day defaults

Rohde & Schwarz supports roles that activate or phase out by time of day, and it is the closest thing prior art offers to admin-enforced-at-certain-times. It is out of v1 for two independent reasons.

[ADR-0031](./0031-v1-injects-but-does-not-schedule.md) already ruled that VoxLoop offers *do it now* and anything deciding *when* lives outside it, so a time-of-day default is that scheduler wearing a different hat. The second reason is the one worth leading with: **it changes an operator's console with nobody touching it**. Everything else that alters a console either announces itself or is the operator's own act. A console that quietly re-subscribes itself at 18:00 is a console that cannot explain why it looks the way it does, which is what [ADR-0016](./0016-displayed-state-is-observed-or-asserted.md) exists to prevent.

## Consequences

- **`Observer` ships subscribed to nothing, and the board is what makes that safe.** Every new user is eligible for `Observer`, which holds `monitor` on every loop present at install. Defaulting it to all of them delivers a wall of audio to whoever is least equipped for it. [ADR-0032](./0032-the-console-is-two-views-of-one-loop-list.md) renders one card per loop in reach, so an Observer with no subscriptions still sees every loop they could hear and picks. Silence is not a mystery when the whole board is visible. A site that wants new starters on `FLIGHT` edits the default once.
- **A role default edit is the first configuration change in VoxLoop with no live blast radius.** [ADR-0038](./0038-sqlite-behind-domain-shaped-repositories.md) made blast radius computation part of the grid edit transaction, crossing both seams in one operation, because a grid edit reshapes live sessions. A role default reaches no live session by construction. It is an ordinary write.
- **Audit splits along [ADR-0028](./0028-the-audit-log-records-decisions-not-traffic.md)'s existing line.** A role default edit is a configuration change by system administration, so it is audited. A user changing or resetting their own personalisation is not: it grants nothing and decides nothing about the system, the same reasoning that left hails unaudited in [ADR-0047](./0047-a-hail-is-a-monitoring-directive-without-the-authority.md).
- **Nothing in VoxLoop can still compel a subscription to be kept.** [ADR-0035](./0035-a-monitoring-directive-promotes-a-loop-it-does-not-police-it.md) recorded that gap and this decision declines to close it. If such a tool is ever genuinely needed it must be designed against the finding above, not assumed to be a configuration option.

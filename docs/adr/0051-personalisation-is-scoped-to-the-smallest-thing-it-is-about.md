# Personalisation is scoped to the smallest thing it is about, and the device belongs to the machine

> **Amends [ADR-0002](./0002-permissions-attach-to-role-and-loop.md)**, which scoped all personalisation to `(user, role, loop)`. Only volume is.

Three different scoping keys are already written into decisions with no stated rule behind them: [ADR-0002](./0002-permissions-attach-to-role-and-loop.md) says `(user, role, loop)`, [ADR-0013](./0013-arming-is-independent-of-subscription.md) says personal presets are `(user, role)`, and [ADR-0021](./0021-ptt-input-is-a-level-with-liveness.md) says bindings are per user. The rule that produces all three is that **each item is scoped to the smallest thing it is actually about**, and there are exactly four scopes:

- **Per user**: PTT bindings. Muscle memory follows the person, not the seat they are in.
- **Per `(user, role)`**: subscription set, loop order, default console view, personal presets.
- **Per `(user, role, loop)`**: volume.
- **Per machine**: audio input and output device.

## The device belongs to the desk

A headset belongs to a console, not to a person. An operator who sits at a spare desk and inherits the device identifiers from their own gets no microphone, and [ADR-0021](./0021-ptt-input-is-a-level-with-liveness.md)'s independent liveness signals mean the console will report a dead microphone accurately and uselessly, for a choice that operator never made.

**So device selection is held client side, in the browser, and never on the server.** VoxLoop cannot identify a machine. There is no machine concept anywhere in the model, and inventing one to hang a device identifier off is a large addition to the model for a small return. The browser already is the per-machine store.

The costs are real and accepted. Device choice is not backed up, not visible to an administrator, does not move between browsers on the same desk, and disappears when someone clears site data. Each of those is a shrug at a desk where re-picking a headset takes one click.

## The grid always wins

Personalisation stores loop references, so it stores things that can go stale against the permission grid. **It may only ever narrow within reach, it is evaluated against the grid on every load, and it is kept but inert when it falls outside.**

A saved subscription to a loop the role has since lost `monitor` on does not apply, and it comes back if reach is restored. This is [ADR-0013](./0013-arming-is-independent-of-subscription.md)'s preset posture one level up, where "a preset silently narrows to the loops its user may emit on". Keeping rather than deleting means a temporary revocation does not permanently destroy someone's console arrangement.

## Consequences

- **Personalisation is not the second authority layer [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) forbids.** It grants nothing and it is overruled silently and always. It is a preference the grid rules on, not a layer the grid composes with.
- **`CONTEXT.md`'s definition of `Role` was wrong and is corrected.** It claimed the role "is the carrier of permissions and console layout". The role carries permissions and a **default** console its occupants start from ([ADR-0052](./0052-a-role-default-is-a-starting-point-never-a-floor.md)), which is a different thing from carrying the layout itself.
- **A support request about audio devices cannot be answered from the server.** Nothing about which headset someone selected reaches the deployment, so a device problem is diagnosed at the desk or not at all.

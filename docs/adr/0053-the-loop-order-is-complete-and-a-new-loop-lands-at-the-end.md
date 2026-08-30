# The loop order is complete, and a new loop lands at the end

Loops arrive after install as a matter of course. [ADR-0015](./0015-the-admin-console-reads-one-row-at-a-time.md) invented the **unreviewed loop** precisely because they do, so a saved loop order has to answer for a loop that did not exist when it was saved.

There are three layers. Underneath everything is a **deployment-wide loop order** set by system administration. A **role default** may override it ([ADR-0052](./0052-a-role-default-is-a-starting-point-never-a-floor.md)). On top of that is the operator's own order, shared across the board and the ledger as [ADR-0032](./0032-the-console-is-two-views-of-one-loop-list.md) requires.

**The base order is administered, not derived.** Not alphabetical, and not creation order. A site that runs `FLIGHT`, `GNC`, `THERMAL` in that order on every wall display wants that order on the console too, and creation order is an accident of whoever typed fastest during setup.

**An operator's order is a complete ordering of the loops in their reach**, not a sparse set of pinned loops sitting on a base order. Sparse pinning sounds tidier and is worse to use: an operator who has arranged twelve loops has made twelve decisions, and a sparse model discards every decision that happens to agree with the base order, so the arrangement silently rearranges itself when the base order changes underneath it.

**A loop entering reach is appended at the end and marked as new** until the operator moves it or dismisses the marking. Appending is the only honest placement, because the system genuinely does not know where they want it, and inserting it into the middle of an arrangement they built is how it gets missed.

## Consequences

- **A loop that leaves reach and returns comes back where it was.** [ADR-0051](./0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md) keeps personalisation inert rather than deleting it, and the order entry is personalisation like any other, so a temporary revocation does not push a loop to the end of the list on its way back.
- **This is where [ADR-0015](./0015-the-admin-console-reads-one-row-at-a-time.md)'s unreviewed loops surface to operators.** A loop ruled on months after install enters reach and appears at the bottom of every affected console, marked. That is the operator-side counterpart of the administrator being told the cell was never ruled on.
- **The marking is the whole of the notification.** Nothing announces a new loop beyond its position and its mark. A loop somebody needs to be *on* is a hail ([ADR-0047](./0047-a-hail-is-a-monitoring-directive-without-the-authority.md)) or a monitoring directive, not an ordering concern.

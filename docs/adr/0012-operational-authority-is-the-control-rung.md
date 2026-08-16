# Operational authority is the `control` rung, and is scoped by loop

[ADR-0003](./0003-operational-authority-follows-the-role.md) established that operational authority is conferred by the role, and explicitly left its *scope* — the brief's "someone with elevated permissions for the group of people they're responsible for" — to the permission model. It is the top rung of the [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) ladder: a role holding `control` on a loop may act operationally **on that loop**, and holds no authority anywhere else.

**Scoping by loop rather than by people** is the substance of the decision. Scoping by people requires the system to answer "does this lead own that person", which churns with every shift change and every re-staffing, and has no natural home in a model where users carry nothing but eligibility. Scoping by loop needs no new concept at all — every operational action is already loop-shaped: cut an emission *on a loop*, direct monitoring *of a loop*. A shift lead is expressed as `control` on the three loops they run, which is a row in the grid an administrator can read like any other.

**Forcing a takeover is the awkward case**, because it targets a *role* rather than a loop. Rather than admit a second authority axis, it is derived: you may force a takeover of a role if you hold `control` on a loop that role is a **staffing role** for. This reuses a concept that already exists for occupancy and keeps the model to one grid.

## Consequences

- **`control` confers no configuration rights.** Editing the grid, creating loops and granting eligibility remain system administration ([ADR-0003](./0003-operational-authority-follows-the-role.md)) — a lead can cut a transmission on their loop but cannot change who may emit on it. The two are visibly different acts, held by different people, which is the point of the split.
- **Authority over a role with no staffing loop cannot be expressed.** A role that staffs nothing has no loop through which takeover authority can reach it. This is acceptable — such a role is by definition not accountable for a loop — but it is a real edge and should be surfaced in the admin console rather than discovered.
- **`control` implies `emit` implies `monitor`.** A lead who can cut people on a loop can also talk on it and hear it. There is no case for the alternative.

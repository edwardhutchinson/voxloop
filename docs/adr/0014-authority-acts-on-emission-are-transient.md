# Authority acts on emission are transient, and there is no proportionate sanction

[ADR-0008](./0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md) left open whether an administrator's silencing action targets the user or the role. The answer is neither, in the sense the question assumed: **it is not a permission at all.** Nothing an operational authority holder does during an event changes the grid; [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) admits no per-user exception layer for it to write into. What exists instead are two transient acts.

**Cut** stops a user's emission, and is latched until cleared. It applies to the **whole uplink**, not one loop, because a user emits one stream fanned out to their armed loops ([ADR-0007](./0007-the-client-emits-one-stream.md)) and an open mic is open on all of them. It is set by anyone holding `control` on a loop the target is currently emitting on ([ADR-0012](./0012-operational-authority-is-the-control-rung.md)), enforced server-side by closing the fan-out entries — reusing ADR-0008's existing "cut by authority" machinery, which matters because a hot-mic client is by definition one that has stopped paying attention. **The target clears it themselves**, as may any control holder. It dies with the session.

Cut exists to announce an open mic, not to punish one. It is **latched rather than one-shot** because of the stuck footswitch: a one-shot cut is defeated instantly by a client that re-keys because the button is physically held down. It is **cleared by the target** because the whole framing is a courtesy — "your mic is open" — and a sanction the target cannot lift is a different feature with different consequences.

**Force sign-out and account lock are two capabilities, not one**, because they fall on opposite sides of [ADR-0003](./0003-operational-authority-follows-the-role.md)'s split. Ending a live session is operational: gated by `control` on a loop the target's role staffs, it vacates the role — loudly and visibly, per [ADR-0005](./0005-occupancy-means-listening-not-signed-in.md) — and the user may sign straight back in. Disabling an account is system administration: the user cannot authenticate at all. Bundling them would mean an event lead either cannot act at all, or can permanently disable accounts. Account lock is kept distinct from revoking eligibility, which already ends occupancy immediately with a reason — lock kills every role at once and blocks authentication, and revoking eligibility role by role mid-incident is fiddly and easy to leave half-done.

## The gap, recorded deliberately

The brief asks that where "a certain person is taking up a lot of the airtime without merit on a certain loop, an administrator should be able to come in and just kick them off that loop". **VoxLoop v1 has no proportionate answer to that.** The available responses are Cut, which the target can clear, and force sign-out, which vacates a staffed position mid-event. Nothing sits between them, because the thing that would sit between them is a persistent per-user deny, and [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) admits none.

This is a conscious retreat from the brief, taken for two reasons. Mission-control practice handles airtime discipline by voice and through the shift lead, not through tooling. And a punitive lever, once built, invites its use — a persistent silence is a state somebody must remember to lift, and a forgotten one means an operator is mute at the moment they most need to speak.

## Consequences

- **A determined bad actor is not stoppable by degrees.** The escalation is Cut, then force sign-out, then account lock. Anyone unhappy with that should reopen the question rather than reintroduce per-user denies quietly.
- **Cut needs a distinct signalling event** from the target's own unkey, and from ADR-0008's revocation cut — the operator must be told which happened and why.
- **Force sign-out has a loud side effect**: the role goes vacant and every loop it staffs changes staffing state. Correct for a last-resort action, but the console must make it obvious before the act, not after.

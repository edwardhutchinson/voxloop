# The admin console reads one row at a time

[ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) closed with a bet it could not settle alone: *"the admin console must render the grid well enough that reading it is genuinely how administrators reason."* A throwaway prototype of three structurally different consoles over the same pilot-scale world ([#20](https://github.com/edwardhutchinson/voxloop/issues/20)) settled it against the grid. **The console's primary surface is one row — a role and its loops — or one column — a loop and its roles, read as a list at full size.** The whole matrix is retained as a secondary reference view, not as the place administrators work.

**The model is untouched.** A permission is still one cell on a role × loop grid with no second layer; nothing here reopens that. What changes is only how it is rendered and read.

Two facts from the prototype drove it. **The grid is not sparse**: a realistic pilot configuration fills 167 of 300 cells, so reading a column means reading eleven role names rather than three, and the "answerable at a glance" claim assumed a thinner object than the real one. And **the break point is closer than the scale envelope suggests**: at roughly 26–30 loops the matrix needs horizontal scroll, and once the row header and the far column cannot be on screen together a column read stops being a glance and becomes scroll-and-remember. Twenty loops sits under that line with very little headroom.

Crucially, the list rendering does not abandon ADR-0011's argument — it delivers it better. A role page **is** the row read and a loop page **is** the column read; the single-lookup property was always the thing that mattered, and the matrix was only one rendering of it.

## Reach is per (role, loop), and the console does not compose it

There are two administrative surfaces and v1 deliberately does not join them: **eligibility** answers which roles a user may sign into, and **the grid** answers which loops a role may reach. Asking what a *person* can do means reading their roles and then reading those rows — one extra hop, taken knowingly.

A merged per-person view was prototyped and rejected. At pilot scale roughly two-thirds of users have a reach that differs depending on which role they sign into, so a union would display authority nobody can actually hold: a session is bound to exactly one role, so a person's reach is only ever one row at a time. ADR-0011's phrase "what can this person reach is their role's row" turns out to be exact, and the words carrying the weight are *their role's*.

## Live keying is not admin console state; occupancy is

The console shows who is signed in and into which role. It does **not** show who is transmitting. This splits along the line [ADR-0008](./0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md) already draws: occupancy is configuration-adjacent and stable enough to administer against, keying is operational and belongs to the operator console ([#12](https://github.com/edwardhutchinson/voxloop/issues/12)).

## Destructive edits are guarded at commit time, not ambiently

That trim has a consequence which must be paid for. ADR-0011 states that revoking `emit` cuts a transmission mid-word and revoking `monitor` drops subscriptions; with no transmitting indicator anywhere, an administrator would do both blind. So **every cell edit computes its effect and shows it before committing** — who is cut mid-word, whose subscriptions drop, which loop loses its last staffing role and goes vacant, which occupants lose operational authority.

Computing it at edit time rather than displaying it continuously is what makes the trim affordable: the server already holds live sessions, their subscriptions and their arms, so this is one query at the moment it matters rather than a live feed the console must maintain. It is also the only place ADR-0011's stated consequences become visible to the person causing them.

## A loop created after install is unreviewed until an administrator rules on it

`Observer` is seeded `monitor` on every loop present at install ([#7](https://github.com/edwardhutchinson/voxloop/issues/7)), so a loop created later has no Observer cell and every listen-only user is blind to it. The prototype made the scale of this visible: two such holes at pilot scale, twenty-two at forty loops.

An absent cell means denied, and that stands. But absent-because-denied and absent-because-nobody-has-ruled render identically, so the marker attaches to the **loop** instead: a newly created loop is *unreviewed* until an administrator has set or explicitly dismissed each role's cell. Auto-seeding `Observer` on new loops was rejected — it would silently grant reach on every loop created, including the ones created precisely because something should not be broadly audible.

## Consequences

- **ADR-0011's closing consequence is amended, not withdrawn.** The model still has to be readable, and it is; it is read a row at a time rather than as a matrix. Nothing about the single-cell model depended on the matrix rendering, which is why the verdict costs the model nothing.
- **The matrix survives as a reference view.** It is the only place a whole-configuration read is possible — checking the *shape* of a grid, spotting a role with no reach or a loop nobody can hear — and that is a reviewing act, not an administering one.
- **Blast-radius computation is a v1 server requirement, not console polish.** It needs a read of live sessions, their subscriptions and their arms, resolved per (role, loop) at edit time. Anything that makes that read expensive or unavailable breaks the compensation this ADR depends on.
- **`unreviewed` is a state of a loop and needs a home in the configuration model**, alongside the loop's name and its staffing roles. It is cleared per loop, not per cell.
- **Eligibility is not a second matrix.** The prototype rendered it as one — 190 × 15 — and it was the least legible object built. It is administered from the role ("who may sign into this") and from the user ("which roles may this person sign into"), never as a wall.
- **The operator console is not settled by this.** [#12](https://github.com/edwardhutchinson/voxloop/issues/12) faces a different problem — live state under time pressure rather than configuration — and may well reach the opposite verdict about density.

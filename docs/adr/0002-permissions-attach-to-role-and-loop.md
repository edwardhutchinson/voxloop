# Permissions attach to (role, loop), not to the user

> ⚠️ **Amended twice.** [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) **retires the user-scoped override** in the consequences below — there is no per-user exception layer anywhere in VoxLoop, and each case that was expected to need one found a better home (a mid-event silencing became [Cut](./0014-authority-acts-on-emission-are-transient.md), listen-only became the seeded `Observer` role, and request-access became an administered configuration edit). [ADR-0051](./0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md) **corrects the personalisation scoping**: only volume is `(user, role, loop)`; bindings are per user, the subscription set, loop order, default view and personal presets are per `(user, role)`, and the audio device belongs to the machine. Read this ADR for why permission attaches to the pair, not for either of those.

Whether voice can be received or emitted is a property of the pair **(role, loop)**. A user holds no operational permissions of their own; they acquire them by signing into a role. What a user does carry is *eligibility* — which roles they are allowed to sign into — and *personalisation*, such as per-loop volume and console layout, scoped to (user, role, loop).

The originating brief proposed the opposite: a per-user permission vector. Attaching to the role instead is what makes the model comprehensible at the pilot's scale. An administrator reasoning per user faces roughly 190 users against 20 loops and must redo the work whenever staff change; reasoning per role faces roughly 15 against 20, the answer is stable across staff turnover, and a new starter is correct the moment they are granted a role. It also gives the mid-event action the brief asks for — an admin silencing someone on a loop — a well-defined shape: either the role's permission changes, affecting every occupant, or a user-scoped override is applied on top, and those two are visibly different acts.

Every voice loop system surveyed in [prior art](https://github.com/edwardhutchinson/voxloop/issues/4) attaches permission to the position rather than the person, and openvocs independently uses exactly `none | recv | send` per (role, loop) with volume per (user, role, loop).

## Consequences

- Granting one specific person one specific extra loop requires either a role that expresses it or a user-scoped override. Overrides are accepted, to be used sparingly and rendered distinctly in the admin console so they cannot hide.
- Roles are the unit of administration for voice, but *eligibility* — which users may sign into which roles — becomes a second thing admins manage.

# The role is the profile

> **Discharges the deferred work promised by [ADR-0013](./0013-arming-is-independent-of-subscription.md)**, which named profiles as "persistent, deliberately switched, covering subscriptions, volumes and layout". The deferral is settled by deleting the feature rather than by building it.

VoxLoop has no named profiles. A user has **one implicit personalisation set per `(user, role)`**, saved continuously as they change things, never named and never chosen between. The only wholesale act on it is a **reset to role default**.

**The role is already the switching mechanism.** Changing what you are doing means assuming a different role, and `Relinquish` is deliberately a full stop: audio ceases, subscriptions and arms are gone, and it is "never presented as seamless". A named profile would add a second, softer switching axis carrying no authority change behind it, competing with the act the model already has for exactly this.

It is also a worse experience than it sounds. [ADR-0016](./0016-displayed-state-is-observed-or-asserted.md) bans optimistic rendering, so a profile switch lags visibly across every card at once, and the operator has to re-read their whole console mid-shift. The cases that motivate named profiles are served already: **presets** for reaching a different set of loops with your voice ([ADR-0013](./0013-arming-is-independent-of-subscription.md)), and ordinary subscribe and unsubscribe for changing what you hear.

## Reset to role default

The wholesale act, and it is honestly named. **A reset always goes to the role default as it stands now**, never to the state this `(user, role)` pair started in.

Snapshotting the template at first assume was rejected. It means a second store of template data that drifts from the real one, and it fails in the worst place: an administrator who notices a role default is wrong and fixes it would find users resetting straight back to the broken version, which is precisely the moment somebody reaches for reset.

A reset clears the `(user, role)` and `(user, role, loop)` items, so the subscription set, loop order, default view and per-loop volumes. It leaves **PTT bindings** alone, which are per user and follow the person across roles, and it leaves the **audio device** alone, which belongs to the desk ([ADR-0051](./0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md)). It touches **nothing live**: a mute or an arm set is something the operator did minutes ago, and clearing those silently is the surprise [ADR-0050](./0050-personalisation-persists-what-is-safe-to-be-stale.md) exists to avoid.

**A reset is confirmed before it runs.** [ADR-0032](./0032-the-console-is-two-views-of-one-loop-list.md) settled that toggling one loop needs no confirmation, and this is not a safety case either. The reason is the lag: twenty cards redrawing without optimistic rendering will read as a fault unless the operator asked for it a moment earlier.

## Consequences

- **"Profile" is retired from the vocabulary**, and `CONTEXT.md` says so. It survives only in the `_Avoid_` line of **Preset**, where the distinction it was drawn against no longer exists.
- **A mute is dropped when its subscription is.** `CONTEXT.md` defines a mute as leaving the subscription standing, so a mute presupposes one. A reset that unsubscribes a loop takes its mute with it. Derived, not decided.
- **No administrator can see or reset another user's personalisation, and none needs to.** The support answer to "my console looks wrong" is the reset button, which resolves the whole class of complaint without building a way for one person to inspect another's console. [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) worked hard to keep per-user state out of the model, and a per-user console viewer for support convenience is a poor trade against that.
- **Bringing named profiles back needs an argument against role assumption**, not merely a use case for them. The use cases exist; what they lack is a reason to prefer a second switching axis to the one the model already has.

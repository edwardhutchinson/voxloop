# Personalisation persists what is safe to be stale

[ADR-0038](./0038-sqlite-behind-domain-shaped-repositories.md) lists "personalisation" in the persistent set without saying which items that covers, and [ADR-0039](./0039-live-state-is-in-process-behind-one-state-authority.md) holds everything live in process where it dies with the server. This ADR draws the line between them.

| Item | Persists |
|---|---|
| Subscription set | yes |
| Per-loop volume | yes |
| Loop order | yes |
| Default console view | yes |
| Personal presets | yes, already [ADR-0013](./0013-arming-is-independent-of-subscription.md) |
| PTT bindings | yes, already [ADR-0021](./0021-ptt-input-is-a-level-with-liveness.md) |
| Mute | no |
| Arm set | no |
| Off console | no |
| Audio device | yes, on the machine, see [ADR-0051](./0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md) |

**The test is whether a stale value harms the operator before they look at it.**

A stale subscription set does not. Worst case you hear a loop you would have dropped, and the board shows it sitting there. A stale **arm set** does: you key and speak to a loop you had forgotten you were armed on, and [ADR-0013](./0013-arming-is-independent-of-subscription.md) already made emitting blind legal. A stale **mute** does too, and it costs more than the muter: it drops every loop they staff to `away` for the whole operations centre the moment they assume, before they have looked at anything.

**Persisting subscriptions is what makes [ADR-0039](./0039-live-state-is-in-process-behind-one-state-authority.md)'s accepted cost survivable.** A restart ends every session and every operator has to assume again. If the subscription set persists, assuming rebuilds their console. If it does not, every operator rebuilds their loop set by hand during whatever incident caused the restart. That is the strongest single argument in this decision.

## The write path

**Personalisation is written through as the live act is applied.** Capturing it when the session ends was rejected outright: ADR-0039 ends every session with no chance to flush anything, and the restart case is the entire reason subscriptions persist at all. A scheme that loses the state exactly where it is needed solves nothing.

A subscription is therefore a live act with a durable consequence, which crosses the rule [#13](https://github.com/edwardhutchinson/voxloop/issues/13) drew: the WebSocket carries only live session state and mediasoup signalling, and everything configuring the system is ordinary HTTP. **That rule stands and is restated rather than broken.** The WebSocket still carries no configuration API. It carries live acts, and the state authority writes the durable consequence of some of them as it applies them. What the rule protects is an enumerable and individually rulable endpoint list, and this leaves that intact.

**Personalisation persistence is best effort, and it must never be able to fail a live act.** If the live change succeeds and the write fails, the operator's console is correct and their preference is lost. Refusing to subscribe someone to a loop because SQLite is unhappy would be the worst available ordering of those two concerns.

## Consequences

- **A restart costs an assume, not a rebuild.** ADR-0039's consequence is unchanged in kind and much cheaper in practice.
- **[#23](https://github.com/edwardhutchinson/voxloop/issues/23) inherits a statement rather than an exception.** Personalisation adds no endpoint to the authorisation list, because it is not reached through one.
- **Off console never persists, and could not.** [ADR-0016](./0016-displayed-state-is-observed-or-asserted.md) makes it asserted state, only as true as the moment it was asserted. A day-old assertion is not a fact about anything.
- **This is a new write class at operator rate.** A few writes per operator per shift, against grid edits and audit entries. It is not a load question, but it is the first durable write driven by ordinary console use.
- **Deleting a loop or a role takes its personalisation with it.** There is nothing to preserve once the thing being personalised is gone.

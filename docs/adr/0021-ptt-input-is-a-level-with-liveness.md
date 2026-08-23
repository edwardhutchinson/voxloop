# Push-to-talk input is a level with liveness, never an event

Every push-to-talk input source publishes two things across one interface: a **level** — *this source currently wants to emit* — and a **liveness flag** — *this source is present and working*. The client ORs the levels of all live sources and drives [keying](../../CONTEXT.md) from the result. Sources never publish events.

This seam exists from day one, before the source that most needs it. [ADR-0020](./0020-the-browser-is-the-client.md) defers the native global hotkey to the very end of v1, so the difference between the wrapper being a wrapper and being a retrofit is entirely whether this interface was there first.

## Why a level rather than edges

The obvious shape is `keydown` / `keyup`, or `emitStart` / `emitStop`. It was rejected because **edges are lossy and their loss mode is an open mic**: one dropped release and the operator is transmitting indefinitely, with no subsequent event to correct it. A level is self-correcting — the next sample says what is true now, and any source that stops reporting is caught by its liveness flag rather than by silence.

The sources also genuinely differ in what they can report, and a level is the only shape all of them can produce honestly. A HID report is a level (the button is held). A global hotkey is a pair of edges. An on-screen control is a pointer state. Normalising edge-shaped sources up to a level is a small, local, testable piece of work; normalising a level down to edges throws away the thing that makes it safe.

The concrete case this already prevents, in v1's only unfocused-adjacent path: **holding `` ` `` and switching to another application never delivers `keyup`.** The keyboard source drops its level on window `blur`, so focused PTT ends when focus does. Under an event-shaped seam that is a hung transmission.

## Why liveness is part of the interface

Liveness is the part events cannot express at all. *"The headset was unplugged while you were holding the button"* is a property of the **source**, not of any event the source could have sent — by the time it is true, the source is gone and can send nothing.

It is what makes two things possible:

- **Honest degradation.** The console can say *why* push to talk is unavailable — no wrapper, no device, no binding — rather than showing a control that does nothing. Under [ADR-0016](./0016-displayed-state-is-observed-or-asserted.md) the console may not misrepresent what a user can do, and a dead input source with no liveness signal is exactly that misrepresentation.
- **Forced unkey on disappearance.** A source that dies while keyed forces an unkey and says so locally, rather than leaving the operator to discover it.

**The microphone and the PTT source are tracked as independent liveness signals.** They fail separately and they fail together — on a headset with an inline button, a single unplug takes out both — and conflating them would make the common case unreportable.

## Consequences

- **New sources are registrations, not code paths.** The wrapper's global hotkey, WebHID if it ever returns from fog, and a mouse side button are all the same shape. This is the whole reason the wrapper can be last.
- **Mode logic lives above the seam, not in the source.** A source reports intent; it does not know whether it is momentary or latched. [ADR-0022](./0022-latch-is-never-derived-from-a-momentary-press.md) depends on this — a source that decided its own mode could latch by accident.
- **Any source can be bound to any control.** A mouse side button arrives as an ordinary `mousedown`/`mouseup` and needs no work, so the seam gives it away for free.
- **Bindings are per-user personalisation.** Consistent with [ADR-0002](./0002-permissions-attach-to-role-and-loop.md): users carry eligibility and personalisation, roles carry authority. A keybinding is not reach, so it is the user's, and an operator may converge their browser binding and their wrapper hotkey on one combination for a single muscle memory across tiers.
- **Detailed device selection is not settled here.** Which microphone, which output, and whether the choice is remembered per user or per console machine belong with defaults and profiles. What is settled is the rule they must obey: independent liveness, and a device disappearing while keyed forces an unkey.
- **Priority is a second level channel, not a fourth kind of source.** [ADR-0046](./0046-priority-is-keyed-not-held.md) adds a priority binding whose sources report intent and liveness exactly like the others, and the client computes `emitting = ordinary-level OR priority-level` and `is-priority = priority-level`. Elevating a latched transmission and dropping back out of it needs no special case at all, which is the clearest evidence so far that the level abstraction was the right one.

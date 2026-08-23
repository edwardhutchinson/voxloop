# A loop is staffed when someone is listening to it, not when someone is signed in

> **Amended by [ADR-0017](./0017-loop-health-is-measured-not-asserted.md) and [ADR-0019](./0019-presence-is-one-versioned-document-scoped-to-reach.md).** The three-valued state this ADR governs is now called **staffing state**, reserving *occupancy* for roles; the filename is left alone so existing links survive. And `staffed` now means *demonstrably hearing*, not merely *subscribed* — beacon loss and an unreachable session both count against a loop being `staffed`, alongside off console and muted. ⚠️ **Further clarified by [ADR-0045](./0045-priority-defeats-attenuation-and-nothing-else.md)'s session**: staffing state is computed across *every* occupant of *every* staffing role for the loop, so one occupant muting or stepping away moves nothing while another is still hearing it, and there is no partial value in between.

Staffing is a flag on the (role, loop) pair — a loop may have several staffing roles or none — and a loop counts as `staffed` only when an occupant of one of those roles is **currently subscribed to it**. Being signed into a staffing role is not enough.

The originating brief asks for one thing above all here: before you key up to the Support Engineers loop, you need to know whether anyone is actually behind it. Deriving that from sign-in answers a proxy question instead — someone holds the position, but may have dropped the loop from their console — and a tile that reads `staffed` when nobody will hear you is the exact misrepresentation the product cannot afford.

Two constraints keep the signal honest:

- **A staffing role must have `send` on the loop it staffs**, enforced at configuration time. The point of the signal is telling an emitter whether keying up is worth it; a silent monitor who cannot reply on that loop is not cover. Roles may still monitor loops they cannot speak on — that simply does not count toward staffing.
- **Marking a role as staffing a loop subscribes its occupants to that loop by default at sign-in**, and the subscription remains droppable. The default makes the common case right without anyone thinking about it; keeping it droppable means that when someone really has dropped it, `vacant` is the true answer rather than a hidden one.

## Consequences

- One operator tidying their console can make a loop read `vacant` for everyone else. That is correct, but it should not be an invisible side effect — dropping a loop you staff should tell you so as you do it.
- ⚠️ **Where staffing genuinely must not be droppable, there is no tool.** This originally named the [monitoring directive](./0004-monitoring-directives-are-enforced-and-additive.md) as that tool, but [ADR-0035](./0035-a-monitoring-directive-promotes-a-loop-it-does-not-police-it.md) made directed subscriptions droppable. Nothing in VoxLoop now compels a subscription to be kept, and if that is ever genuinely needed it has to be designed.

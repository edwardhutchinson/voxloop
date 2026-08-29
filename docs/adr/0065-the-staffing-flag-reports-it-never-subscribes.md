# The staffing flag reports, it never subscribes

> **Amends [ADR-0005](./0005-occupancy-means-listening-not-signed-in.md)**, striking its second bullet. Marking a role as staffing a loop no longer subscribes that role's occupants to it, by default or otherwise. [ADR-0052](./0052-a-role-default-is-a-starting-point-never-a-floor.md) stands unchanged: the **role default** is the only thing that seeds a console.

Assembling [`docs/spec/v1.md`](../spec/v1.md) found two settled decisions that cannot both hold. ADR-0005 says marking a role as staffing a loop subscribes its occupants "at sign-in", which under [ADR-0023](./0023-sign-in-is-to-the-application-and-a-role-is-assumed.md) now reads *at every assume*. ADR-0052 says the role default is the only seed, that it is **applied once and never re-imposed**, and that a subscription is never re-added under an operator.

The collision is narrower than it first looks. ADR-0052 rejects *enforced minimums*, and ADR-0005's default was always droppable, so that clause is not what bites. What bites is re-imposition. An auto-subscribe firing at every assume silently re-adds a loop the operator deliberately dropped last shift, behind no `control` gate, on a schedule nobody chose. ADR-0052 rejected a time-of-day default partly because it changes an operator's console with nobody touching it; this is that, keyed on the operator's own sign-in instead of the clock.

**So the staffing flag confers nothing.** It says one thing: *this role counts toward this loop's staffing state*. ADR-0005's other constraint stands untouched — a staffing role must hold `emit` on the loop it staffs, because a role that cannot answer is not cover.

## The hole this opens, and why no seeding rule closes it

ADR-0005's default was covering something real. A role that staffs a loop none of its occupants subscribe to leaves that loop reading `away` forever, which is ADR-0005's own misrepresentation arriving by omission.

Nothing seeded can close it. [ADR-0050](./0050-personalisation-persists-what-is-safe-to-be-stale.md) persists the subscription set per `(user, role)`, and the grid **may only ever narrow within reach** — it never adds. So an administrator marking a staffing role on an established deployment reaches nobody who has already personalised, whatever the seed does. The case that produces a permanently `away` loop is precisely the case seeding arrives too late for.

Compulsion is the other candidate and it is unavailable at any price ([ADR-0035](./0035-a-monitoring-directive-promotes-a-loop-it-does-not-police-it.md), ADR-0052). **The answer is therefore visibility, in four places, and no new mechanism anywhere.**

**`away` gains a fifth reason: `not subscribed`.** ADR-0005's three-valued state carried four reasons — muted, off console, unreachable, not receiving the beacon — and an occupant who simply does not have the loop on their console fitted none of them. It is materially different from the other four: they all mean *the loop is on their console and something is wrong*, and this one means the loop was never there. [ADR-0019](./0019-presence-is-one-versioned-document-scoped-to-reach.md)'s audience computation had already carved `not subscribed` out as its own bucket beside *present but not hearing*; staffing state was simply out of step with it.

**The console mark gains a second state.** The loops a role staffs are already marked in both views. The mark now distinguishes *you staff this* from *you would staff this, and are not subscribed* — the second being actionable, since clicking the card body subscribes. Showing the mark only in the second case was rejected: an operator correctly staffing four loops would see nothing telling them so, and the mark would appear for the first time at the moment something was wrong, making it an alarm rather than a fact about their own console.

**The `away` reason is a count over occupants.** ADR-0045's session settled that staffing state is computed across every occupant of every staffing role with no partial value, so occupants can be away for different reasons at once, and the existing sentence forms were written for one person. The ledger reports counts (`away — 1 muted, 2 not subscribed`), collapsing to today's plain sentence when they agree. Picking one winner by precedence across occupants was rejected because no defensible ordering exists — a mute is one click from hearing and so is a subscription. Counts rank nothing, and [ADR-0034](./0034-the-transmit-bar-is-always-visible-and-the-audience-is-a-count.md)'s two-count audience is the precedent.

Within a *single* occupant the ordering does fall out of the mechanism, because the conditions nest. Report the one still true if everything below it were fixed: `unreachable`, then `off console`, then `not subscribed`, then `not receiving it`, then `muted`. This generalises the rule [ADR-0044](./0044-resynchronisation-narrates-the-gap.md)'s session already set, where beacon loss is suppressed while connection state explains the silence. Two pairs are impossible rather than ordered, since mute and the beacon both presuppose a subscription.

**The lobby carries the reason, not just the state.** It is read once, deliberately, by someone deciding whether to take a seat, which is the ledger's posture rather than the board's — and it is the one place `not subscribed` reaches a person about to be in a position to fix it.

## Alternatives

**Keep ADR-0005 and re-impose at every assume.** Rejected above.

**A second seed: the union of the role default and the staffed loops, at first assume only.** This survives ADR-0052's letter, since it is applied once. Rejected because it costs the model's clearest property — *what seeds a console* would have two answers, and `Reset to role default` would mean something other than the role default. It also cannot express an administrator who marks a staffing role and deliberately does not want it defaulted, which the union has no way to say.

**An admin console check on the mismatch**, either a standing condition on the loop and role pages, or a coupled write where setting the staffing flag proposes adding the loop to that role's default in the same gesture. **Rejected on Ed's call**: administrators set defaults how they want, and if an occupant of a staffing role is hearing the loop it is staffed, which is the whole of the logic. The rejection is worth recording because the alternative was designed before it was dropped: it would have reached somebody who has to go looking first, and it bought a standing derived condition and a second audited write to do it, where the operator-facing signals reach the one person who fixes it in a click.

**A banner at assume naming unsubscribed staffed loops.** Designed and dropped for the same reason. It would have been the first server-generated advisory in the product, needing a slot, a lifetime and a dismissal rule, to say something the board is already showing on the card.

**Refusing the staffing-flag commit on a mismatch.** Rejected before it reached Ed: it makes the order of two administration acts load-bearing, and it forbids a configuration that may well be deliberate.

## Consequences

- **A loop can read `away — not subscribed` indefinitely, and nothing in VoxLoop forces it closed.** Accepted knowingly. It is now stated on the console, in the ledger and in the lobby, which is the whole of the compensation.
- **The presence document must carry, per loop, whether this session's role staffs it.** It does not today, and both mark states are derived from it client side. This adds a field, not an operation.
- **No operation is added.** [`api-surface.md`](../spec/api-surface.md) gains no row, so [ADR-0054](./0054-every-operation-declares-its-authorisation.md) is satisfied without an edit. Setting the staffing flag and editing a role default remain two independent system-administration writes, and nothing couples them.
- **Nothing here is audited**, because nothing here is a decision. The staffing flag and role default edits were already audited configuration changes and are unchanged.
- **`away` reasons are now a set with counts rather than a string**, which is a shape change wherever the reason is rendered: the ledger, the lobby, and any later surface. The board is untouched — [ADR-0032](./0032-the-console-is-two-views-of-one-loop-list.md)'s card still carries the bare word.
- **A loop sitting `away — not subscribed` for days is an operations signal nobody watches.** It is the cheapest possible read — the server holds every subscription set and every staffing flag — and it is exactly the standing misconfiguration this decision declines to prevent. It belongs with the observability work.
- **`Observer` is unaffected.** It holds only `monitor`, and a staffing role must hold `emit`, so it can never be one.

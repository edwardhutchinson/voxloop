# The console announces as a designated principal, and never as the administrator

[ADR-0031](./0031-v1-injects-but-does-not-schedule.md) put one human-facing surface in the product — an administrator types text, picks loops, and sends — and called it *"simply another client of [ADR-0029](./0029-an-announcement-is-an-ordinary-transmission.md)'s endpoint"*. That clause does not survive [ADR-0054](./0054-every-operation-declares-its-authorisation.md). Announce requires `ServiceToken`, the page runs in a cookie-authenticated browser, and a request presenting both is refused rather than resolved by precedence. Nothing else about ADR-0031 is disturbed; the page simply had no reachable path, and [#28](https://github.com/edwardhutchinson/voxloop/issues/28) is where that was found.

**The operation is `SystemAdministration`, and the server executes it as the designated console principal.** No token ever reaches a browser, so ADR-0054's cookie-plus-token refusal is never tested — the administrator's request carries a cookie and nothing else. Announce's own reach check then runs against the **bound role's** row, exactly as it does on the token path, because the emitting principal is the same principal either way.

So one call carries two checks against two principals: the **caller** must hold the system-administration flag, and the **actor** must hold `emit` on every loop named. This is the seam ADR-0031 asserted, written down rather than assumed.

## Two rows in the list, one implementation behind them

[`api-surface.md`](../spec/api-surface.md) gains a second row under system administration. It has to be a second row, because ADR-0054 gives every operation exactly one requirement and an operation whose requirement depends on who called is the ambiguity that rule exists to remove.

**Two rows are not two implementations.** The only thing that differs is how the acting principal is resolved: from the presented token, or from the deployment's designated console principal. Reach check, server-composed prefix, pronunciation dictionary, backlog refusal, maximum length, priority flag, synthesis and injection are one path. Stated here because the failure mode is specific and slow: without it the two drift, and the console path is the one that quietly stops refusing an unreachable loop.

`Grid(rung, loop)` is worded in ADR-0054 in terms of the assumed role. It means **the acting principal's role** — the assumed role for a user, the bound role for a service principal — which is what the existing announce row already relies on. That is a wording tidy, not a model change.

## Why a system administrator may trigger an operational act

ADR-0031 flagged this page as *"the first thing in the product where system administration triggers an operational act"*, safe *"only because it triggers it as somebody else"*, and said any later feature tempted to have an administrator act directly should be read against it. A `SystemAdministration` operation that emits voice is exactly such a feature, so the argument is made here rather than inherited.

**An administrator gains no reach they did not already have.** They can edit the console role's grid row and then announce, and that has always been true of every loop in the system — [ADR-0003](./0003-operational-authority-follows-the-role.md) makes system administration the capability to configure, and configuring the grid is the whole of it. What the design buys is that the reach must be configured *before* the moment, in the grid, where a cell edit is audited with the blast radius [ADR-0015](./0015-the-admin-console-reads-one-row-at-a-time.md) already computes. The page cannot widen its own reach.

Attribution stays honest. The transmission is attributed to the bound role, never to the administrator, and [ADR-0029](./0029-an-announcement-is-an-ordinary-transmission.md)'s flag marks it as synthesised, so no operator can mistake it for a person.

**The honest weakness: for a determined administrator, "edit the grid first" is two clicks, not a wall.** The audit log is the only thing watching, which is the same posture [ADR-0046](./0046-priority-is-keyed-not-held.md) took with the ungated priority key.

**Cutting the page was the real alternative.** It sets no precedent and leaves the model cleaner, and it was rejected because ADR-0031 already commits to telling the pilot customer that v1 cannot schedule an announcement. Widening that from *you cannot schedule it* to *you cannot say anything without writing code* costs more than the precedent does. Minting a short-lived token for the browser was rejected on sight and is recorded so nobody proposes it again: it invents a second token lifetime against [ADR-0027](./0027-a-service-principal-acts-through-a-role.md)'s standing grant, puts a live credential in a page anyone can open dev tools on, and buys nothing.

## The page may key priority

The worry [#28](https://github.com/edwardhutchinson/voxloop/issues/28) raised was that an administrator would produce an audit entry attributed to a role they do not occupy. [ADR-0067](./0067-composed-text-is-a-decision.md) dissolves it: the entry names the administrator, with the bound role beside it as attribution, which is the shape [ADR-0028](./0028-the-audit-log-records-decisions-not-traffic.md) already uses for authority acts.

What remains is whether an administrator holding no role should have a full-gain path into every subscriber's ears, and it costs less than it sounds. [ADR-0045](./0045-priority-defeats-attenuation-and-nothing-else.md) makes priority the weakest override in the system: attenuation only, never mute, never reach, never subscription. ADR-0046 gates it on nothing at all for humans. Denying it here would leave the urgent case needing a script and a token, which is the one case that justified building the page.

**One audit entry covers the announcement and its priority together.** Unlike a keyed press there is no separate act to record — the flag is set on the same call.

## Seeding, and what the page does before it is configured

Install seeds one service principal and one role bound to it, the role marked **not human-eligible** per ADR-0027, so *who occupies it* answers **nobody, ever**.

**It is seeded with no reach.** Every cell `none`, which is what ADR-0054 requires of anything nobody has ruled on. `Observer` is the tempting precedent and it is the wrong one: `Observer` listens, this role speaks, and a page that can announce everywhere the moment the box boots contradicts the argument above, which rests entirely on the grid having been edited first. The consequence is that **the page is inert on a fresh install**, and it must say so and point at the role's grid row rather than showing an empty loop picker.

**The seeded role's name is spoken aloud**, because ADR-0029 prepends it and the pronunciation dictionary applies to it. `Announcements` reads correctly — *"Announcements: evacuate the pad"*. A site renames it with an ordinary role edit.

**Which principal the page acts as is fixed, not configurable.** An editable pointer is a knob with no use case, and a site wanting a different name renames the role. Beyond the designation it is an ordinary service principal, so an administrator may issue it a token like any other; forbidding that would put a special case in the principal model to prevent something harmless. Install issues no token, so by default no credential for the console principal exists anywhere.

## Consequences

- **The announcement path now has two front doors and must be tested through both.** The shared implementation is the mitigation, not the alibi.
- **A system administrator can announce onto a loop no role they are eligible for can reach.** This is the direct consequence of the acting principal being somebody else, and it is the point of the design rather than a leak in it. The compensations are that the reach is configured in the grid, the transmission is attributed to the console role, and [ADR-0067](./0067-composed-text-is-a-decision.md) names the human in the log.
- **The seeding obligation ADR-0031 left loose is now discharged here** rather than in deployment fog: a principal, a role, no reach and no token.
- **Nothing about the token path changes.** A script bound to a different principal behaves exactly as ADR-0029 specifies, and its announcements remain unaudited.

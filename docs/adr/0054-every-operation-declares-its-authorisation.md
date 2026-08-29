# Every operation declares its authorisation, and nothing defaults to open

[#15](https://github.com/edwardhutchinson/voxloop/issues/15) found that openvocs' admin API — `get_clients`, `start_record`/`stop_record` — performs **no authorisation check at all**, while the database-admin API beside it checks properly. Nobody decided that. It survived because the requirement was implicit, and an implicit requirement is one somebody forgets on the next endpoint. This ADR fixes the rule for every operation VoxLoop has, and fixes what happens to an operation nobody ruled on.

The companion artefact is [`docs/spec/api-surface.md`](../spec/api-surface.md), which lists every operation with its requirement. This ADR says why the list has the shape it does.

## Six requirements, and no seventh

Every operation carries exactly one of:

| | |
|---|---|
| `Public` | no principal |
| `SignedIn` | an authenticated user, no role |
| `Session` | a user who has assumed a role |
| `Grid(rung, loop)` | `Session`, and the assumed role holds at least `rung` on `loop` |
| `SystemAdministration` | the user-level flag of [ADR-0003](./0003-operational-authority-follows-the-role.md) |
| `ServiceToken` | a service principal's token ([ADR-0027](./0027-a-service-principal-acts-through-a-role.md)) |

Two candidates were considered and rejected. A `Self` tier, for a user acting on their own record, is a check *inside* a handler on an argument, not a rung — promoting it would mean every handler taking a user id has to be read twice to find out which kind it is. And splitting `SystemAdministration` into read and write was refused because ADR-0003 makes it one user-level capability that has to exist before any role does; a read/write split is the beginning of a second grid, and [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) permits one.

`Grid` is the only requirement that is a function of arguments rather than of the caller alone. That is deliberate — authority is read off the grid via the assumed role, exactly as [ADR-0029](./0029-an-announcement-is-an-ordinary-transmission.md) already does for announcements, and never invented per endpoint.

## The default is refusal, and it is enforced by the compiler

Registering a route takes an authorisation requirement as a **mandatory argument**. There is no default value and no way to register a route without one; `Public` is a variant somebody has to type by name, in a diff a reviewer sees. A default-deny middleware sits behind it as a backstop, but the middleware is not the mechanism — an allowlist table is one more place to forget an entry, which is the failure this ADR exists to prevent, wearing a different hat.

The same rule holds one level down, on the grid itself. An **unreviewed loop**'s cells are enforced as `none`, with no exception anywhere. `unreviewed` is a display state and a prompt in the admin console; the evaluator cannot see the difference between it and a deliberate `none`.

## The cookie carries no claims

[ADR-0026](./0026-one-credential-and-the-media-path-carries-none.md) promises that revocation is immediate and central. That only holds if the cookie is an **opaque reference to a sign-in and nothing else**. Neither the system-administration flag nor the assumed role is in it.

So authority is resolved per request: the flag from SQLite, the role from the state authority ([ADR-0039](./0039-live-state-is-in-process-behind-one-state-authority.md)), which already holds it and is the single writer. Putting either in the cookie would buy a lookup and cost the promise — a revoked administrator would keep administering until their cookie expired.

## The signalling channel is a second authorised surface, with two tiers

[ADR-0040](./0040-one-binary-one-unit-four-moving-parts.md) put configuration on HTTP and left the WebSocket carrying live acts and mediasoup signalling. That makes the HTTP list enumerable; it does not make the socket trusted. Cut, forced relinquish and monitoring directives are the highest-authority acts in the product and every one of them is a socket message. Ruling only on HTTP would move openvocs' unchecked endpoint onto our socket rather than remove it.

**Every message is authorised, not just the upgrade.** Upgrade-time authorisation is the tempting shortcut and it breaks the moment an administrator edits a grid cell mid-shift: the operator's socket is already open, and a revoked `emit` would keep arming until they happened to reconnect. [ADR-0015](./0015-the-admin-console-reads-one-row-at-a-time.md) already has the server computing blast radius at commit time, so the current answer is in hand.

**One socket per tab, opened at sign-in.** It starts at `SignedIn`, carrying the lobby's presence document ([ADR-0023](./0023-sign-in-is-to-the-application-and-a-role-is-assumed.md)), and the hello presenting a session id ([ADR-0041](./0041-a-session-is-resumed-by-name.md)) moves it to `Session`. A message needing a session, arriving on a lobby-tier socket, is refused by the same per-message check. The alternative — polling the lobby over HTTP — would build a second way to render system state, which is the thing ADR-0023 reused the presence document to avoid.

## An authority act names the loop it claims authority through

Cut, forced takeover and forced relinquish are all gated on `control`, and each derives its loop by a different rule: the loop the target is currently emitting on, a loop the target role staffs, a loop the target's role staffs. Three derivations is three places to get it wrong.

Instead, **the caller names the loop**. The server checks that the caller's assumed role holds `control` on it, and that the target is reachable through it by that act's own rule. One shape for all three.

This also fills a hole in the audit log. [ADR-0028](./0028-the-audit-log-records-decisions-not-traffic.md) records the actor's role as well as the person, because ADR-0003 makes the role the source of the authority — but the role's authority is scoped by loop ([ADR-0012](./0012-operational-authority-is-the-control-rung.md)), so without the loop the entry does not actually say why the act was permitted. Now it does.

## `control` compels; it does not configure and it does not merely ask

The rung is for acts that change another session's live state **against their will**: Cut, forced takeover, forced relinquish, monitoring directives. It is not the rung for changing a loop in general.

Configuration is unaffected — ADR-0012 already states that `control` confers no configuration rights, so creating a loop is `SystemAdministration` and always was. And a **hail** stays on `emit` ([ADR-0047](./0047-a-hail-is-a-monitoring-directive-without-the-authority.md)), because it can be dismissed with one click. Pulling hail up to `control` would make it identical to the monitoring directive again, which is the distinction ADR-0047 spent the whole ticket establishing.

## The unauthenticated surface, in full

Four things and no more: the static bundle, sign-in, enrolment-code redemption, and first-start bootstrap redemption. Plus one health route, returning **liveness only** — no version, no counts, no user or loop names. Without it a customer's monitoring polls the sign-in page, which is worse for everyone. Subprocess, disk and backup health stay on the admin console behind the flag.

**A signed-in user may change their own password.** ADR-0025 removed self-service *reset* because there is no mail path; a change presenting the current password needs no mail path at all, and leaving it out means a password read aloud during onboarding can only be replaced by finding an administrator. It is `SignedIn`, rate-limited, audited, and does **not** end the session — an administrator-forced reset still does.

**The bootstrap route is re-minted, not persistent.** Every start with no system administrator in the database mints a fresh code and invalidates the previous one. The old code is sitting in a log file that may already have been copied off the box during a failed install. Once a system administrator exists, the route is not registered at all.

**The last system administrator cannot be removed.** Clearing the flag on, locking, or deleting the final one is refused. ADR-0025 built the on-box CLI for a lockout nobody chose; making it the answer to one the console cheerfully permitted is worse, and the recovery needs somebody with shell access at a customer site.

## Credentials do not mix, and limits are keyed on source

A request carries **exactly one credential kind**. Presenting both a cookie and a service token is refused rather than resolved by precedence — a confused deputy needs somewhere to be confused, and a precedence order is that place. The token rides an `Authorization` header, never a query string (ADR-0026), and the socket upgrade refuses tokens outright: a service principal has no session, no client and no media path ([ADR-0029](./0029-an-announcement-is-an-ordinary-transmission.md)).

**Rate limits are keyed on source, never on the submitted account name**, with a global ceiling behind them. ADR-0025 chose rate limiting over auto-lock precisely so that nobody could lock out the Flight Director. Keying the limiter on the account name reintroduces that attack in softer form: hammer the name, and the person starting the shift waits. The same limiter covers sign-in, enrolment redemption and bootstrap redemption.

## Refusals

A refused call says **you may not**, with the reason, rather than hiding the operation's existence. VoxLoop is one organisation on one box; hiding a loop's existence from a signed-in colleague buys nothing against a threat model where the box is already inside the perimeter. The single exception is the bootstrap route, which genuinely stops existing.

**Refused authority acts and refused administration writes are audited. Refused reads are not.** A denied Cut is a decision somebody tried to make, which is exactly ADR-0028's subject. A denied read is usually a stale console tab, and auditing those means a forgotten browser writes the log for you.

## Consequences

- **Adding an endpoint is now a decision somebody has to type.** The requirement is an argument, so the reviewer sees `Public` in the diff. This is the whole mechanism; if it is ever softened into a default, this ADR is void.
- **Authorisation costs a lookup on every request and every socket message.** That is the price of ADR-0026's immediate revocation, paid at the scale envelope of ~200 connected sessions rather than at internet scale.
- **A system administrator in the lobby cannot cut anyone.** Operational authority follows the role (ADR-0003), so an authority act needs `Session` before it needs `Grid(control, …)`. A sysadmin who wants to act operationally must assume a role that holds `control`, exactly like everyone else.
- **A service principal can call one endpoint.** Announce, and nothing else. Loop names are configuration a script holds; a read endpoint on the token path is how the service surface starts growing, and `get_clients` is what that looks like a year later.
- **There is no access-request endpoint**, and its absence is deliberate rather than pending. Asking for reach is a conversation with an administrator who then edits a cell, and the edit is the part VoxLoop already does.
- **Whether reading the audit log is itself audited is left open**, alongside retention, export and tamper-evidence. All four wait on the same missing fact: nobody has established the pilot customer's compliance posture.

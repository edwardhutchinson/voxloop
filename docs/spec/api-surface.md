# API surface and authorisation

Every operation VoxLoop exposes, with the requirement it carries. The rule behind the shape of this list is [ADR-0054](../adr/0054-every-operation-declares-its-authorisation.md); this file is the enumeration.

**Requirements.** `Public` (no principal) · `SignedIn` (authenticated user, no role) · `Session` (a role assumed) · `Grid(rung, loop)` (`Session`, and the assumed role holds at least `rung` on `loop`) · `SystemAdministration` (the user-level flag) · `ServiceToken` (a service principal).

**Audited** means the operation writes to the audit log per [ADR-0028](../adr/0028-the-audit-log-records-decisions-not-traffic.md). Refused authority acts and refused administration writes are audited too; refused reads are not.

Nothing here defaults to open. A route is registered with its requirement as a mandatory argument, so an operation missing from this list cannot exist.

## Public

| Operation | Transport | Notes |
|---|---|---|
| Fetch the client bundle | HTTP | Static assets embedded at release ([ADR-0037](../adr/0037-the-client-ships-as-static-assets-embedded-at-release.md)) |
| Sign in | HTTP | Rate-limited on source. **Audited**, success and failure |
| Redeem an enrolment code | HTTP | Single-use, expiring. The code identifies the user. Rate-limited. **Audited** |
| Redeem the bootstrap code | HTTP | Registered only while no system administrator exists. Re-minted each start, invalidating the previous code. Rate-limited. **Audited** |
| Liveness | HTTP | Liveness only. No version, no counts, no names |

## Signed in

| Operation | Transport | Requirement | Notes |
|---|---|---|---|
| Sign out | HTTP | `SignedIn` | **Audited** |
| Read own principal | HTTP | `SignedIn` | Eligible roles, system-administration flag |
| Change own password | HTTP | `SignedIn` | Current password re-presented. Rate-limited. **Audited**. Does not end the session |
| Open the signalling channel | WebSocket | `SignedIn` | One per tab. Starts at this tier |
| Lobby presence document | WebSocket | `SignedIn` | Scoped to eligibility ([ADR-0023](../adr/0023-sign-in-is-to-the-application-and-a-role-is-assumed.md)) |
| Assume a role | WebSocket | `SignedIn` + eligibility | Mints the session id. Moves the socket to `Session`. **Audited** as session start |
| Request a takeover | WebSocket | `SignedIn` + eligibility | Issued from the lobby against a single-occupant role |
| Resume a session | WebSocket | `SignedIn` + session id owned by this user | The session id is not a credential ([ADR-0041](../adr/0041-a-session-is-resumed-by-name.md)) |

## In a session, no grid check

| Operation | Transport | Requirement | Notes |
|---|---|---|---|
| Relinquish | WebSocket | `Session` | **Audited** as session end, with reason |
| Mute / unmute a loop | WebSocket | `Session` | Personal, never persisted |
| Set a loop's volume | WebSocket | `Session` | Persisted per (user, role, loop) |
| Reorder loops · set default view | WebSocket | `Session` | Persisted per (user, role) |
| Set / clear off console | WebSocket | `Session` | Never inferred |
| Edit push-to-talk bindings | WebSocket | `Session` | Persisted per user. Console only; there is no lobby settings surface |
| Create / edit a personal preset | WebSocket | `Session` | Silently narrowed to reach at use ([ADR-0013](../adr/0013-arming-is-independent-of-subscription.md)) |
| Key / unkey | WebSocket | `Session` | The arm set was already validated at arm time |
| Key priority | WebSocket | `Session` | Available to anyone holding `emit`. **Audited** on every press, no minimum duration |
| Clear a Cut on oneself | WebSocket | `Session` | The target clears their own ([ADR-0014](../adr/0014-authority-acts-on-emission-are-transient.md)) |
| Dismiss a directed subscription | WebSocket | `Session` | Droppable like any other |
| Report loop health | WebSocket | `Session` | Beacon counts ([ADR-0017](../adr/0017-loop-health-is-measured-not-asserted.md)) |
| Answer a takeover request | WebSocket | `Session` | Occupant of the targeted role only |
| mediasoup signalling | WebSocket | `Session` | Transport bound to the session at creation ([ADR-0026](../adr/0026-one-credential-and-the-media-path-carries-none.md)) |

## In a session, checked against the grid

| Operation | Transport | Requirement | Notes |
|---|---|---|---|
| Subscribe / unsubscribe | WebSocket | `Grid(monitor, loop)` | |
| Arm / disarm a loop | WebSocket | `Grid(emit, loop)` | Server-enforced ([ADR-0008](../adr/0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md)) |
| Hail | WebSocket | `Grid(emit, loop)` | Asks and cannot compel, which is why it is not on `control` ([ADR-0047](../adr/0047-a-hail-is-a-monitoring-directive-without-the-authority.md)) |
| Cut | WebSocket | `Grid(control, loop)` | Caller names the loop; server checks the target is emitting on it. **Audited** |
| Force a takeover | WebSocket | `Grid(control, loop)` | Caller names the loop; server checks the target role staffs it. **Audited** |
| Force a relinquish | WebSocket | `Grid(control, loop)` | Caller names the loop; server checks the target's role staffs it. **Audited** |
| Clear another user's Cut | WebSocket | `Grid(control, loop)` | Same loop rule as setting it. **Audited** |
| Issue / clear a monitoring directive | WebSocket | `Grid(control, loop)` for every loop named | **Audited** |

## System administration

All `SystemAdministration`, all HTTP, all **audited** with before and after plus the blast radius ([ADR-0015](../adr/0015-the-admin-console-reads-one-row-at-a-time.md)). Reachable from the lobby and from within a session, and never from a role.

| Operation | Notes |
|---|---|
| Create · read · edit · delete users | Deleting a user must not orphan their audit entries |
| Lock / unlock an account | System administration, distinct from forced relinquish ([ADR-0014](../adr/0014-authority-acts-on-emission-are-transient.md)) |
| Issue an enrolment code | It is a credential: expiring, single-use |
| Force a password reset | Ends the user's sign-in and session immediately |
| Create · read · edit · delete roles | Includes `max_occupants` |
| Create · read · edit · delete loops | New loops arrive `unreviewed` |
| Set a grid cell | The only place voice authority is configured ([ADR-0011](../adr/0011-a-permission-is-one-cell-on-the-grid.md)) |
| Dismiss an unreviewed cell | Records a deliberate `none` |
| Grant / revoke eligibility | Revocation ends occupancy immediately |
| Set the staffing-role flag per (role, loop) | Only where the role may emit on that loop |
| Create service principals, issue and revoke tokens, bind a role | Standing grant with no expiry ([ADR-0027](../adr/0027-a-service-principal-acts-through-a-role.md)) |
| Edit the pronunciation dictionary | One list for the deployment ([ADR-0030](../adr/0030-speech-synthesis-is-a-swappable-sidecar.md)) |
| Edit a role default | Subscription set, view, loop order. No live blast radius ([ADR-0052](../adr/0052-a-role-default-is-a-starting-point-never-a-floor.md)) |
| Edit the deployment loop order | ([ADR-0053](../adr/0053-the-loop-order-is-complete-and-a-new-loop-lands-at-the-end.md)) |
| Query the audit log | Filterable by actor and target. Reads are not themselves audited in v1 |
| Read subprocess, disk and backup health | The mediasoup worker and the text-to-speech sidecar ([ADR-0040](../adr/0040-one-binary-one-unit-four-moving-parts.md)) |

## Service principal

| Operation | Transport | Requirement | Notes |
|---|---|---|---|
| Announce | HTTP | `ServiceToken` + `Grid(emit, loop)` for **every** loop named | Bearer token in an `Authorization` header, never a query string. A loop the bound role cannot reach refuses the whole call. Carries a priority flag. The announcement is not audited; keying priority is ([ADR-0029](../adr/0029-an-announcement-is-an-ordinary-transmission.md)) |

The token reaches nothing else. It is refused on the socket upgrade and on every cookie route, and a request presenting both a cookie and a token is refused rather than resolved by precedence.

## Outside the model by design

| Surface | Notes |
|---|---|
| The on-box CLI | Creates an administrator, resets a password. Bypasses every check here. Shell access to the host is the highest privilege in the system ([ADR-0025](../adr/0025-credentials-are-administered-because-there-is-no-email.md)) |
| The first-start bootstrap code | Written to the server's own log, so log-read access is equivalent to administrator at that moment |

## Deliberately absent

| | Why |
|---|---|
| Reading or resetting another user's personalisation | Ruled out rather than left unbuilt ([ADR-0049](../adr/0049-the-role-is-the-profile.md)) |
| A per-user permission exception of any kind | No exception layer exists ([ADR-0011](../adr/0011-a-permission-is-one-cell-on-the-grid.md), [ADR-0014](../adr/0014-authority-acts-on-emission-are-transient.md)) |
| An access-request endpoint | Asking for reach is a conversation; the administrator's cell edit is the part VoxLoop does |
| A forceful announcement endpoint | Designed and dropped ([ADR-0029](../adr/0029-an-announcement-is-an-ordinary-transmission.md)) |
| A service-principal read endpoint | Loop names are configuration a script holds |
| Self-registration and self-service password reset | No mail path ([ADR-0025](../adr/0025-credentials-are-administered-because-there-is-no-email.md)) |
| A personalisation configuration endpoint | Written through as the state authority applies the live act ([ADR-0050](../adr/0050-personalisation-persists-what-is-safe-to-be-stale.md)) |

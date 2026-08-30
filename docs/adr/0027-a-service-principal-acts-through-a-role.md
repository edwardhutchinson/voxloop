# A service principal acts through a role, but never occupies one

[#7](https://github.com/edwardhutchinson/voxloop/issues/7) settled that the text-to-speech service is a **principal, not a role**. So authentication deals in two kinds of principal from day one: **users**, who are people, and **service principals**, which are not.

A service principal holds a **long-lived, administratively issued token**. It has no password, no enrolment code, no lobby and no interactive sign-in.

## Its authority is read off the grid, via a role it is bound to

The alternative — letting a service name loops directly — would be a second authority layer, and [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) permits exactly one. So a service principal is **bound to a role**: its reach is that role's row, and its transmissions are attributed to that role, exactly as a human's would be. An alarm announcement arrives on the loops `Ground Alarms` may emit to, attributed to `Ground Alarms`.

This keeps the grid the only place voice authority is ever configured, and it means an administrator revokes a service's reach the same way they revoke anyone's: by editing a cell.

## Reach, never occupancy

**A service principal's role binding gives it reach; it never makes the role occupied.** A loop is not `staffed` because a service is bound to a role that staffs it.

[ADR-0005](./0005-occupancy-means-listening-not-signed-in.md) defines staffing as somebody demonstrably hearing a loop — the question it answers is *"if I key up, will a human respond?"* A synthesiser cannot respond, and a loop reading `staffed` because a text-to-speech service exists is the exact misrepresentation that ADR was written to prevent. Service principals therefore have no session, no subscriptions, no presence and no staffing contribution. They emit, and that is all.

## Consequences

- **Roles bound to a service principal should be modelled as roles no human is eligible for**, so that "who occupies `Ground Alarms`" has the honest answer *nobody, ever*. A role that is sometimes human and sometimes synthetic would make attribution ambiguous at exactly the moment it matters.
- **The presence document must never count a service principal as an occupant**, in the lobby view or the operator console. This is one line of logic and a whole class of wrong staffing states.
- **A service token is a standing grant with no idle window and no expiry.** [ADR-0023](./0023-sign-in-is-to-the-application-and-a-role-is-assumed.md)'s 24-hour reaping applies to users in the lobby, which a service never enters. Rotation is therefore an administrative act with no automatic backstop, and issuing and revoking tokens belongs in the audit log.
- **What a service principal may actually call is [#11](https://github.com/edwardhutchinson/voxloop/issues/11)'s.** This ADR fixes only that it is not a user, does not enter through the grid on its own account, and cannot staff a loop. It also fixes that non-browser callers cannot use the cookie of [ADR-0026](./0026-one-credential-and-the-media-path-carries-none.md), so the API needs a token-bearing path that browsers never take.

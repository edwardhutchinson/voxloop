# Operational authority follows the role; system administration follows the user

"Administrator" is two capabilities that we deliberately keep apart.

**System administration** — creating users, roles and loops, granting eligibility, setting the (role, loop) matrix — is a capability of the *user*. It has to be, because somebody must configure the system before any role exists to confer it.

**Operational authority** — silencing an emitter mid-event, forcing a takeover of an occupied position, issuing monitoring requests — is conferred by the *role*. It is held by whoever is signed into a role that carries it, and it transfers at shift change along with the position itself.

Making both a property of the user would mean a shift lead keeps event-level power over the loops after handing over the position, and an IT administrator who configures the system acquires the ability to talk over a live pass. Making both a property of the role would mean nobody can configure a system that has no roles yet. Splitting them is what makes each one land in the right place, and it is consistent with [ADR-0002](./0002-permissions-attach-to-role-and-loop.md): operational capability follows the position, exactly as voice permission does.

## Consequences

- The person configuring VoxLoop and the person running the operation are different capabilities, and either may be held without the other.
- Scoped operational authority — a lead with power over only the roles they are responsible for — is not settled here and is left to the permission model.

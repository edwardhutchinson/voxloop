# The loop is the only destination for voice

Voice can be addressed to a loop and to nothing else. There is no user-to-user direct call and no way to address a role; private conversation happens on standing, pre-provisioned conference loops that operators move to ("move to Conference 3") rather than on channels created on demand.

The reason is [Patterson, Watts-Perotti and Woods, _Voice loops as coordination aids in space shuttle mission control_ (CSCW 8:353–371, 1999)](https://doi.org/10.1023/A:1008722214282), which argues explicitly against letting operators create channels on demand: doing so forces them to work out who to include and to negotiate membership *during* the event that prompted it, and it produces idiosyncratic channels whose membership nobody else knows. Their recommendation is standing conference loops that "are continuously monitored but lie unused until a situation arises" — one action, zero decisions, at the moment of highest load.

A single destination kind also keeps the rest of the system singular: one permission matrix, one selection grid on the console, one audio path, and one answerable question in "who can hear me".

## Consequences

- A 1:1 conversation, if it is ever needed, is modelled as an auto-provisioned pairwise loop rather than as a second addressing primitive. Adding users as destinations later would double both the permission model and the console UI, so the cost of deferring is low and the cost of adopting it now is not.
- "Is anyone behind this?" is answered through a loop's staffing role, not by addressing the role directly. See [ADR-0002](./0002-permissions-attach-to-role-and-loop.md).

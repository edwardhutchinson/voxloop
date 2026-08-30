# A loop with no staffing roles has no staffing state

> **Amends [ADR-0005](./0005-occupancy-means-listening-not-signed-in.md).** Staffing state has three values and one absence. A loop with no staffing roles configured does not read `vacant`; it has no staffing state at all, and the console shows nothing where the word goes.

Staffing state is computed from staffing roles. Where a loop has none, the computation has nothing to run on, and every available answer is a lie of some kind. `vacant` is the one it would fall to by default, and it is the worst of them: two people can be talking on that loop right now while the board card says nobody is behind it.

ADR-0005 exists because a loop reading `staffed` when nobody will hear you is the misrepresentation the product cannot afford. This is that misrepresentation reversed, and it is worse in one respect — the false `staffed` is at least transient, whereas a loop with no staffing roles reads `vacant` permanently and trains everyone to stop reading the field.

**Absence is derived, never configured.** Nothing new is set on a loop. The state is simply not computed where there is nothing to compute it from, which is the same shape as the rest of the model: staffing is a flag on the (role, loop) pair, and a loop with no such pairs has no staffing question to answer.

## Alternatives

**Mark every emit-capable role as a staffing role on such loops.** The card would then read `staffed` whenever anyone is there. Rejected because it quietly redefines `staffed` on those loops to mean *someone who could be here is here*, which is a weaker claim than the same word makes everywhere else, and ADR-0005's whole value is that the word means one thing.

**A kind of loop that opts out.** Rejected as a second concept bought to express something already derivable from configuration, and [ADR-0055](./0055-there-is-no-conference-loop.md) had just declined to add a loop kind for a larger reason than this one.

## Consequences

- **The board and the ledger need a rendering for absence.** Blank where the word goes, not a fourth word: `n/a` and `unstaffed` both read as states, and this is the absence of one. [ADR-0032](./0032-the-console-is-two-views-of-one-loop-list.md) keeps the two views consistent, so they render it the same way.
- **"Is anyone there" on such a loop is answered before keying, by the [transmit bar's](./0034-the-transmit-bar-is-always-visible-and-the-audience-is-a-count.md) audience count.** That is the right instrument and it was always the right instrument: staffing state answers *is someone covering this position*, and a loop with no position to cover was never in its scope.
- **Every loop starts here.** A newly created loop has no staffing roles until an administrator sets one, so it shows no staffing state rather than a discouraging `vacant` from the moment it exists. This sits alongside the [unreviewed loop](./0054-every-operation-declares-its-authorisation.md) prompt in the admin console: one says the permissions have not been ruled on, the other says nothing staffs it, and both are honest about being unconfigured rather than pretending to a value.
- **A loop can lose its staffing state.** Removing the last staffing role from a loop moves it from `staffed`, `away` or `vacant` to nothing. That is a legitimate configuration change, it applies to live sessions like any other ([ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md)), and the console must not treat the transition as an error.

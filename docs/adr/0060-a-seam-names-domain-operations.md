# A seam names domain operations, and VoxLoop is eleven modules

VoxLoop accumulated seven seams across fifty-nine ADRs, every one found incidentally by a ticket asking about something else, and until now nothing listed the parts of the system or said what a seam is supposed to guarantee. The enumeration lives at [`docs/spec/modules.md`](../spec/modules.md); this ADR is the rule behind its shape.

**A seam's interface names domain operations and domain types, and nothing on the caller's side can tell which adapter is behind it.**

Two ADRs had already reached this independently, about different things. [ADR-0038](./0038-sqlite-behind-domain-shaped-repositories.md) rejected a generic `query`/`execute` interface because the SQL dialect leaks through it anyway, choosing `set_cell`, `grant_eligibility` and `record_authority_act` instead. [ADR-0024](./0024-identity-is-a-replaceable-front-door.md) made the identity adapter's entire output a resolved internal user id, so nothing downstream learns how it was obtained. Stating the rule once means seam number eight is not invented from scratch by whoever writes it.

The test is mechanical: **if a type or an error crossing the interface names the mechanism, the seam has failed.** A `rusqlite::Row`, a `mediasoup::Producer`, an HTTP status leaving a module that is not Transport. `sqlx::Any` is the instructive failure because it looks like it *is* the seam and is not, which is why ADR-0038 named it rather than merely avoiding it.

## Eleven modules, and Audit is not one of them

Seven in the Rust binary: **Identity**, **Configuration**, **Authorisation**, **State authority**, **Media plane**, **Synthesis**, **Transport**. Four in the client bundle: **Input**, **Audio**, **Console**, **Session**.

**Audit was the obvious twelfth and it folds into Configuration.** [ADR-0038](./0038-sqlite-behind-domain-shaped-repositories.md) already forces a grid edit and its audit entry into one transaction, because an entry recording a blast radius that was never true is worse than none. Keeping Audit separate creates a module somebody can forget to call; folding it in makes writing the entry part of the write it records.

**These are modules inside two artefacts, not processes.** [ADR-0040](./0040-one-binary-one-unit-four-moving-parts.md)'s four moving parts are the deployment and nothing here adds to them.

## Consequences

- **Not every boundary is a seam, and the ones that are not should not pretend otherwise.** [ADR-0021](./0021-ptt-input-is-a-level-with-liveness.md) set the standard: a seam nothing has ever been swapped through is a seam that will not fit when somebody needs it. Input is the only seam in VoxLoop with genuine variation today. The rest are justified by a future swap or by a test double ([ADR-0064](./0064-tests-run-against-the-real-store.md)), and modules.md marks the ones with neither rather than presenting all seams as equals.
- **The rule is language-agnostic**, which is why the four client modules sit on the same list rather than in a document of their own.
- **The spec finally has a map.** An implementing agent reconstructing the system from sixty ADRs was the real problem [#26](https://github.com/edwardhutchinson/voxloop/issues/26) found, and modules.md should be the first thing anyone reads.

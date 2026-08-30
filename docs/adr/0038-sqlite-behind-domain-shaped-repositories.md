# SQLite behind domain-shaped repositories, with the transaction in the seam

Everything genuinely persistent in VoxLoop lives in **one SQLite database in WAL mode**, reached through **domain-shaped repository traits** rather than a generic database interface. Postgres is a plausible second implementation later; nothing about it is designed for now.

## The data is small and cold

The persistent set is users, external identities, password hashes, enrolment codes, roles and their `max_occupants`, loops, the (role, loop) permission grid, staffing-role flags, eligibility, unreviewed-loop markers, service principals and their tokens, the pronunciation dictionary, personalisation, and the audit log.

At the pilot's shape that is a grid of roughly 300 cells, eligibility in the low thousands of rows, and an audit log that grows by thousands of rows a *year* — because [ADR-0028](./0028-the-audit-log-records-decisions-not-traffic.md) records decisions and deliberately not traffic. Writes are administrative edits: single digits per day. Postgres would be a second daemon to install, configure, back up and patch inside a possibly air-gapped customer network, for a database measured in megabytes.

## The seam is domain-shaped, and `sqlx::Any` is ruled out by name

A generic database interface — `query`, `execute`, rows in and out — is the shape people reach for and it leaks. The SQL dialect bleeds through it anyway, so it delivers no portability while permanently constraining every query to what both backends share.

Instead the seam is **repository traits whose methods are domain operations and whose types are domain types**: `set_cell`, `grant_eligibility`, `record_authority_act`. Swapping to Postgres means writing a second implementation with dialect-native queries — and that is the *correct* amount of work, contained behind a boundary, rather than a rewrite that leaks into calling code. This is the posture [ADR-0024](./0024-identity-is-a-replaceable-front-door.md) already established for identity, where the adapter's entire output is a resolved internal user id and nothing downstream learns how it was obtained.

`sqlx::Any` — the runtime-polymorphic driver that lets one binary address either backend — is rejected explicitly, because it looks like it *is* the seam. It costs compile-time query checking and pins every query to the intersection of two dialects, which is the generic-interface failure arriving through the driver.

## The transaction is part of the seam, and the audit log shares the store

Committing a permission-grid edit is not one write. It reads the current cell, computes the **blast radius** over live sessions that [ADR-0015](./0015-the-admin-console-reads-one-row-at-a-time.md) requires at commit time, writes the cell, and writes an audit entry carrying before, after and that blast radius. A repository-per-aggregate design with a connection each would let the cell land without its audit entry, or the entry record a blast radius that was never true.

So a **transaction handle is passed into repository methods** rather than hidden behind them, and the blast radius — which comes from the *in-memory* side of the system ([ADR-0039](./0039-live-state-is-in-process-behind-one-state-authority.md)) — is computed and passed in as a value before the transaction opens. Neither seam has to know about the other.

The corollary is accepted deliberately: **the audit log lives in the same store as the configuration it audits.** A separate audit database would make that atomicity unbuildable, and an audit log that can silently miss the change it exists to record is worse than none.

## Consequences

- **Append-only is an application discipline, not a database guarantee.** SQLite will happily let anything with the file update an audit row. The property [ADR-0028](./0028-the-audit-log-records-decisions-not-traffic.md) promises is enforced by there being no code path that updates or deletes one, which means it must be *tested*, not merely intended.
- **The box is the database.** There is no replica and no network endpoint, so backup is a real deployment obligation rather than an assumed one, and it belongs in the packaging work rather than being discovered by a customer.
- **`sqlx` with the concrete SQLite driver**, offline query metadata committed to the repository so builds do not need a live database, and **migrations embedded in the binary and run at startup** — right for an appliance a customer upgrades by replacing a binary.
- **The binary refuses to start against a schema newer than it knows.** Without that, a rollback appears to succeed and then writes into a schema it misunderstands.
- **Personalisation persists; session state does not.** That is the boundary between this ADR and [ADR-0039](./0039-live-state-is-in-process-behind-one-state-authority.md), and it is the whole of it.

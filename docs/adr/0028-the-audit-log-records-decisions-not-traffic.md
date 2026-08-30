# The audit log records decisions, not traffic

VoxLoop ships an **append-only audit log in v1**, carrying three classes of event and no more:

- **Authentication events** — sign-in success and failure, sign-out, session end with its reason, enrolment, and password reset.
- **Configuration changes** — user, role and loop creation and deletion, eligibility grants and revocations, and permission cell changes, each with before and after, alongside the **blast radius** [ADR-0015](./0015-the-admin-console-reads-one-row-at-a-time.md) already has the server computing at commit time.
- **Operational authority acts** — Cut, forced takeover, monitoring directives.

The actor is the **internal user id**, and for authority acts the **role as well** — because [ADR-0003](./0003-operational-authority-follows-the-role.md) makes the role the source of that authority, so recording only the person would drop the half that explains why the act was permitted.

## Keying is deliberately excluded

*Who talked on Flight at 14:03* is genuinely valuable and it is **not** in the audit log. It is high-volume operational traffic rather than a decision about the system, and it belongs with recording — which [ADR-0009](./0009-recording-taps-plain-rtp-on-loopback.md) already gives a seam and which will need the timeline anyway. Mixing the two would bury a handful of permission edits per month under thousands of keying events per shift, which is how an audit log stops being read.

The boundary is stated here rather than left to drift, because both efforts could plausibly claim it and neither would notice the gap.

## It must be readable

The log is written to a queryable store and surfaced in the admin console, **filterable by actor and by target**. An audit log nobody can query is a compliance artefact, not a tool.

The cautionary tale is in this project's own research: openvocs' admin API — `get_clients`, `broadcast`, `start_record` — performs no authorisation check at all, while the database-admin API beside it checks properly. That gap survived because it was implicit. Auditing is the same kind of thing, and gets missed the same way.

## Consequences

- **Retention, export and tamper-evidence are not settled here.** A regulated customer will ask for all three; v1 commits only to append-only and queryable. That is a known follow-up, not an oversight.
- **The audit log outlives the records it references.** Deleting a user must not orphan or erase their audit entries, so entries store the internal id *and* a snapshot of the name as it stood — the id keeps it correct, the snapshot keeps it readable.
- **Reading the audit log is itself a system-administration capability**, and it shows who cut whose transmission. It is not an operator-console feature.
- **Failed sign-ins are recorded but never lock the account** ([ADR-0025](./0025-credentials-are-administered-because-there-is-no-email.md)). The log is where a brute-force attempt becomes visible, which is the compensating control for choosing rate-limiting over auto-lock.

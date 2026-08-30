# Composed text is a decision; captured audio is traffic

[ADR-0029](./0029-an-announcement-is-an-ordinary-transmission.md) keeps announcements out of the audit log: an announcement is a transmission, and [ADR-0028](./0028-the-audit-log-records-decisions-not-traffic.md) puts traffic in recording's hands. Every other `SystemAdministration` operation in [`api-surface.md`](../spec/api-surface.md) is audited. The console announcement of [ADR-0066](./0066-the-console-announces-as-the-designated-principal.md) is both, and this settles which wins.

**A console announcement is audited. A token-path announcement is not.** The asymmetry is principled rather than a fudge, and it rests on two things.

ADR-0028 excluded keying on **volume** — thousands of events per shift burying a handful of permission edits is how an audit log stops being read. A human typing sentences is a handful per shift at most, so that argument never fires here.

And the token path's *decision* was **issuing the token**, which ADR-0027 already audits. Every announce after that is a script doing what it was authorised to do, in bulk, unattended. The console path has no per-use grant, so **the use is the decision**.

## The reason it has to be in the log at all

This is the only place in VoxLoop where the actor and the attribution come apart on purpose. The ops floor hears the bound role. [ADR-0033](./0033-the-console-shows-that-someone-is-talking-never-who.md) keeps the operator console from naming anyone, [ADR-0029](./0029-an-announcement-is-an-ordinary-transmission.md) attributes the transmission to the role, and [ADR-0066](./0066-the-console-announces-as-the-designated-principal.md) has the server acting as somebody else by design. **The administrator who caused it appears in no surface anywhere.** If the audit log does not hold it, nothing does.

## The text goes in the entry

The entry carries the **text**, the acting user, the bound role, the loop list, the priority flag, the timestamp and the clip duration.

Recording is a seam in v1 rather than a feature ([ADR-0009](./0009-recording-taps-plain-rtp-on-loopback.md)), so an entry without the text would let v1 say *an administrator announced on Flight and Ground at 14:03* and never say what was announced. The obvious objection is that content belongs to recording, and the line that answers it is the title of this ADR:

**An administrator typed that sentence and chose to put it in a synthetic voice. That is not the same kind of thing as a microphone being open.** Composed text is authored, deliberate, small and low-volume. Captured audio is none of those. The boundary ADR-0028 drew was decisions against traffic, and composition falls on the decisions side of it.

## Consequences

- **This is the only content anywhere in the audit log**, and somebody will cite it later to argue for logging more. The composed-against-captured line is what refuses that, and it is written here so the refusal has somewhere to point.
- **The log now holds text a customer may consider operational content**, which lands on the open retention, export and tamper-evidence questions ADR-0028 left for a compliance posture nobody has established yet. It sharpens them rather than answering them.
- **The audit entry outlives the announcement in every case**, since v1 has no recording. For this one path the log is the record, which is the reverse of the arrangement everywhere else in the product.

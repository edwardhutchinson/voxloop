# Resynchronisation narrates the gap

[ADR-0019](./0019-presence-is-one-versioned-document-scoped-to-reach.md) collapses resynchronisation to *"here is version N"* — one versioned document, rendered atomically, describing the world **now**.

That is necessary and it is not sufficient, because current state cannot express a change to state. The clearest case is the one [ADR-0018](./0018-no-signalling-channel-means-no-emission-path.md) already flagged: a **latch that was dropped** looks identical to a latch that was never set. ADR-0018 required the client to announce that one locally, from its own knowledge, because the server could not reach it at the time. On resume the server *can* reach it — and the same argument covers four more things that happened *to* this operator while they were dark.

**So the first document after a resume carries a bounded set of gap events.** ADR-0018's local announcement is the special case; this is the general one.

## A bounded set, not a diff

The events are only things done to the operator that the current state cannot reveal:

- a latched emission was dropped
- the operator was **cut** by operational authority ([ADR-0014](./0014-authority-acts-on-emission-are-transient.md))
- permissions changed underneath them, removing arms or subscriptions
- a **monitoring directive** added loops ([ADR-0035](./0035-a-monitoring-directive-promotes-a-loop-it-does-not-police-it.md))
- the role was force-relinquished

**A full diff was rejected.** The document already carries current state, so a diff would restate it and bury the five things that matter under everything that merely moved. It would also cost what ADR-0019 and [ADR-0039](./0039-live-state-is-in-process-behind-one-state-authority.md) deliberately refuse to pay: the presence document is a **projection the state authority computes rather than a record it keeps**, so there is no history to diff against, and manufacturing one would mean storing documents in order to describe them.

That also settles the resume wire format with nothing to decide: **the server always sends a full document**, and the client's last-seen version is useful only for detecting that the server restarted ahead of ADR-0041's instance id.

## Where they live and how long

Gap events are sentences, and [ADR-0032](./0032-the-console-is-two-views-of-one-loop-list.md) holds that a card carries a word — so they are not per-card, and they are not per-loop in the first place.

They render as a **dismissible banner in both console views**, in the slot ADR-0018 already defined for its stale-state banner, and they **persist until dismissed**. Auto-expiry was rejected by name: a dropped latch that faded out unread is precisely the failure ADR-0018 called out — an operator who believes they are still transmitting, arriving through the other door.

Repeated gaps **coalesce rather than stack**. A flapping VPN produces nine outages in a minute, and stacking would bury the one cut-by-authority under eight copies of a rebuilt transport.

## Consequences

- **The state authority tracks per-session what has happened since that session last had a socket**, which is the one piece of gap history the system keeps. It is bounded by the event list above, so it does not become a log by accretion.
- **Cut-by-authority appears twice**, in the audit log as a decision ([ADR-0028](./0028-the-audit-log-records-decisions-not-traffic.md)) and here as something the target must be told. Neither is redundant: one is the record, the other is the notification.
- **This is the only channel carrying how long the outage was.** [ADR-0043](./0043-a-resume-restores-everything-except-the-key.md) restores identically regardless of duration, so if the banner is ever weakened both decisions move together.
- **A dismissal is a deliberate act**, so it counts as evidence under ADR-0016 and refreshes last-active — unlike the reconnect that produced it.

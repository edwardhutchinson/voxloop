# Priority is keyed, not held

Priority is a **transient act**, not an attribute of a person, a role, a grid cell or a loop. It is conferred by a third momentary input binding — a user holds the priority key and their transmission is a priority transmission for exactly as long as they hold it. It is available to anyone holding `emit` on the loops they are armed on, and every press is audited.

[ADR-0045](./0045-priority-defeats-attenuation-and-nothing-else.md) settles what priority *does*. This settles who may do it and how.

**A flag on the loop was the strong alternative and it lost on the wrong kind of coarseness.** Making some loops priority loops — everything said on them at full gain — would have had the grid do all the access control for free, since "who may speak with priority" collapses into "who holds `emit` on that loop", and it would have avoided inventing a second authority axis, the same move [ADR-0012](./0012-operational-authority-is-the-control-rung.md) made for takeover. It also changes nothing at fire time, so there would be nothing to explain when it fires. It was rejected because it moves the answer from a *moment* to a *place*: urgency is a property of what you are saying, not of where you are saying it, and a priority loop makes routine chatter on that loop full-gain too. The decisive case is the announcement — under a loop flag, an urgent announcement needs a dedicated loop to land on; under a key, the injection endpoint carries a flag per announcement.

**A flag on the (role, loop) cell is rejected** because it breaks [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md)'s "one value and nothing else", turning each cell into a rung plus a bit. That doubles what a row means immediately after [ADR-0015](./0015-the-admin-console-reads-one-row-at-a-time.md) found that reading a row is how administrators actually reason.

**Deriving it from `control` is rejected** for two independent reasons. The person who spots the urgent thing is usually not the lead — it is the specialist watching the anomaly — so gating on authority puts the mechanism in the hands of the one person least likely to need it. And it breaks announcements outright: [ADR-0027](./0027-a-service-principal-acts-through-a-role.md) has the service principal acting through a role, so a priority announcement would require handing the text-to-speech service `control`, and with it the authority to cut people.

## Ungated, and audited instead

**Anyone holding `emit` may key priority.** This is consistent with VoxLoop's posture everywhere else: [ADR-0034](./0034-the-transmit-bar-is-always-visible-and-the-audience-is-a-count.md) refuses to let the console overrule an operator about their own operation, [ADR-0014](./0014-authority-acts-on-emission-are-transient.md) refuses punitive levers, [ADR-0035](./0035-a-monitoring-directive-promotes-a-loop-it-does-not-police-it.md) accepts that hearing cannot be compelled. It is also the weakest override in the system — it defeats attenuation only, never mute and never reach — so the worst an abuser achieves is being louder than someone wanted, and the escalation for that already exists in Cut.

**Every press produces an audit entry**, with no minimum-duration threshold: the acting user, the role, the armed loop set at the moment of the press, the timestamp and the duration. A 200 ms fumble is still a decision that momentarily overrode everyone's volume settings, and filtering belongs to the reader rather than the writer — a log that silently drops short entries cannot answer *was this abused*, because abuse may well look like a hundred short presses. This is [ADR-0028](./0028-the-audit-log-records-decisions-not-traffic.md)'s decisions-not-traffic rule applied straightforwardly.

## The binding

**A third independent binding, momentary only, and priority never latches.** Default ``Ctrl+` ``, keeping the family of [ADR-0021](./0021-ptt-input-is-a-level-with-liveness.md)'s `` ` `` and [ADR-0022](./0022-latch-is-never-derived-from-a-momentary-press.md)'s ``Shift+` ``, and user-configurable like them.

A modifier held alongside the ordinary push-to-talk key was the obvious cheaper option and is rejected on ADR-0021's own reasoning: a modifier is derived state, and derived state loses releases. [ADR-0022](./0022-latch-is-never-derived-from-a-momentary-press.md)'s argument against deriving latch applies here with more force, because a stuck priority control does not merely leave one mic open — it overrides *everyone's* volume settings for as long as it is stuck. Momentary-only means the override lives exactly as long as a finger.

**The level model absorbs this with no special case**, which is a good sign ADR-0021 chose the right abstraction. Priority is a second level channel alongside the ordinary one:

- `emitting = ordinary-level OR priority-level`
- `is-priority = priority-level`

Holding both the ordinary key and the priority key is therefore **one transmission at priority**, not two streams — consistent with [ADR-0007](./0007-the-client-emits-one-stream.md), where the client emits one stream regardless. Pressing priority while latched raises the priority level without touching the latch, so releasing it returns to a latched, ordinary transmission with nothing to restore. Pressing priority from cold both keys and elevates, so releasing it ends the transmission. Both fall out of the same two lines.

## Because it changes at fire time, it must be visible

A loop flag would have been standing state on a card. A key is not, so audio changes level with nothing on screen having changed first, and the map's rule is that displayed state is factual.

⚠️ **Re-grounded by [ADR-0059](./0059-a-priority-transmission-is-marked-wherever-it-lands.md):** the mechanism below is unchanged, but the mark is a declaration rather than an explanation of gain, so it renders on loops the receiver has at full volume and on loops they have muted, and it has no minimum display time.

**The talking indicator gets a priority variant, and that is the entire answer** — no banner, no toast, no message telling a subscriber their volume was overridden. This stays inside [ADR-0033](./0033-the-console-shows-that-someone-is-talking-never-who.md), because it says what *kind* of transmission is on the loop and still never whose, and inside [ADR-0032](./0032-the-console-is-two-views-of-one-loop-list.md), because it is a word on a card. It explains the gain change while the gain change is happening, which is the only moment it matters. On the emitting side, ADR-0034 already requires the key control to render live in both views, so an elevated latch shows as elevated with no new surface.

## Announcements

**The event-injection endpoint carries a priority flag, set per announcement by the caller.** This discharges the audibility question [ADR-0029](./0029-an-announcement-is-an-ordinary-transmission.md) handed to [#19](https://github.com/edwardhutchinson/voxloop/issues/19). Announcements remain ordinary transmissions in every other respect — they still duck nothing, still override nothing, still run for the length of their audio — and an urgent one is now distinguishable from a routine one by the property that actually differs, rather than by which loop it was routed to.

**The priority is audited even though the announcement is not.** ADR-0029 deliberately keeps announcements out of the audit log as traffic rather than decisions; keying priority is a decision under the rule above. What lands in the log is that a priority transmission occurred, attributed to the bound role, which is what ADR-0028 wants and does not reopen ADR-0029.

⚠️ **On the admin console's announcement path, one entry covers both** ([ADR-0067](./0067-composed-text-is-a-decision.md)). The announcement is audited there too, so there is no separate priority record to make — the flag is set on the same call rather than keyed as its own act, and the entry names the acting administrator alongside the bound role.

## Consequences

- **A third global hotkey is a third chance to fail registration.** [ADR-0020](./0020-the-browser-is-the-client.md) established that Windows `RegisterHotKey` is exclusive and **fails rather than warns** when a combination is claimed. This binding is also the one an operator is least likely to have exercised before the moment they need it, which makes binding-time conflict detection more load-bearing than it already was.
- **A priority key held across an outage is suppressed until released**, exactly like any other key ([ADR-0043](./0043-a-resume-restores-everything-except-the-key.md)). It needs no rule of its own.
- **An ungated priority key can degrade to nothing.** If everyone uses it always, priority means nothing and the design's only answers are social pressure, the audit trail and Cut. This is accepted knowingly; anyone wanting to gate it should reopen this ADR rather than add a grid dimension quietly.
- **Presets and priority compose without a new rule.** [ADR-0013](./0013-arming-is-independent-of-subscription.md) left open whether a preset's emission ducks or interrupts. It does not duck, because nothing does; it may be keyed with priority like any other emission, and the priority then applies to the preset's whole replaced arm set per [ADR-0045](./0045-priority-defeats-attenuation-and-nothing-else.md).
- **The presence document gains a field.** Whether a loop's current transmission is priority is server-pushed state ([ADR-0019](./0019-presence-is-one-versioned-document-scoped-to-reach.md)) like everything else the console draws, and the client keys it and signals it exactly as ADR-0008 has it signal keying.

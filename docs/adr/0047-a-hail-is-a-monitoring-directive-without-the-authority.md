# A hail is a monitoring directive without the authority

Any operator may **hail** a role — or a named person currently occupying one — to a loop the hailer holds `emit` on. The hail puts that loop on the target's console as a directed subscription marked with who asked and why. It grants nothing, compels nothing, is dismissed with one click, and gets no reply.

This is deliberately **not a new mechanism**. [ADR-0004](./0004-monitoring-directives-are-enforced-and-additive.md) built something that puts a loop on someone else's console, and [ADR-0035](./0035-a-monitoring-directive-promotes-a-loop-it-does-not-police-it.md) then took the enforcement out of it, leaving it *promoting* a loop the operator may drop, mute or reorder at will. What ADR-0035 did not do was revisit the `control` gate, which had been justified by exactly the power it had just removed. So "does v1 have operator-to-operator nudges" turned out to be a question about a gate on a mechanism that already existed: both motivating examples — *come listen to me*, *join this loop* — are literally what a directive does, and the only thing an ordinary operator lacked was permission to do it.

**A second, message-shaped mechanism was the alternative and it lost on more than cost.** A transient "Payload asks you on Conference 3" delivered as its own thing would have been the first pure event in the product, needing its own delivery path and its own answer for a disconnected session, and it would have sat beside a mechanism doing nine tenths of the same job — the second-mechanism objection [ADR-0011](./0011-a-permission-is-one-cell-on-the-grid.md) exists to raise. Reusing the directive also dissolves the [ADR-0016](./0016-displayed-state-is-observed-or-asserted.md) problem the question started with: a hail's *effect* is a subscription, which is ordinary observed state the presence document already carries and the console already knows how to render. Only the accompanying sentence is transient, and the console already has somewhere to put a sentence.

## Two acts, one result

A hail and a monitoring directive put the same marked loop on the same console. They are nonetheless different acts, and the marking says which one it was:

|            | Monitoring directive           | Hail                          |
| ---------- | ------------------------------ | ----------------------------- |
| Who may    | `control` on the loop          | `emit` on the loop            |
| Lifetime   | Stands until cleared           | One-shot                      |
| Reaches    | Whoever holds the seat, now or later | Whoever holds the seat at that moment |
| Audited    | Yes — an operational authority act | No                        |

The gate is `emit` rather than `monitor` because a hail means *come hear me*. Allowing it on `monitor` would have added "go listen to Conference 3, something is happening there" — a thin case that turns hailing into general-purpose redirection of colleagues by people with no stake in the destination.

It is not audited because it is not an authority act: it changes no configuration, grants no reach, and is shed with one click. [ADR-0028](./0028-the-audit-log-records-decisions-not-traffic.md) draws the line at decisions about the system, and a request between colleagues is on the other side of it.

## Consequences

- **A hail reaches a seat, not a person.** Targeting a named person is targeting the session they are in right now. If the seat they are in cannot hear the destination loop, they cannot be hailed to it at all — there is deliberately no reasoning about other roles they are eligible for, and no way to hail someone into a different seat. A user in the lobby has no console to deliver to and is not hailable.
- **A vacant role cannot be hailed, and v1 has no way to ask for one to be staffed.** *"Somebody go and be the Thermal Engineer"* is a staffing request, a different feature with a different audience — it would have to reach the lobby — and it is out of scope. Only *"the person in that seat, come here"* is in v1.
- **A hail cannot defeat a mute.** If the target has muted the destination loop, they get the sentence and no audio change. This is the same gap as an announcement defeated by mute ([ADR-0029](./0029-an-announcement-is-an-ordinary-transmission.md)) and a directive defeated by mute (ADR-0035), and it is also the case that most justifies hailing in the first place: the sentence is how you reach someone the audio cannot.
- **If the target already subscribes, the hail is purely the sentence.** Nothing changes on their board except the banner saying you want them, which is correct — there is nothing to add.
- **The sentence renders in the banner slot [ADR-0044](./0044-resynchronisation-narrates-the-gap.md) already defines**, persisting until dismissed, with no sound, no modal and no focus steal. Anything louder would make a colleague's request more intrusive than a priority transmission ([ADR-0045](./0045-priority-defeats-attenuation-and-nothing-else.md)), which is backwards. The cost is honest: a banner and a new card among twenty are missable, so a hail carries no guarantee of attention and never claimed one.
- **A hail to a `disconnected` session lands on resume**, carrying its age, as an ordinary gap event. The reconnection window bounds the staleness at 120 seconds ([ADR-0041](./0041-a-session-is-resumed-by-name.md)); past that the seat is empty and there is nothing to deliver to.
- **Repeat hails coalesce rather than stack**, per (hailer, target), which is the rate limit. Hailing is ungated among operators and unaudited, so coalescing is the only thing standing between the feature and a spam vector — hammering the control achieves nothing, and there is no threshold to tune. The residual risk is accepted on the same grounds [ADR-0046](./0046-priority-is-keyed-not-held.md) accepted its own: a hail grants nothing, and forty colleagues in one room is not where this fails first.
- **There is no reply channel** — no *on my way*, no *busy*. The feedback already exists where it is actionable: the hailer is about to speak on that loop, and the transmit bar's audience count ([ADR-0034](./0034-the-transmit-bar-is-always-visible-and-the-audience-is-a-count.md)) shows the target arrive. A reply would be VoxLoop's first person-to-person message, a far larger door than this opens, and it repeats the read receipt ADR-0035 already retired for the same reason.
- **[ADR-0001](./0001-the-loop-is-the-only-destination.md) is untouched.** It governs *voice*, and a hail carries none — the takeover request was already a person-directed notification, so this is the second, not the first.

# A session is resumed by name, and reaped when the window expires

[ADR-0023](./0023-sign-in-is-to-the-application-and-a-role-is-assumed.md) settled that losing the signalling channel does **not** end occupancy — otherwise every VPN blip would cost an operator their subscriptions, arms and staffing — and left the length of the reconnection window and what it restores to this ticket. [ADR-0018](./0018-no-signalling-channel-means-no-emission-path.md) left the ladder's numbers here too.

## The cookie names the user; the session needs its own name

[ADR-0026](./0026-one-credential-and-the-media-path-carries-none.md) authenticates the signalling WebSocket from the sign-in cookie. That cookie identifies the **user**, and a reconnecting client has to say which *session* it is resuming.

Reattaching on the cookie alone was the simpler option and it is wrong, because ADR-0023 explicitly permits a user to be signed in on several machines. Under cookie-only reattachment, opening VoxLoop in a second tab or on a phone silently steals the live console's socket — an ordinary act with a catastrophic result.

So **`assume` mints a session id**, which the client keeps in the tab's `sessionStorage` and presents on the WebSocket hello. The server reattaches only if that session exists and belongs to the cookie's user.

This does not reopen ADR-0026's one-credential rule. The session id is not a credential: it is presented over a channel the cookie has *already* authenticated, and it can only ever select among that user's own sessions. Holding someone else's session id buys nothing.

Keeping it in `sessionStorage` rather than memory is deliberate: it is per-tab, and it survives a page reload. **F5 is therefore a resume with a media rebuild rather than a lost console**, which is worth having on a machine an operator may sit at for twelve hours.

## Reattachment is exclusive, and eviction is terminal

A successful reattach **evicts** any socket already holding that session, telling it why. This is the same posture ADR-0023 already takes when a role is assumed elsewhere.

It needs a matching rule on the client, because Chrome copies `sessionStorage` on *Duplicate Tab*: two tabs holding one session id would evict each other indefinitely, flapping the operator's audio every few seconds. **The retry policy therefore keys on why the socket closed, not on the fact that it closed.** A transport-level failure auto-reconnects; a server-sent eviction is terminal and the tab sits in a local end state until a human acts. Only one of the two tabs is ever retrying, so the ping-pong cannot start.

The close reason is consequently on the wire, not inferred.

## The timers

| | | |
|---|---|---|
| heartbeat | 2s | |
| `unconfirmed` | 5s | state frozen and marked stale, PTT still live |
| latched emission dropped | 2s of `unconfirmed` | fixed by ADR-0018, not here |
| `disconnected` | 12s | PTT disabled, fan-out closed |
| reconnection window | 120s | session held, then reaped |

All four are **startup settings**, read once with the rest of the configuration, and not hot-reloadable. ADR-0018 calls the disconnect threshold a safety parameter that must be tuned against the pilot's VPN, so it cannot be a compile-time constant — but it carries a **hard ceiling**, because an unbounded knob lets a site set it to five minutes and reintroduce exactly the hot mic ADR-0018 removed.

The 120s window is the contentious number. It means a single-occupant `Flight Director` is held by an unreachable ghost for two minutes. That is accepted: the role shows its occupant as unreachable with a running age, and ADR-0023's forced relinquish is the escape when someone genuinely needs the position now.

## An unrecognised session id is explained, not merely refused

A returning client can be refused for four quite different reasons — reaped at expiry, ended by an assume elsewhere, ended by forced relinquish or revoked eligibility, or the server restarted ([ADR-0039](./0039-live-state-is-in-process-behind-one-state-authority.md)) and remembers nothing. Collapsing all four into *"your session ended"* is [ADR-0016](./0016-displayed-state-is-observed-or-asserted.md)'s problem in miniature.

Two cheap mechanisms cover it. The state authority keeps **session tombstones** — id, end reason, timestamp — evicted after about fifteen minutes, so the lobby can say *"`Flight Director` was assumed on another machine at 14:02"*. And the WebSocket hello carries the **server's instance id and start time**, which covers the case a tombstone structurally cannot: after a restart every session id is unknown, and a client comparing instance ids can report **"the server restarted"** rather than implying this operator specifically timed out. ADR-0039 notes that a restart is indistinguishable from total network loss *at the time*; this makes it distinguishable afterwards.

## Expiry is a relinquish

When the window expires the session is reaped exactly as a relinquish: loops drop, the role frees, and an audit entry records the reason. The user remains **signed in** — ADR-0026's cookie is untouched — so a client reconnecting later lands in the **lobby** with the tombstone's explanation.

**It never auto-assumes.** Doing so would silently reopen an audio path the operator is not watching, and could re-occupy a role someone has taken in the meantime.

Per [ADR-0028](./0028-the-audit-log-records-decisions-not-traffic.md), session start and session end are audited **with their reason**; individual reconnects are not, because a VPN blip is not a decision.

## What the rest of the room sees during the gap

A blip must never read `vacant`. Occupancy survives, so someone still holds the staffing role, and `vacant` would be false.

Loops the disconnected operator staffs drop to **`away` at `disconnected`, not at `unconfirmed`** — reason *unreachable*, with the age. `unconfirmed` means precisely *we cannot confirm*, which is not yet a fact about staffing, and flipping the whole room's board on a three-second reroute is worse than a twelve-second lag.

One trap is closed here explicitly. [ADR-0017](./0017-loop-health-is-measured-not-asserted.md)'s beacon health is **counted by the client and reported over the WebSocket**, so a signalling loss stops beacon reports as a side effect. Connection state wins and beacon loss is suppressed while the channel is down, or one failure would arrive as two competing reasons for the same `away`.

## Consequences

- **`assume` now returns something the client must keep.** The session id is the first piece of state the client holds that is not a projection of the presence document.
- **The lobby renders tombstones.** ADR-0023 already made the lobby a rendering target; this adds an explanation to it, with a fifteen-minute memory after which the honest answer is the generic one.
- **A reaped session's loops go `away` → `vacant` at the 120s mark**, which is a state change other operators see with no act behind it. It is correct, and it is the moment a colleague learns the position is genuinely free.
- **The tombstone map is live state**, so ADR-0039 applies to it: it does not survive a restart, which is exactly the case the instance id covers instead.

# Sign in to the application; assume a role

Entering VoxLoop is two acts with two lifetimes, and this ADR names them, orders them, and says how long each lasts.

**Sign in** authenticates a principal to the application as a whole. It confers no role, no reach and no audio. **Assume** takes up a role, creating the session that carries voice. Relinquishing the role returns you to the **lobby** — signed in, roleless — and signing out ends everything.

## The naming is deliberately inverted from where the glossary started

`CONTEXT.md` originally used *sign-in* for taking up a role and reserved *log in* for authentication. That is now reversed, because [ADR-0003](./0003-operational-authority-follows-the-role.md) requires a user who authenticates and **never** holds a role — the IT administrator who configures the system and must not be able to talk over a live pass. Under the old naming that person is "logged in but not signed in", which reads as a half-broken state rather than the perfectly ordinary one it is. Under the new naming they are simply signed in, in the lobby, doing their job.

Every ADR before this one says "sign into a role" where it now means **assume a role**. Those are historical records and are left as written.

## The lobby is read-only and narrow

A signed-in user with no role sees: the roles they are eligible for, who occupies each, and the **staffing state of the loops those roles staff**. No audio, no authority, no talking indicators.

Occupancy alone would not have been enough. [ADR-0005](./0005-occupancy-means-listening-not-signed-in.md) made occupancy and staffing deliberately different things, so *"Flight has an occupant"* does not mean Flight is being covered — and `away` on a loop you are eligible to staff is exactly the signal that should pull you into a role. That is the question the lobby exists to answer, so it is the question the lobby is scoped to.

Live talking indicators were considered and dropped. They are churn shown to somebody who can neither hear them nor act on them, and they are the only part carrying within-loop detail — so they strain [ADR-0019](./0019-presence-is-one-versioned-document-scoped-to-reach.md)'s scoping for no operational return.

This does read across roles, which [#20](https://github.com/edwardhutchinson/voxloop/issues/20) rejected for the operating console. The distinction is that #20 rejected the union because **it displays authority nobody can hold**; in the lobby the user holds no authority and can do nothing, so there is no authority being misrepresented. The lobby uses the same presence-document machinery, versioned and rendered atomically, merely scoped differently — building a second way to render system state would mean building ADR-0016's freeze-and-stale-mark behaviour twice.

## Lifetime: the clock only runs in the lobby

There is **no idle timeout on a session and no absolute cap on a sign-in.** A sign-in ends after **24 hours with no deliberate act** — and that clock only runs while the user is in the lobby. Assuming a role stops it.

Short idle timeouts were already refused by [ADR-0016](./0016-displayed-state-is-observed-or-asserted.md), on the grounds that an operator watching telemetry is idle and very much on console; a timeout that signs them out is the same mistake by another route. "Deliberate act" is not redefined here — it is the notion ADR-0016 already uses when it shows how long ago a claimant last did anything deliberate.

The clock stopping under an assumed role follows from what the window is actually for: reaping abandoned sign-ins. An occupied role is by definition not abandoned — occupancy is visible to everyone, and an unattended one already surfaces as `away` through machinery built for exactly that. Without this rule, an operator holding `Flight Director` through a thirty-hour incident is signed out mid-event for not clicking anything.

An absolute cap was proposed and rejected. Its case was attribution: with no cap, a console occupied indefinitely never re-authenticates, so the *user* behind a role can go stale and shift change becomes inheritance rather than an act. **That consequence is accepted rather than solved.** Attribution of voice still holds — every emission is attributed and audited — and *who is physically at that console* is answered by occupancy, never by the credential.

## What ends what

| Event | Role occupancy | Sign-in |
|---|---|---|
| Relinquish role | ends | survives → lobby |
| Assume a role elsewhere | previous ends | both survive |
| Eligibility revoked | ends immediately, reason shown | survives → lobby |
| Forced relinquish (operational authority) | ends | survives → lobby |
| Account lock (system administration) | ends | ends |
| Password reset by an administrator | ends | ends |
| 24h with no deliberate act | n/a — clock runs only in the lobby | ends |
| `disconnected` ([ADR-0018](./0018-no-signalling-channel-means-no-emission-path.md)) | **survives** | survives |

The last row is load-bearing. Losing the signalling channel withdraws the emission path but does **not** end occupancy, or every VPN blip would cost an operator their subscriptions, arms and staffing. The session persists server-side for a reconnection window; **its length and what it restores belong to [#18](https://github.com/edwardhutchinson/voxloop/issues/18)**, stated here so neither ticket assumes the other owns it. *(Answered by [ADR-0041](./0041-a-session-is-resumed-by-name.md) — 120s, resumed by name — and [ADR-0043](./0043-a-resume-restores-everything-except-the-key.md).)*

## Consequences

- **Changing role is a relinquish followed by an assume, and the console says so.** Reach changes, and the presence document is scoped to reach, so subscriptions, arms, mutes and staffing all belong to the old role and none may survive. Audio genuinely stops; dressing that as a smooth transition is the class of lie ADR-0016 exists to prevent. No re-authentication is required, since eligibility is unconditional.
- **Sign-ins are unlimited; role occupancy stays one per user.** A sysadmin may hold the admin console on a laptop while occupying `Flight Director` at a console. Attribution is untouched, because it flows through the role and there is still only one.
- **The admin console is reachable from the lobby *and* from within a session**, gated on the user's system-administration flag and never on a role. This is the concrete payoff of ADR-0003's split: an operator who is also a sysadmin must not have to drop off the air to add a loop.
- **The lobby is a rendering target, not just a state.** It needs the presence document scoped to eligibility rather than to reach — a third scoping rule alongside ADR-0019's, and the only place reach is composed across roles.

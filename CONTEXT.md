# VoxLoop

A software voice loop system for operations centres: people listen to and speak to each other over administered audio conferences. Spacecraft operations is the guide use case, but nothing in the model is specific to it.

## Language

### Core entities

**User**:
An account belonging to a person. A user is never a destination for voice and never carries operational permissions of their own — both attach to the role they are signed into.
_Avoid_: entity, account, operator, client

**Role**:
A staffable position that users sign into, such as `Flight Director` or `Support Engineer`. It is the carrier of permissions and console layout, and it is a slot users step into — not a group of users.
_Avoid_: position, station, group, team, entity

**Loop**:
An audio conference: a many-to-many bus that can be monitored and emitted to. A loop is never a person and never a role, and it is the only thing voice can be addressed to.
_Avoid_: channel, group, conference, net, entity

### Occupancy

**Sign-in**:
The explicit act by which a user takes up a role, making the role occupied. Occupancy is only ever established this way — never inferred from membership or presence.
_Avoid_: log in (which is authentication, a separate act), check in

**Max occupants**:
The limit on how many users may be signed into a role at once. Single-occupant and multi-occupant roles are the same concept under different limits, not different kinds of thing.
_Avoid_: cardinality, capacity, seats

**Staffing role**:
A role marked as counting toward a particular loop's occupancy. It is set per (role, loop), so a loop may have several staffing roles or none, and it can only be set where the role may emit on that loop — a role that cannot answer cannot staff.
_Avoid_: owner, host, primary role

**Occupancy**:
Whether anyone is behind a loop: `staffed` (an occupant of one of its staffing roles is subscribed to it, available, and not muting it), `away` (such occupants exist but all have declared themselves off console or muted the loop) or `vacant` (neither). Occupancy asks who is actually hearing the loop, not who is merely signed in.
_Avoid_: online/offline, presence, availability

**Session**:
A user's single live connection, bound to exactly one role. A user has at most one at a time; signing in elsewhere ends the previous session and tells it why.
_Avoid_: connection, client, device, login

**Eligibility**:
The unconditional grant permitting a user to sign into a role. It carries no permissions of its own and no conditions; revoking it while the user is signed in ends their occupancy immediately, with the reason shown to them.
_Avoid_: assignment, membership, entitlement

**Takeover request**:
A notification sent to the occupant of a single-occupant role by an eligible user who wants it. Sign-in to an occupied single-occupant role is always refused rather than granted silently, so a takeover only ever happens by the incumbent's consent or by operational authority.
_Avoid_: handover, kick, steal

**Away**:
The occupancy state of a loop whose staffing-role occupants have all declared themselves off console or muted it — materially different from `vacant`, where nobody is signed in at all.
_Avoid_: off console (the user-facing phrasing for the action, not the state), idle, AFK

### Authority

**Observer**:
The seeded role every new user is eligible for: `monitor` on every loop present at install, `emit` on none. It is how listen-only is expressed — a role, never a property of a user.
_Avoid_: listen-only user, guest, viewer, spectator

**Reach**:
The set of loops a role may monitor, emit to or control, read straight off that role's permissions. Reach belongs to a (user, role) pair and is never composed across the several roles a user may be eligible for — a session is bound to one role, so a person's reach is only ever one role's worth at a time.
_Avoid_: access, scope, what someone can do, effective permissions

**Unreviewed loop**:
A loop created after install, on which system administration has not yet set or explicitly dismissed each role's permission. It exists so that a cell nobody has ruled on is distinguishable from one deliberately left at `none` — most sharply for `Observer`, which is seeded only against the loops present at install and is therefore blind to every later loop until someone says otherwise.
_Avoid_: new loop, unconfigured loop, draft

**Access request**:
A user's ask for reach they do not hold, resolved by system administration editing configuration — a grid cell, or an eligibility grant. It never produces a per-user exception, because there are none.
_Avoid_: permission request, escalation, override

**System administration**:
The capability to configure VoxLoop — creating users, roles and loops, granting eligibility, and setting the (role, loop) permission matrix. It belongs to the user, not to any role, because it must exist before any role does.
_Avoid_: superuser, root, owner

**Permission**:
The single value held by a (role, loop) pair, one of an ordered four — `none`, `monitor`, `emit`, `control` — each rung carrying everything below it. It is the only place voice authority is configured, so the entire model is one grid of roles against loops.
_Avoid_: access level, rights, grant, ACL, permission vector

**Operational authority**:
The capability to act during operations — cutting a live transmission, forcing a takeover, issuing monitoring directives. It is conferred by the `control` rung of a permission, so it is always scoped to particular loops, and it transfers at shift change along with the position.
_Avoid_: admin, supervisor, elevated permissions

### Voice

**Emission**:
Sending voice to one or more loops. Emission is always to a loop, never to a user, and is either momentary (held) or latched (press to open, press to close). It decomposes into two acts with different enforcement — *arming* and *keying* — which must not be collapsed into one.
_Avoid_: transmit, broadcast, push-to-talk (which names the input, not the act), talk

**Arming**:
Selecting a loop as a destination for emission. Arming says where voice *would* go; it is enforced by the server, which will not route to a loop the role lacks `emit` on, so an unarmed loop is unreachable rather than merely unselected. Arming is independent of subscription — a loop may be armed without being monitored, and monitored without being armed.
_Avoid_: selecting, enabling, talk-select, opening

**Keying**:
Actually emitting on the armed loops. Keying says whether voice *is* going. It is performed by the client for latency and signalled to the server, which remains the sole authority for telling anyone else that it is happening.
_Avoid_: push-to-talk (which names the input), transmitting, going live

**Preset**:
A named set of loops that momentarily replaces the loops a user has armed while keyed, reverting the moment they unkey. It is an ergonomic shortcut across loops the role may already emit on — never extra reach — and it narrows as readily as it widens.
_Avoid_: broadcast, all-call, macro, profile (which is persistent, and covers subscriptions and layout)

**Monitoring**:
Receiving the audio of a loop. A user monitors a loop; they do not "join" it, because a loop has no membership of its own.
_Avoid_: listening in, joining, tuning

**Subscription**:
The live choice to monitor a particular loop, held per session. Distinct from permission: permission says which loops a role *may* monitor, subscription says which of them it *currently is*.
_Avoid_: channel selection, tuning, membership

**Monitoring directive**:
A live instruction issued by operational authority requiring named roles to monitor named loops. It only ever adds subscriptions, never removes them; it applies to anyone occupying a targeted role, including those who sign in after it was issued; and it remains in force until explicitly cleared.
_Avoid_: mandatory subscription, forced listen, watch order, monitoring request

**Loop health**:
Whether a subscriber is actually receiving a loop's audio path — a third axis, distinct from whether anyone is talking on it and from whether it is `staffed`. A quiet loop and an unreachable loop sound identical, so they must never look identical.
_Avoid_: connection status, signal strength, online

**Mute**:
A user silencing a loop in their own ears, affecting nobody else. It is a personalisation, not a permission and not an unsubscribe — the subscription stands, so loop health and talking indicators keep arriving. Muting a loop one staffs drops it to `away`, because a loop nobody can hear is not staffed.
_Avoid_: silence, deafen, pause, unsubscribe

**Cut**:
An operational authority holder stopping another user's emission, latched until cleared. It closes the fan-out in the media plane rather than asking the client, applies to the whole uplink rather than one loop, and is cleared by the target themselves — its purpose is announcing an open mic, not punishing one.
_Avoid_: mute (which is the personal act), kick, silence, gag

**Attribution**:
The identity carried by a transmission. Every transmission is attributed to the role its emitter is signed into, with the individual user as a secondary reference — there is no way to emit as oneself rather than as one's role.
_Avoid_: talker identity, speaker name, caller ID

# VoxLoop

A software voice loop system for operations centres: people listen to and speak to each other over administered audio conferences. Spacecraft operations is the guide use case, but nothing in the model is specific to it.

## Language

### Core entities

**User**:
An account belonging to a person. A user is never a destination for voice and never carries operational permissions of their own — both attach to the role they have assumed.
_Avoid_: entity, account, operator, client

**Role**:
A staffable position that users assume, such as `Flight Director` or `Support Engineer`. It carries permissions, and it carries a default console its occupants start from rather than the console they end up with, and it is a slot users step into — not a group of users.
_Avoid_: position, station, group, team, entity

**Loop**:
An audio conference: a many-to-many bus that can be monitored and emitted to. A loop is never a person and never a role, and it is the only thing voice can be addressed to.
_Avoid_: channel, group, conference, net, entity

### Identity

**Principal**:
Anything that authenticates to VoxLoop. There are exactly two kinds — a user, who is a person, and a service principal, which is not. Everything downstream of authentication deals in principals, never in how one proved itself.
_Avoid_: identity, actor, client, caller

**Service principal**:
A non-human principal, such as a text-to-speech service, holding a long-lived administered token and bound to a role. The binding gives it that role's reach and its attribution, but never occupancy — a loop is not staffed because a service can speak on it.
_Avoid_: service account, bot, robot user, integration user

**Sign in**:
Authenticating to VoxLoop as a whole, by presenting credentials as a principal. It is the outermost act — it establishes who you are and nothing else, conferring no role, no reach and no audio. Signing out ends it.
_Avoid_: log in (a plain synonym, not a distinct act), assume (which is the separate act of taking a role), authenticate

**Lobby**:
The state of a signed-in user who has not assumed a role: no session, no audio, no authority, read-only. It shows the roles that user is eligible for, who occupies them, and the staffing state of the loops those roles staff — enough to answer *should I assume a role, and which?* and deliberately nothing more.
_Avoid_: home, dashboard, waiting room, idle, lounge

**Enrolment code**:
A single-use code issued by system administration by which a user sets their own password. It is the only way a password is ever set or reset, because VoxLoop has no mail path to send a link down, and it is handed over out of band.
_Avoid_: invite, invitation link, reset link, activation token

**External identity**:
The durable pair of issuer and subject naming a user in a customer's identity provider, stored against the user record and linked only by an explicit administrative act. An email address is never one, and never stands in for one.
_Avoid_: SSO identity, federated id, email, subject

**Audit log**:
The append-only record of authentication events, configuration changes and operational authority acts — decisions about the system, never the traffic through it. Who talked, and when, is not in it.
_Avoid_: activity log, event log, history, journal

### Occupancy

**Assume**:
The explicit act by which a signed-in user takes up a role, making the role occupied and creating their session. Occupancy is only ever established this way — never inferred from membership or presence, and never from being signed in.
_Avoid_: sign in (which is authentication to the application), log in, check in, claim

**Relinquish**:
Giving up an assumed role, ending the session and returning the user to the lobby. It is a full stop rather than a transition: audio ceases, subscriptions and arms are gone, and staffed loops drop — so changing role is a relinquish followed by an assume, and is never presented as seamless.
_Avoid_: sign out (which ends the whole authenticated state), release, drop, switch

**Max occupants**:
The limit on how many users may occupy a role at once. Single-occupant and multi-occupant roles are the same concept under different limits, not different kinds of thing.
_Avoid_: cardinality, capacity, seats

**Staffing role**:
A role marked as counting toward a particular loop's staffing state. It is set per (role, loop), so a loop may have several staffing roles or none, and it can only be set where the role may emit on that loop — a role that cannot answer cannot staff.
_Avoid_: owner, host, primary role

**Occupant**:
A user who has assumed a role and not yet relinquished it. Roles have occupants; loops have a staffing state — the two must not be run together.
_Avoid_: member, holder, incumbent

**Staffing state**:
Whether anyone is behind a loop: `staffed` (an occupant of one of its staffing roles is demonstrably hearing it), `away` (such occupants exist but none is hearing it) or `vacant` (neither). It asks who is actually hearing the loop, not who has merely assumed a staffing role, and it always carries the reason when `away`. A loop with no staffing roles configured has no staffing state at all and shows nothing, because `vacant` would read as an answer when there is no question — two people may be talking on it.
_Avoid_: occupancy (which belongs to roles), online/offline, presence, availability

**Session**:
A user's single live connection to the voice loops, created by assuming a role and bound to exactly one. A user has at most one at a time, though they may be signed in on several machines; assuming a role elsewhere ends the previous session and tells it why. Losing the signalling channel does not end it — the session is held for the reconnection window and resumed by name.
_Avoid_: connection, client, device, login, sign-in (which is the outer act and outlives every session)

**Resume**:
Reattaching a client to a session it already holds, after losing and regaining the signalling channel. It is not an act the user performs and not a new session — everything the server holds simply becomes visible again — so it confers nothing, clears nothing, and is never evidence that a human came back to the chair.
_Avoid_: reconnect (which is the channel's recovery, not the session's), rejoin, restore, re-assume

**Reconnection window**:
How long a session outlives the loss of its signalling channel before it is reaped. Reaching the end of it is a relinquish in every respect except that nobody chose it, and the user is left signed in, in the lobby, told why.
_Avoid_: timeout, grace period, TTL, session expiry (which is the sign-in's)

**Eligibility**:
The unconditional grant permitting a user to assume a role. It carries no permissions of its own and no conditions; revoking it while the user occupies the role ends their occupancy immediately, returning them to the lobby, with the reason shown to them.
_Avoid_: assignment, membership, entitlement

**Takeover request**:
A notification sent to the occupant of a single-occupant role by an eligible user who wants it. Assuming an occupied single-occupant role is always refused rather than granted silently, so a takeover only ever happens by the incumbent's consent or by operational authority.
_Avoid_: handover, kick, steal

**Away**:
The staffing state of a loop whose staffing-role occupants exist but none is hearing it — because they are off console, have muted it, are unreachable, or are not receiving its beacon. It is a property of the loop computed across every occupant of every role that staffs it, so one occupant going quiet moves nothing while another is still hearing; there is no partial value between `staffed` and `away`. Materially different from `vacant`, where nobody occupies a staffing role at all.
_Avoid_: idle, AFK, unavailable

**Off console**:
A user's assertion that they have stepped away. It drops the staffing state of the loops they staff to `away` and changes nothing else — subscriptions stand and audio keeps flowing. It is never inferred — not from idleness, focus or mouse movement — and it is cleared only by a deliberate act, either keying or clearing the assertion explicitly.
_Avoid_: away (which is the resulting loop state, not the act), idle, AFK, break

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
A user's ask for reach they do not hold, resolved by system administration editing configuration — a grid cell, or an eligibility grant. The ask itself happens out of band: VoxLoop has no mechanism for making it, because a user cannot see the loops outside their reach to name one. It never produces a per-user exception, because there are none.
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

**Authority loop**:
The loop an operational authority act is performed through, named by the actor rather than derived by the system. Every such act carries one: it is the loop the actor's role holds `control` on, and the loop through which the target must be reachable for the act to be permitted. It is what makes an authority act explicable after the fact, so it is recorded in the audit log alongside the actor and their role.
_Avoid_: scope, context, target loop (which is the loop being acted on, not the one conferring the right)

### Voice

**Emission**:
Sending voice to one or more loops. Emission is always to a loop, never to a user, and a user's emission is either momentary (held) or latched (press to open, press to close). It decomposes into two acts with different enforcement — *arming* and *keying* — which must not be collapsed into one.
_Avoid_: transmit, broadcast, push-to-talk (which names the input, not the act), talk

**Arming**:
Selecting a loop as a destination for emission. Arming says where voice *would* go; it is enforced by the server, which will not route to a loop the role lacks `emit` on, so an unarmed loop is unreachable rather than merely unselected. Arming is independent of subscription — a loop may be armed without being monitored, and monitored without being armed.
_Avoid_: selecting, enabling, talk-select, opening

**Keying**:
Actually emitting on the armed loops. Keying says whether voice *is* going. It is performed by the client for latency and signalled to the server, which remains the sole authority for telling anyone else that it is happening. It is driven by the intent of the input sources, never by any one of them directly.
_Avoid_: push-to-talk (which names the input), transmitting, going live

**Priority transmission**:
An emission that plays at full gain in every subscriber's ears whatever they have set that loop's volume to. It is conferred by a third momentary binding — a user *keys priority* — rather than by any standing property of a person, role or loop, and it lasts exactly as long as the key is held. It defeats the subscriber's own attenuation and nothing else: it lowers no other talker, does not defeat mute, and compels no subscription. It is marked on every loop it reaches, on the console of everyone monitoring that loop, whatever volume they have set and whether or not they have muted it, because the mark declares that somebody called this urgent rather than explaining why their audio got louder.
_Avoid_: priority speaker (which implies a standing attribute on a person), override, break-in, all-call, urgent, ducking (which is the mechanism VoxLoop does not have)

**Input source**:
Something a user can key with — a keyboard binding, an on-screen control, a peripheral, a native hotkey. A source reports only two things about itself, an *intent* (whether it currently wants to emit) and its *liveness* (whether it is present and working); it never knows which emission mode it is serving. Sources are additive: a user may have several, and keying follows whether any live one wants to emit.
_Avoid_: push-to-talk button, PTT key, input device (which is the hardware, not the binding), control

**Preset**:
A named set of loops that momentarily replaces the loops a user has armed while keyed, reverting the moment they unkey. It is an ergonomic shortcut across loops the role may already emit on — never extra reach — and it narrows as readily as it widens.
_Avoid_: broadcast, all-call, macro, profile (a term VoxLoop does not have; personalisation is the persistent thing, and it is never switched between)

**Monitoring**:
Receiving the audio of a loop. A user monitors a loop; they do not "join" it, because a loop has no membership of its own.
_Avoid_: listening in, joining, tuning

**Subscription**:
The live choice to monitor a particular loop, held per session. Distinct from permission: permission says which loops a role *may* monitor, subscription says which of them it *currently is*. The set is remembered as personalisation, so assuming a role restores the loops that role last had, but each subscription is live state that ends with the session.
_Avoid_: channel selection, tuning, membership

**Monitoring directive**:
A live instruction issued by operational authority that puts named loops on the consoles of everyone occupying named roles. It only ever adds subscriptions, never removes them, and it applies once per session — when issued, and again to anyone who assumes a targeted role while it stands. It promotes a loop rather than policing it: once added, the subscription is an ordinary one the operator may drop, mute or reorder.
_Avoid_: mandatory subscription, forced listen, watch order, monitoring request, hail (which is the same promotion without the authority)

**Directed subscription**:
A subscription somebody else added rather than the operator choosing it — by a monitoring directive or by a hail. It is marked with which of the two put it there and carries the reason, and it is droppable like any other: the marking exists so nobody mistakes it for their own choice, not to stop them dismissing it.
_Avoid_: forced subscription, mandatory loop, pinned loop

**Hail**:
One operator asking a role — or one named person occupying it — to come to a loop the hailer may emit on. It adds that loop to the target's console as a directed subscription and says who asked, and it is dismissed with one click. It reaches whoever holds the seat at that moment and nobody who takes it later, it grants no reach and defeats no mute, and it gets no reply: it asks, and it cannot compel. A monitoring directive is the same promotion carrying authority — standing, binding late arrivals, and audited.
_Avoid_: ping, invite, page, summons, call (which is the user-to-user voice VoxLoop does not have), notification

**Mute**:
A user silencing a loop in their own ears, affecting nobody else. It is a personalisation, not a permission and not an unsubscribe — the subscription stands, so loop health and talking indicators keep arriving. Muting a loop one staffs contributes to it going `away`, but does not decide it: staffing state is computed across every occupant of every role that staffs the loop, so it stays `staffed` while any one of them is still hearing it. A priority transmission does not defeat mute. It lasts as long as the session and no longer: a mute is never remembered, because a forgotten one silences a loop the moment its owner assumes the role again.
_Avoid_: silence, deafen, pause, unsubscribe

**Cut**:
An operational authority holder stopping another user's emission, latched until cleared. It closes the fan-out in the media plane rather than asking the client, applies to the whole uplink rather than one loop, and is cleared by the target themselves — its purpose is announcing an open mic, not punishing one.
_Avoid_: mute (which is the personal act), kick, silence, gag

**Attribution**:
The identity carried by a transmission. Every transmission is attributed to the role its emitter has assumed — or, for a service principal, the role it is bound to — with the individual user as a secondary reference. There is no way to emit as oneself rather than as one's role. It is carried by the model and read by recording; the operator console never renders it, showing only that a loop is being spoken on.
_Avoid_: talker identity, speaker name, caller ID

**Announcement**:
A synthesised transmission emitted by a service principal on loops its bound role may emit to, heard by whoever is subscribed and not muted. It is an ordinary transmission in every respect — it overrides nothing, and it runs for the length of its audio rather than being held or latched.
_Avoid_: broadcast (retired in favour of *preset*), notification (which is sent to a person, not a loop), alert, injection, message

**Pronunciation dictionary**:
The deployment's single list of literal replacements applied to an announcement's text before it is spoken, so that the site's own acronyms are read aloud correctly. It is configuration, held once for the whole deployment, and it applies to the attributed role's name as readily as to the caller's words.
_Avoid_: lexicon, phonetic overrides, word list

### State

**Observed state**:
A fact the server has seen for itself — a transport connected, a producer sending, a subscription held, an arm set. It is true as of the version it was published in.
_Avoid_: known state, verified state, real state

**Asserted state**:
A claim a user has made about themselves, of which there is exactly one: off console. It is only ever as true as the moment it was asserted, so it is always shown alongside how long ago the claimant last did anything deliberate. It is never inferred and never rendered like observed state.
_Avoid_: reported state, self-reported, declared status

**Presence document**:
The single versioned document, one per session, carrying every state that session may see. It is pushed by the server and rendered atomically, so what is on screen was simultaneously true; it is scoped to the loops the session's role may monitor.
_Avoid_: state feed, snapshot, sync payload

**Gap event**:
Something done to a session while it had no signalling channel, told to the operator on resume because current state cannot reveal it — a dropped latch is indistinguishable from one never set. It is a bounded list of things done *to* them, never a diff of the world, and it persists until dismissed.
_Avoid_: notification, missed event, changelog, diff, replay

**State authority**:
The single thing that holds every live fact about the running system — sessions, occupancy, subscriptions, arms, key state, connection state and loop health — and the only writer of any of them. Presence documents are projections it computes rather than records it keeps, which is what lets their versions be monotonic and what they show be simultaneously true. It holds nothing durable: everything it knows ends when the server does.
_Avoid_: session store, state store, cache, registry

**Connection state**:
A session's standing with the signalling channel: `confirmed`, `unconfirmed` (heartbeats missed, displayed state frozen and marked stale, emission still permitted) or `disconnected` (emission withdrawn at both ends, and the loops the session staffs drop to `away`). It describes the state channel, never the audio path — that is media path state, and the two fail independently in both directions.
_Avoid_: online/offline, connectivity, network status

**Media path state**:
A session's standing with the audio transport: `connected`, `impaired` (a transient fault that routinely heals itself) or `lost` (emission withdrawn). It is the audio path's own ladder, read from both ends and merged pessimistically, and it is what makes a session that can be told everything and heard by nobody a state the console can show rather than a silence it cannot explain.
_Avoid_: transport state, ICE state (which is one end's reading of it), audio health, connection state (which is the signalling channel's)

**Loop health**:
Whether a session is actually receiving a loop, measured from the arrival of that loop's beacon — a third axis, distinct from whether anyone is talking on it and from its staffing state. It is per (session, loop), so two subscribers may correctly disagree. A quiet loop and an unreachable loop sound identical, so they must never look identical. It is reported by the client over the signalling channel, so losing that channel stops the reports as a side effect — connection state answers for the loop then, and beacon loss is suppressed rather than counted as a second reason.
_Avoid_: connection status, signal strength, online

**Loop beacon**:
A silent low-rate stream the server emits on every loop, counted by each subscriber, whose arrival is what loop health is measured from. It proves the loop reaches a session; it does not prove any particular talker would be heard.
_Avoid_: keepalive, heartbeat (which is the signalling channel's), ping, tone

**Audience**:
The set of people who would actually hear a given emission, resolved per (role, user) into hearing, present but not hearing, and not subscribed. It is computed rather than stored, and it is shown before keying, not only during — as two counts rather than names, the third bucket being computed and deliberately not displayed.
_Avoid_: listeners, subscribers (which is who chose to listen, not who will hear), recipients

### Console

**Board**:
The console view showing one card per loop in reach. It is the glanceable view: a card carries the loop's staffing state as a word, not a sentence, and clicking its body toggles monitoring.
_Avoid_: grid (which is the admin console's role × loop matrix), tiles, dashboard

**Ledger**:
The console view showing one compact table row per loop in reach. It is the reading view, and it is where state too long for a card lives — the staffing reason above all. It holds the same loops in the same order as the board.
_Avoid_: list, table, detail view

**Transmit bar**:
The strip present in both console views carrying the armed set in words, the audience counts and the key state. It answers *who am I about to talk to* and, because it stays live while the key is held, *who am I talking to*, so it is never scrolled away and never worded differently between the views. A change to the armed set the session did not ask for is marked; a preset or a deliberate arm just redraws.
_Avoid_: status bar, toolbar, PTT bar

**Hail picker**:
The control opened from a loop, listing the roles permitted to hear that loop and whoever occupies each, from which a hail is addressed. It opens showing only what can be hailed, and reveals the rest greyed with the reason on request, so that an absence never has to be guessed at. It is the only place the console names a person, and it names them as the holder of a seat — never as the source of a transmission.
_Avoid_: roster, directory, shift board (none of which VoxLoop has), contact list, people panel

### Personalisation

**Personalisation**:
Everything a user has set about their own console that carries no authority: their subscriptions, per-loop volumes, loop order, default view, personal presets, push-to-talk bindings and audio devices. Each item is scoped to the smallest thing it is about, which is the user for a binding, the (user, role) pair for a subscription set, the (user, role, loop) triple for a volume, and the machine for a device. It is saved continuously rather than chosen between, it may only ever narrow within reach, and the permission grid overrules it silently and always.
_Avoid_: profile (nothing in VoxLoop is named and switched between), preferences, settings, layout, config

**Role default**:
The console an administrator sets for a role: its subscription set, default view and loop order, but never its volumes. A user occupying that role for the first time starts from it. It is applied once and never re-imposed, so it is a starting point rather than a floor, and nothing in VoxLoop can oblige an operator to keep a subscription.
_Avoid_: template, profile, enforced subscriptions, mandatory loops, preset (which is momentary and covers arms)

**Reset to role default**:
The single wholesale act on a user's personalisation, discarding what they have set for a role and starting again from that role's default as it stands now — never as it stood when they first assumed the role. It clears nothing live, so a mute or an arm set survives it, and it leaves bindings and audio devices alone because those are not the role's.
_Avoid_: restore defaults, revert, factory reset, start state

**Loop order**:
The order the loops in reach are shown in, the same in the board and the ledger. It is a complete ordering rather than a set of pins over a base order, so an operator's arrangement never rearranges itself underneath them, and a loop entering reach is appended at the end and marked new until they move it or dismiss the mark.
_Avoid_: sort, layout, pinning, watch order (which is the monitoring directive's)

# VoxLoop

A software voice loop system. Users assume roles, a role's reach is read off the (role,
loop) permission grid, and that grid is the only place voice authority is configured.

The design lives in [`docs/spec/v1.md`](docs/spec/v1.md); read
[`docs/spec/modules.md`](docs/spec/modules.md) first, because it says what the system *is*
rather than why. [`docs/adr/`](docs/adr/) holds the reasoning.

## What runs

One Rust binary — console, API, signalling, permission enforcement, TLS and the mediasoup
worker — beside the text-to-speech sidecar and one SQLite file. No reverse proxy, no Node
runtime, and one systemd unit ([ADR-0040](docs/adr/0040-one-binary-one-unit-four-moving-parts.md)).

The mediasoup worker is **linked into the binary and runs on a thread of it** rather than
beside it as a child process
([ADR-0070](docs/adr/0070-the-mediasoup-worker-is-a-thread-of-this-process.md)) — the Rust
API works that way, where the Node.js one spawns a child. Its health is observed on a channel
and reaches the console as **media path state**. A worker that dies takes every transport with
it and cannot be replaced in place, so the binary **stops, non-zero**, and systemd brings the
unit back — the ordinary restart path, rather than a console that works and will never make a
sound again.

## Building

`cargo build` needs Rust, a **C++ toolchain** and **Python 3**. The last two are `mediasoup-sys`,
which compiles `libmediasoup-worker` from source and bootstraps meson and ninja into a
throwaway virtualenv to do it. It is a few minutes the first time and cached afterwards, and
it is the price of the worker being inside the binary rather than beside it
([ADR-0070](docs/adr/0070-the-mediasoup-worker-is-a-thread-of-this-process.md)).

The mediasoup crate is **pinned exactly** — the `=` in `Cargo.toml` is load-bearing, because
cargo treats every `0.x` minor bump as breaking and this is the audio path. Upgrading it is
scheduled work: bump the line by hand as its own commit, having read the changelog between
the two versions, and re-run the load test ([ADR-0006](docs/adr/0006-mediasoup-carries-the-audio.md)).
Never a `cargo update`.

**`mediasoup-client` is pinned exactly too**, and for the same reason said about the other end
of the same negotiation: the version in `web/package.json` carries no caret, so a minor bump
cannot arrive on somebody's `npm install`. The two halves have to agree about ICE, DTLS and
RTP, and the failure mode of them drifting apart independently is audio that does not cross
under a deployment nobody tested. Bump it the way the crate is bumped — by hand, as its own
commit, with the changelog read.

That build does not embed the console, and does not need Node or `web/dist` to exist.

A **release** build embeds the console, and has an ordering requirement:

```sh
cd web && npm install && npm run build   # writes web/dist, which is never committed
cd .. && cargo build --release --features embed-web
```

`cargo build --release` without `--features embed-web` refuses to compile, so a release can
only be built one way. Build it with the feature and no `web/dist` and it fails outright.
Build it over a **stale** `web/dist` and it succeeds and ships the previous console: that is
the one failure nothing here can catch, which is why the two commands belong together in CI
([ADR-0037](docs/adr/0037-the-client-ships-as-static-assets-embedded-at-release.md)).

## Running it

```sh
scripts/dev            # build the console, start the server, make an administrator
scripts/dev --fresh    # the same, from an empty store
```

It writes a self-signed certificate, a deployment file and a store under `.dev/`, redeems
the bootstrap code itself, and prints the URL and a password generated for that store. Set
`VOXLOOP_DEV_PASSWORD` to keep one across `--fresh` runs, `VOXLOOP_DEV_PORT` to move the
port.

It opens the page for you; `VOXLOOP_DEV_NO_OPEN=1` stops that. If you are typing it instead,
type it exactly as the banner prints it — **`https://127.0.0.1:8443`**. A host and a port on
their own get you `http`, which nothing in a VoxLoop deployment speaks, and `localhost`
resolves to IPv6 first on most boxes while the binary listens on one address; either mistake
reads as the site refusing the connection.

Accept the certificate warning before signing in: the sign-in cookie is `Secure` and the
browser will not keep it otherwise. You should land on a sign-in form.

It is a development launcher and not a way to provision anything: nothing it writes belongs
on a machine anybody else can reach, which is why `.dev/` is ignored by git.

### By hand

A deployment does the same thing deliberately. VoxLoop terminates TLS itself, so it needs a
certificate before it will start; for local work any self-signed one will do:

```sh
openssl req -x509 -newkey rsa:2048 -nodes -days 365 \
  -subj "/CN=localhost" -addext "subjectAltName=DNS:localhost" \
  -keyout private-key.pem -out certificate.pem

cp voxloop.example.toml voxloop.toml   # then point it at those two files
cargo run
```

`cargo run` takes the deployment file as its first argument, or from `VOXLOOP_CONFIG`, or
as `voxloop.toml` in the working directory. Every value in it can be overridden from the
environment: `VOXLOOP_LISTEN__ADDRESS=127.0.0.1:9443 cargo run`.

The file's `[media]` section has **the one value with no default**: `announced_address`,
which is what goes into every ICE candidate and so is the address a client dials rather than
the one the box binds. Get it wrong and VoxLoop comes up, serves the console and fills its
seats while no audio ever arrives, so a deployment that has not said where it is does not
start. The rest of the section is one port carrying UDP with ICE-TCP on the same number, and
there is no TURN server to configure
([ADR-0006](docs/adr/0006-mediasoup-carries-the-audio.md)).

## First start

There are no default credentials, ever. A deployment nobody administers yet mints a one-time
**bootstrap code** to its own log on every start, invalidating the code the start before it
minted, and redeeming it creates the first system administrator
([ADR-0025](docs/adr/0025-credentials-are-administered-because-there-is-no-email.md)):

```sh
curl -k -X POST https://localhost:8443/api/bootstrap \
  -H 'content-type: application/json' \
  -d '{"code":"<from the log>","username":"you","password":"a long enough password"}'
```

Passwords are Argon2id with a twelve-character floor, no forced rotation and no complexity
rules. Once an administrator exists that route is not registered at all — it is the one
operation VoxLoop hides rather than refuses. From then on it is `/api/sign-in` and
`/api/sign-out`, and **the root of trust is being on the box**: whoever can read the server's
log at first start is the administrator.

## Enrolment codes

Everyone after the first administrator gets in the same way. VoxLoop has no mail path, so
there is no invitation link, no "forgot password" and no self-service reset — and no
self-registration either. What replaces all of them is one thing
([ADR-0025](docs/adr/0025-credentials-are-administered-because-there-is-no-email.md)):

An administrator creates the user record, then issues an **enrolment code** against it —
single-use, expiring after a week, and **handed over out of band**, in person or over the
comms the operations centre already has. Redeeming it sets that user's password:

```sh
curl -k -X POST https://localhost:8443/api/enrolment \
  -H 'content-type: application/json' \
  -d '{"code":"<handed to you>","password":"a long enough password"}'
```

The code identifies the user, so there is no username to send and nothing to aim at somebody
else's account. **A password reset is the same act again**: issue another code. Issuing one
invalidates whatever that user had outstanding, so a mislaid code is replaced rather than
left in circulation, and the console shows a code exactly once — nothing reads one back
afterwards, the audit log included.

Redeeming a code **ends every sign-in the user holds**, because the credential those
sign-ins stood against is not the one the account has any more.

A signed-in user changes their own password by re-presenting the current one, at
`POST /api/password`. That one **does not end the session**: an operator on the air who
changes their password should not lose audio for it. Both routes are rate-limited on source
and audited, and no number of failures locks anybody out — auto-lock is a denial of service
aimed at whoever is starting a shift, so account lock stays a deliberate administrative act.

## The on-box CLI

The same binary, run with a subcommand instead of a deployment file:

```sh
voxloop administrator <username>     # make or promote a system administrator
voxloop reset-password <username>    # take a password away and issue a code
voxloop help
```

Both print a single-use enrolment code to hand over; neither sets a password itself, because
an enrolment code is the only way one is ever set. **That code is redeemed over HTTPS**, so
the recovery these commands offer is a way back into a deployment that is still serving —
not a way to sign in to one that is down. Point either at a deployment file with
`--config <file>` or `VOXLOOP_CONFIG`, exactly as serving does.

`administrator` also **unlocks the account**, which is a third act neither the console's
*unlock* nor the enrolment path performs from here. It has to: *last system administrator*
counts flag holders and nothing else, deliberately, so a box with two administrators can have
both of them locked and nobody left to unlock either. That is the state this command exists
to get out of.

**These commands run outside VoxLoop's authorisation model entirely.** They evaluate no
requirement, resolve no principal and answer to nobody: being able to run this binary against
the deployment's store is the whole of the authorisation. That is deliberate and permanent
rather than a first-run convenience — with no mail path, the last administrator locking
themselves out would otherwise be an unrecoverable deployment, and the bootstrap code is not
re-minted while somebody still holds the flag. **It means shell access to this box is the
highest privilege in the system** ([v1 §16](docs/spec/v1.md#16-accepted-gaps)).

Everything the CLI does is written to the audit log, attributed to `the on-box CLI` with no
actor id, because there is no person to attribute it to.

## The lobby

Signing in puts you in the **lobby**: signed in, no role assumed, so no audio, no authority
and nothing to configure. It answers one question — *should I assume a role, and which?* — by
listing the roles you are eligible for and who occupies each
([ADR-0023](docs/adr/0023-sign-in-is-to-the-application-and-a-role-is-assumed.md)).

Sign in and assume are two acts with two lifetimes. There is **no idle timeout on a session
and no absolute cap on a sign-in**; a sign-in ends after 24 hours with no deliberate act, and
that clock runs only in the lobby, so an operator holding a role through a thirty-hour
incident is never signed out for failing to click anything.

The lobby arrives over the **signalling channel**: one WebSocket per tab, opened at sign-in
at `/api/signalling`, carrying one versioned document that is rendered whole. It is a second
authorised surface and **every message on it is checked, not just the upgrade**
([ADR-0054](docs/adr/0054-every-operation-declares-its-authorisation.md)) — an administrator
editing a grid cell mid-shift has to land on a socket that is already open. The upgrade takes
the sign-in cookie and nothing else: a service principal has no session and no socket, and a
request presenting a cookie and a token together is refused rather than resolved by
precedence.

## Assuming a role

**Assume** takes up a role from the lobby and creates the **session** that carries voice. It
mints a session id, moves the socket from `SignedIn` to `Session`, and swaps the lobby for
the **presence document**. **Relinquish** ends the session and puts you back in the lobby.

A user has **at most one session**, though they may be signed in on several machines, so
assuming a role anywhere ends whatever session they held and tells that console why.
Assuming an **occupied single-occupant role is refused** rather than granted silently, and
`max_occupants` is enforced at every value. Both ends of a session are audited, and the end
carries its reason.

**Changing role is a relinquish followed by an assume, and the console says so.** There is no
role picker on the console and no *switch*: you give the role up, land in the lobby, and take
the other one from there. Audio genuinely stops in between, and a control that hid that would
be the class of lie the product exists to avoid.

## The presence document

State reaches a session as **one versioned document, pushed by the server and rendered
atomically** ([ADR-0019](docs/adr/0019-presence-is-one-versioned-document-scoped-to-reach.md)).
There are no per-topic streams: they permit a torn state — arms as of one instant beside
subscriptions as of another, each individually true and the combination never true at any
moment.

**The document is the API.** Whatever the console renders is in it, and anything in it is
something the server has committed to keeping true. It carries the session, the role it is
bound to, its **media path state**, whether the server has this session down as
**transmitting**, and the loops in reach with **which of them the session is monitoring**,
**which it has armed**, **which are being spoken on**, **which it has muted**, **how loud
each plays** and **whether each monitored loop is reaching it**; staffing state and the
audience land in it one ticket at a time.

It is **scoped to reach** — only loops the session's role holds at least `monitor` on — and
it is recomputed on every tick, so a grid edit narrows or widens a live session's document
without a re-assume. Occupancy is *not* scoped to reach and is deliberately not in the
document at all: the hail picker fetches a roster when it opens
([ADR-0048](docs/adr/0048-the-hail-picker-is-the-only-place-the-console-names-a-person.md)).

Versions are **monotonic per session** and move only when the document does, so *is this the
same state* stays answerable. The wire is JSON at a ~5 Hz tick.
`permessage-deflate` with context takeover is specified and **not yet built** — the WebSocket
implementation underneath negotiates no extensions — which costs bandwidth and nothing else
([#78](https://github.com/edwardhutchinson/voxloop/issues/78)).

## The media path

A session gets a **media path of its own, bound to it at creation**, opened by the assume
that minted the session and closed by whatever ends it. It is **two WebRTC transports, one
each way**, because a browser's media library builds a directional one at each end. One
Worker, one Router and one shared `WebRtcServer` port carry every session's, because a loop is
not a transport primitive: a transport belongs to one router, so a router per loop would give
somebody monitoring six loops six ICE and DTLS sessions. Two per session is two, whatever they
are armed on and however many loops they monitor.

The media plane's interface names domain operations only — open a path, close a path, take
this client's uplink, make this audience hear this talker — and **it executes routing rather
than computing it**
([ADR-0063](docs/adr/0063-the-media-plane-executes-routing-it-never-computes-it.md)): no
subscription, arm set or permission rung crosses into it, and a loop crosses only as an
opaque label. It is a **sink** ([ADR-0062](docs/adr/0062-the-call-graph-is-acyclic-and-effects-modules-are-sinks.md)):
it calls nothing, every operation on it answers nothing, and what it has to say it says on a
channel. mediasoup's callbacks fire on mediasoup's threads, so that channel is the whole of
the bridge into axum — no blocking call, no borrowed tokio handle.

**Media path state** is a session's standing with the audio transport: `connected`,
`impaired` (a transient fault that routinely clears itself, through which emission stands) or
`lost` (emission withdrawn)
([ADR-0042](docs/adr/0042-the-media-path-has-its-own-ladder.md)). It is a **second, entirely
independent axis** from the signalling channel — a session can be told everything and heard
by nobody — and it is in the presence document because the transmit bar has to say **which**
of the two withdrawal conditions applies.

It is **client-driven and server-backstopped**. A browser tells a transient `disconnected`
from a terminal `failed`; mediasoup's `iceState` has no `failed` at all and takes around
thirty seconds of ICE consent freshness to say anything, which is longer than the whole
signalling ladder — so the client reports over the socket and the server's
`on_ice_state_change` and `on_dtls_state_change` cover the client that is wedged or lying.
The two ends **merge pessimistically: green needs both, red needs one.** A session that has
just been minted reads `lost` at both ends, because a transport nobody has connected to
carries no audio, and that is what the bar says.

The client half is driven by the two transports' own `connectionstatechange`, merged the same
way at that end: green needs both directions, red needs one. A session that cannot receive is
as unable to work as one that cannot send, and the transmit bar has one thing to say about
either.

**The client's own media negotiation crosses the seam as a value nothing above the media plane
can read.** ICE candidates, DTLS fingerprints and RTP parameters are a conversation between
the worker and the library in the browser; VoxLoop owns the channel it happens on and has no
opinion about what is on it, which is what keeps `DtlsParameters` from turning up in
Transport's signature. It travels on the signalling socket like everything else, because
**there is one channel and no second one**.

**One session's media path going is not the worker going**, and the two are opposite
decisions. A session whose transport has failed keeps its role indefinitely: the operator is
present, reading a working console that can say exactly what is wrong, and ending it for them
takes the decision from the person best placed to make it, possibly mid-fix. The worker going
is the deployment losing its purpose, with nobody left to leave the judgement with — so live
state moves first, so the last thing every console is told about itself is true, and then the
unit goes down.

## Loop health and the loop beacon

DTX means silence sends no packets, so **a quiet loop and an unreachable loop sound identical,
and they must never look identical** (v1 §6). Loop health is the third axis beside connection
state and media path state: whether a session is actually receiving a loop, **measured rather
than asserted** ([ADR-0017](docs/adr/0017-loop-health-is-measured-not-asserted.md)).

**Every loop runs a loop beacon**: one silent Opus packet every five seconds, produced on a
direct transport on the one router, from the moment the loop is created until it is deleted,
**whether or not anybody monitors it**. The list comes from the store at startup and after
every loop is created or deleted. Every session monitoring a loop, muted or not, is carried
**one paused carriage of that loop's beacon**, which the client builds, never plays, and counts
from its receiver's `packetsReceived`. It sends the running totals up as `beacons-counted`
whenever they move. **The client counts and the server judges**: a total that moved is an
arrival. A loop with nothing counted for fifteen seconds (three intervals, so two lost packets
say nothing) is `not-receiving`. A loop just taken up, or just back from a channel outage, is
`checking` until the first count or the window runs out. A wedged client reports nothing, so it
fails safe.

The beacon **is never a talker**. It is never added to the `AudioLevelObserver`, and it has no
session to be a talker as, so the recording tap, which is per (talker, destination loop), cannot
address it. At pilot scale it is around 240 packets a second across the deployment.

**Health is per (session, loop)**, so two subscribers may correctly disagree. It is in the
presence document on each monitored loop as `receiving`, `checking` or `not-receiving`, and
`null` elsewhere. It is **also `null` while the server has the session's channel as anything but
confirmed**: the counts ride that channel, so connection state already explains the silence, and
showing beacon loss as well would turn one failure into two competing reasons. The board says
`Not receiving` or `Checking` as a word and says nothing when the loop is being received, the way
it says nothing about a loop at unity. The ledger says it in a sentence on every monitored row.

**Beacon loss is one of the reasons an occupant is not hearing a loop**, which is what turns
`staffed` from *says they're listening* into *demonstrably receiving*. The state authority
answers that per occupant, taking the reason furthest upstream: `unreachable`, then `off
console`, then `not subscribed`, then `not receiving it`, then `muted`. Staffing state
itself, counted across every occupant of every staffing role, is
[#48](https://github.com/edwardhutchinson/voxloop/issues/48).

**What the beacon does not prove is recorded and left open**: the downlink is per talker, so
the beacon's carriage is not the one carrying anybody's voice. Loss proves deafness. Arrival
does not prove you would hear a given talker. One beacon per (loop, talker) would close the gap,
and it was rejected as forty times the mechanism.

**The console also checks its own output path**, which neither the server nor the beacon can
see. At assume it asks the operator to play a check tone and say whether they heard it. After
that it watches `devicechange` and compares the label and group of the `default` output across
events, so a headset unplugged or a default swapped under the operator is **said aloud, with
the tone offered again**. The swap detection is asserted at moderate confidence and is to be
confirmed on real hardware alongside
[#17](https://github.com/edwardhutchinson/voxloop/issues/17).

## Off console

**Every state VoxLoop shows is observed or asserted**
([ADR-0016](docs/adr/0016-displayed-state-is-observed-or-asserted.md)). Observed state is
something the server saw for itself; asserted state is a claim a user made about themselves,
and there is exactly one of those: **off console**.

**It is set and cleared by hand, and never inferred.** Idle-based auto-away is rejected
outright — an operator watching telemetry is idle at the keyboard and very much on console —
so there is no idle timer anywhere in the product, and `npm test` refuses `mousemove`,
`scroll`, `focus` and `visibilitychange` across the whole console for that reason.

**Any deliberate act clears it.** Keying, changing a subscription, changing an arm, answering
a prompt, dismissing a banner: each of those is a person acting on a console, and each of them
already arrives at the server as a signalling message. Transport rules on every message
exhaustively — the same function that decides what renews a sign-in — so a heartbeat, a media
path report and a beacon count clear nothing and refresh nothing, and a message nobody has
ruled on does not compile. *I am back on console* is one of those acts rather than a second
mechanism: its handler does nothing but answer with the document.

**The claim is never shown without the age of its evidence.** The document carries the two as
one value — `off_console` is `null`, or an object holding how many seconds ago the claimant
last did anything deliberate — so there is no rendering in which the claim appears alone. The
console draws it in words that say who said it (*You said you are off console. Last active 14
min ago.*) inside a dashed outline nothing observed wears, above both views, because it is
about the person in the chair rather than about any loop.

**This is the one running age in the document.** The connection's age belongs to the console,
because a session that hears nothing is told nothing; this one belongs to the server, because
the acts it measures arrive there and because everyone else who is later shown the claim — the
audience ([#49](https://github.com/edwardhutchinson/voxloop/issues/49)), the staffing reason
([#48](https://github.com/edwardhutchinson/voxloop/issues/48)) — has no clock of their own to
run it on. It moves the version once a second, and only while somebody is off console.

**A stale assertion is still shown, with its age.** Nothing expires one and nothing resolves
it: an operator who said they were stepping out three hours ago reads exactly that, and the
judgement is the reader's. That is a deliberate refusal to be helpful.

**Declaring it changes nothing else.** Subscriptions stand, arms stand and audio keeps
flowing, so the operator who steps away and hears something over their headset from three
metres away still hears it, and coming back is a click rather than a resynchronisation. What
it costs is the staffing state of the loops the role staffs, which drops to `away` — the state
authority already reports off console as the reason an occupant is not hearing a loop, and
counting that across occupants is [#48](https://github.com/edwardhutchinson/voxloop/issues/48).

**It is never remembered.** Nothing writes it anywhere, so a seat taken up again starts on
console: a day-old assertion is not a fact about anything
([ADR-0050](docs/adr/0050-personalisation-persists-what-is-safe-to-be-stale.md)).

## The operating console

Assuming a role puts the **presence document** on screen as **two views of one loop list**,
both complete, both driven by that one document, so they cannot disagree about anything
except layout
([ADR-0032](docs/adr/0032-the-console-is-two-views-of-one-loop-list.md)).

The **board** is a card per loop in reach: the glanceable view, and what a control room reads
at a glance. The **ledger** is a compact table row per loop: the reading view, and where state
too long for a card lives. A card cannot hold a sentence, so anything the model requires fits
the board as a word and may be a sentence only in the ledger — the rung the role holds is
`emit` on a card and *hear it, and speak on it* in a row, and whether the loop is being
monitored is `Monitoring` on a card and *you are hearing this loop* under the row's control.
**From here on, a state that renders in only one view is a bug**, and
`web/tests/board-and-ledger.test.js` asks every question of both views at once for that
reason.

**Order is shared.** Both views are handed one list, in the order the document arrives in,
which is the **administered base loop order**
([ADR-0053](docs/adr/0053-the-loop-order-is-complete-and-a-new-loop-lands-at-the-end.md)) —
neither alphabetical nor creation order. Two independent orders would put the same loop third
in one view and eleventh in the other, which is the quiet kind of disagreement that teaches an
operator to distrust the console. A personal order, and a remembered default view, are
personalisation and are still to come.

The **transmit bar** is present in both views, placed differently in each, **worded
identically** and **never scrolled away**
([ADR-0034](docs/adr/0034-the-transmit-bar-is-always-visible-and-the-audience-is-a-count.md)):
on the board it closes the field along the bottom edge, and in the ledger it rides above the
rows rather than under a table of unknown length. It is one component so that it is one
wording. It carries **media path state**, the **armed set in words**, the **key
state** and the two ways to key; the two audience counts arrive with
[#49](https://github.com/edwardhutchinson/voxloop/issues/49) and the presets with
[#56](https://github.com/edwardhutchinson/voxloop/issues/56).

**Nothing renders optimistically**
([ADR-0016](docs/adr/0016-displayed-state-is-observed-or-asserted.md)). Neither view keeps any
state of its own: what is on screen came out of the last document and can be nothing else, so
a toggle will visibly lag a round trip and switching views loses nothing. **Nothing about the
world is held on the page.** What it does hold is four things that are not the world's to say
— which view is showing, which keys talk, whether the key is latched open, and the source that
went while it was being held — and a test names them, so a fifth has to be argued for.

**The console renders no motion**, and the one exception is a file the check names. Motion is
permitted in exactly one place, the talking indicator
([ADR-0033](docs/adr/0033-the-console-shows-that-someone-is-talking-never-who.md)) — so
`npm test` refuses `animation`, `transition`, `@keyframes` and Svelte's motion directives
everywhere but `Talking.svelte`, and holds that file to what the permission was given for: one
animation, one set of keyframes, a fixed duration, and `steps` rather than an ease, because a
continuous ramp reads as a level and a level is the one thing the indicator may never imply.

## Monitoring a loop

**Subscription is the live choice to monitor a loop, and it is distinct from permission**
(v1 §5): the grid says which loops a role *may* monitor, and the subscription says which of
them it currently is. Every loop in reach is on the console whether or not it is being heard,
so an operator sees what they could hear and picks.

**Clicking a loop toggles it, with no confirmation.** On the board that is the card body; in
the ledger it is a control in the row, because a table row is not a control and one that
swallowed clicks would take the mute and the cog down with it. **Arm, mute and cog must not
propagate the card's click**, and that is kept structurally rather than by remembering to
stop propagation later: the card body is a `<button>`, which cannot contain another control,
so anything added to a card is its sibling.

Nothing renders optimistically, so **the loop changes when the server says it has**. The
click visibly lags a round trip, and that is the design rather than a cost of it — a misclick
on a loop the operator staffs announces itself by dropping that loop to `away` for everyone.
It is **two messages rather than one toggle** for the same reason: a second click on a card
that has not caught up yet says the same thing twice and lands on the same state, where a
toggle would undo the first.

It is gated on `Grid(monitor, loop)` — **the first live consumer of that requirement**, and
the first message whose requirement is a function of what it carries rather than of who sent
it, so it is built per message rather than registered once.

**The set is remembered per (user, role) and restored on assume.** That is what makes a
restart survivable: a restart ends every session and every operator must assume again, and if
the set persists, assuming rebuilds their console instead of every operator rebuilding their
loop set by hand during whatever incident caused the restart
([ADR-0050](docs/adr/0050-personalisation-persists-what-is-safe-to-be-stale.md)). The set is
the memory of a live act rather than the act: a subscription itself ends with the session.

**The write rides the live act and is best effort.** There is consequently **no
personalisation configuration endpoint** — the signalling channel still carries no
configuration API, and the endpoint list stays enumerable. The write **can never fail a live
act**: if the live change lands and the write does not, the console is correct and the
preference is lost, which is the right way round. A failure is logged loudly, because a
deployment whose personalisation writes are failing is one whose operators will rebuild their
consoles by hand after the next restart.

**The grid overrules personalisation silently and always, and keeps it inert rather than
dropping it**
([ADR-0051](docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md)). A
remembered subscription to a loop the role has since lost `monitor` on is not rendered and
not deleted, so a temporary revocation does not destroy somebody's console arrangement and a
loop that leaves reach and comes back comes back where it was.

A pair with nothing remembered starts with nothing up. Seeding a first assume from the
**role's default console** is [#27](https://github.com/edwardhutchinson/voxloop/issues/27)'s,
along with the rest of the personalisation rules — loop order and the default view.

## Mute, volume and loudest-wins

The three things an operator does to shape what they hear, and the rule that settles overlap
(v1 §5).

**Mute silences a loop in the operator's own ears and touches nobody else.** It is not an
unsubscribe: the subscription stands, so the loop's talking indicator and its priority mark
keep arriving, and so does its loop health. It is enforced **in the fan-out rather
than in the client** — a muted loop is one the state authority does not count the operator as
hearing, so no talker is carried to them on it. That is also what makes a mute sovereign over
priority ([ADR-0045](docs/adr/0045-priority-defeats-attenuation-and-nothing-else.md)): a priority
transmission raises the gain on a carriage, and there is no carriage. Everybody else on the
loop hears exactly what they did before.

**A mute presupposes a subscription**
([ADR-0049](docs/adr/0049-the-role-is-the-profile.md)), so a loop nobody is monitoring offers
no mute, and dropping a loop drops its mute with it. **It is never remembered**
([ADR-0050](docs/adr/0050-personalisation-persists-what-is-safe-to-be-stale.md)) — a forgotten
one silences a loop the moment its owner assumes the role again — and **it never expires**,
because an unexpected un-mute mid-incident is its own hazard. Nothing but the operator's own
hand, or dropping the loop, takes one away.

**Per-loop volume is personalisation per (user, role, loop)**, from silence to unity and no
further: it is an attenuation control, and every loop starts at unity. It is written through
as it is set, best effort, exactly as the subscription set is, and it comes back at the next
assume. It sits **behind a cog on the card and the row**, which opens a modal scoped to that
loop holding only the volume — it is not a live operational control, so nothing on the main
surface can nudge it. The modal is a native `<dialog>`, so the page behind it is inert and
Escape closes it. The card says a loop is turned down (`40%`) and says nothing of one at unity;
the ledger says it on every row. **Per-loop volume is the one attenuation in VoxLoop that
nothing warns anybody about**, and the card is the only place the operator who turned it down
is reminded.

**Loudest-wins is settled at the client.** The downlink is one stream per audible talker, so a
talker reaching an operator on several loops arrives once. Each carriage comes with **which of
the operator's own loops it is heard on** — handed down with the audience, carried by the media
plane as the labels it was given, and said again when they move — and the client plays it at
the loudest volume among them, read off the same presence document the cards are drawn from.
They are only ever loops the operator is monitoring, so the receiver still learns nothing about
where else the talker went
([ADR-0057](docs/adr/0057-the-receiver-is-never-told-where-else-a-transmission-went.md)). A loop
turned all the way down is still a loop the fan-out carries: silencing one outright is what a
mute is for.

Mute and volume are `Session` rather than a grid check (`docs/spec/api-surface.md`) — they
reach nothing and nobody — and neither is audited, because a user shaping what they hear is not
a configuration change.

## Arming, keying and hearing

**Emission is two acts with two enforcements** ([ADR-0008](docs/adr/0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md)).

**Arming** selects a loop as a destination. It is gated on `Grid(emit, loop)` and **that check
is the whole of the enforcement**: the fan-out is built from the arm set and from nothing
else, so a loop that never got past it has no route and there is nothing for a client to
bypass. Arming and disarming cost **no renegotiation** — the uplink already exists and does not
address, so both directions are a routing change at the server.

**Keying** is the client enabling its own microphone track, and then telling the server. That
order is what buys **key to first audio under 100 ms**: publishing on each press would put a
renegotiation on the most latency-critical action in the product and clip the *"Flight,
CAPCOM"* that identifies the speaker. **The server is the sole authority for saying it is
happening**, including to the operator doing it — the transmitting lamp is a field of the
presence document, and the console has nothing else to light it from. That round trip is the
cost of the honesty rule and it is paid deliberately: audio is already flowing by then.

**The route is per arm and not per key**, which is the same decision seen from the other side.
Gating the fan-out on the key signal would put the server back in the latency path and would
quietly remove the residual ADR-0008 accepts out loud: **a defective or hostile client can
keep sending while claiming to be unkeyed.** The arm boundary caps that to loops the role may
already reach, and mediasoup's **`AudioLevelObserver` runs in v1** — not as optional
instrumentation — so the discrepancy is visible from the server. Supervision is where the two
halves meet: the media plane knows a voice is on the wire and nothing about anybody's claims,
the state authority knows the claims and nothing about the wire, and neither can ask the
question alone. It is a log line rather than an act, because cutting somebody automatically on
a half-second average level is not a decision to take without a person behind it
([#51](https://github.com/edwardhutchinson/voxloop/issues/51) is the act that has one).

**Arming and subscription are independent in both directions**
([ADR-0013](docs/adr/0013-arming-is-independent-of-subscription.md)). Arming puts a loop in
nobody's ears, the arming operator's least of all, and monitoring makes no destination. An arm
never enters the subscription set, because staffing state is read off subscriptions and arms
pushed into that set would make loops read `staffed` because somebody was *talking at* them.
**Emitting blind is therefore legal**, and the console compensates: a blind arm says so in
words, and every armed loop shows whether somebody is transmitting on it.

The two sets are told apart once more when a cell moves. **A subscription outside reach is
kept and left inert** so a revocation that is undone leaves the console where it was
([ADR-0051](docs/adr/0051-personalisation-is-scoped-to-the-smallest-thing-it-is-about.md));
**an arm outside reach is dropped for good**, because a route that came back on its own when
the cell did would put somebody on the air with their hand on nothing. For the same reason
**an arm is not remembered** — the subscription set is restored on the next assume and the arm
set starts empty.

### Three layers, and only the middle knows what a loop is

([ADR-0007](docs/adr/0007-the-client-emits-one-stream.md))

| | |
|---|---|
| **Uplink** | one stream, encoded once, whatever the talker is armed on. It transmits; it does not address |
| **Server** | fans it into every loop the talker has armed. The only place loop identity exists in the media path |
| **Downlink** | **one stream per audible talker**, not per (talker, loop), mixed in the client |

Opus, **48 kHz, mono, 20 ms frames**, a ceiling around 32 kbps, **inband FEC and DTX both on**
([ADR-0010](docs/adr/0010-opus-mono-and-the-latency-budget.md)). The router advertises
`useinbandfec` and `usedtx`; the encoder's half of the same decision is the browser's and is
asked for when the microphone is published.

**The state authority computes the audience and the media plane executes it**
([ADR-0063](docs/adr/0063-the-media-plane-executes-routing-it-never-computes-it.md)). The rule
is one line and every clause in it is load-bearing: **for each loop the talker has armed,
everybody else monitoring that loop within their own reach.** A listener appears once per
destination, because the recording tap is addressed per (talker, destination loop)
([ADR-0009](docs/adr/0009-recording-taps-plain-rtp-on-loopback.md)) — and the media plane
collapses the pairs into one carriage, because the finer split would hand somebody monitoring
two of a talker's loops the same voice twice.

The whole fan-out is recomputed and **handed down when it moves**, the same way the presence
document's version moves when the document does. It is **taken rather than read**: whichever
socket asks first while it has moved is the one that carries it, and the rest are told there is
nothing to do. It is worked out for every talker at once rather than per session, because one
operator taking a loop up changes the audience of everybody armed on it.

### The talking indicator

**A loop is being spoken on, and never who**
([ADR-0033](docs/adr/0033-the-console-shows-that-someone-is-talking-never-who.md)). One flag on
the loop, identical for one talker and for five, in both views and in the same words. It is
true whether or not this console is monitoring the loop, which is what makes it the
compensation for arming blind. Attribution is still carried by the model and read by recording;
v1 ships no surface on which an operator reads who is talking, and there is no field in the
document or on the downlink that could tell them.

The indicator is the console's **only** motion, and it is a component so that *exactly one
place* is a path the styling check can name.

### Push-to-talk input

**Every source publishes a level and a liveness flag, never events**
([ADR-0021](docs/adr/0021-ptt-input-is-a-level-with-liveness.md)), and the client ORs the live
ones. Edges are lossy and their loss mode is an open mic; a level is self-correcting, and
liveness is what expresses *the headset was unplugged while you were holding it* — a property
of the source rather than of anything it could have sent. **A source that dies while keyed
forces an unkey**, because it leaves the OR rather than being remembered in it.

Two sources ship: the **on-screen key control**, live while its control is on screen, and the
**keyboard**, live while there is a window to listen on and a role to key under. A keystroke
footswitch is a keyboard and needed no code — it is the only PTT peripheral v1 supports, and
VoxLoop ships none. `$lib/input` is the way in and `web/eslint.input-seam.js` fails the build
for anything that reaches past it, which is what makes ADR-0020's promise — the Tauri wrapper
may only ever *add a source* — a check rather than a paragraph.

**Two modes and no third**: momentary (held) and latched (press to open, press to close). They
live in `web/src/lib/modes.js`, **above** the seam and above the names — Input is handed a
binding per name and reports under the same names, so nothing under `input/` has a word for
what any of them means, and `npm test` fails if that stops being true. **Latch has its own
binding and is read off the rising edge alone**
([ADR-0022](docs/adr/0022-latch-is-never-derived-from-a-momentary-press.md)) — no tap, no
double tap, no held duration, because a button that stopped reporting its release is
indistinguishable from a deliberate tap, and deriving latch from one would make an open mic
the failure mode of a hardware fault. **A single-button device is therefore momentary only.**

The defaults are `` ` ``, `` Shift+` `` and — for priority, below — ``Ctrl+` ``, changed from
the console and refused in three
places: Space activates focused controls, `CapsLock` does not reliably report being released,
and a key another mode already holds would be one press meaning two things. **PTT keys are
inert while focus is in a text field or on an interactive control**, so the key controls on
the transmit bar do not take focus when they are pressed. Autorepeat may not raise a level
that is low and a window losing focus drops what it was holding, which is
[v1 §7](docs/spec/v1.md#7-reconnection)'s stale-high rule arriving from the two cases in a
browser that produce it. A binding lasts as long as the console until
[#55](https://github.com/edwardhutchinson/voxloop/issues/55) persists it.

### Priority

**A priority transmission plays at full gain in every subscriber's ears, whatever they set that
loop's volume to, and that is all it does**
([ADR-0045](docs/adr/0045-priority-defeats-attenuation-and-nothing-else.md)). It lowers no
other talker, defeats no mute and compels no subscription. **Nothing in VoxLoop ducks**, in the
client or the server. Per-loop volume is the one attenuation nothing warns anybody about, and
priority covers exactly that gap.

**It is an act rather than an attribute**
([ADR-0046](docs/adr/0046-priority-is-keyed-not-held.md)): the third binding, ``Ctrl+` ``,
momentary only, and **it never latches**. It is a second level beside the ordinary one rather
than a mode, so `modes.js` absorbs it in two lines — `emitting = ordinary OR priority`,
`is-priority = priority`. Pressing it over a latch elevates the latched transmission and letting
go lowers it again; pressing it from cold keys and elevates, and letting go ends both. The
transmit bar carries a Priority control beside Key and Latch, drawn like the key control, and
the lamp says `Keyed at priority` when the server says so.

The client says the two levels separately: `key` and `unkey` carry the OR as before, and
`key-priority` and `unkey-priority` carry the priority level. Neither names a loop, because
**priority applies to the whole arm set**. There is one stream, fanned out at the server, so a
transmission cannot be priority on one armed loop and ordinary on another. That makes a wide arm
set the abuse vector, and it is **ungated by choice**: both messages are `Session`, open to
anyone holding `emit`, with no `control` gate and no flag on a role, a loop or a cell.

**It is audited instead: every press, with no minimum duration** (v1 §12). The entry names the
actor, the role, the armed loop set as it stood when the key went down, the time of the press
and how long it was held. A press ends when the key comes up, when the session ends and when
the socket goes, and each of those records it. A key held across an outage is suppressed until
released ([ADR-0043](docs/adr/0043-a-resume-restores-everything-except-the-key.md)), so the
socket closing ends the press.

**The mark is the talking indicator's one variant**
([ADR-0059](docs/adr/0059-a-priority-transmission-is-marked-wherever-it-lands.md)), and it
names nobody: `Talking — Priority`, on every loop the transmission reaches and on every console
whose reach holds that loop. That includes loops the receiver has at full volume, loops they have
muted, and loops they are not monitoring. It declares that somebody called this urgent; it does
not explain why the audio got louder. It lasts exactly as long as the press, with no minimum,
so **a sub-second press may leave no trace on any console**, and the audit log is then the only
record.

**The gain is read off the mark.** Each loop in the presence document carries `priority`, and
Audio plays a carriage at full gain when any loop it is heard on is marked and not muted,
bypassing loudest-wins rather than competing with it. The mark and the gain arrive as one fact,
so the first moments of an urgent call can play attenuated and unmarked together, because the
attribute rides signalling and the audio does not. The mark belongs to the loop, since the
console cannot tell one talker on a loop from another
([ADR-0033](docs/adr/0033-the-console-shows-that-someone-is-talking-never-who.md)). So anybody
else talking on a marked loop at the same moment plays at full gain as well. That raises them;
it lowers nobody.

**Cut beats priority** without any rule to say so. The mark is read off a transmission that is
landing, and Cut closes the fan-out, so a talker with no route is marked nowhere and has no
carriage to raise. The disconnected talker, whose fan-out closes by the same machinery, is how
that is tested until Cut arrives with [#51](https://github.com/edwardhutchinson/voxloop/issues/51).
There is **no personal opt-out**, because mute is already the escape.

## The admin console

Signing in as a system administrator opens the console, reachable from the lobby. It is
gated on the user's system-administration flag and **never on a role**, so an operator who
is also a sysadmin reaches it without dropping off the air
([v1 §9](docs/spec/v1.md#9-the-admin-console)). The flag is read from the store on every
request rather than carried in the cookie, so taking it away closes the console at once.

**Every page of the console has a URL**: `/admin/users` and a user's roles page under it,
`/admin/roles` with a role's reach and eligibility pages under that, `/admin/loops` and a
loop's column, and `/admin/grid`. Reloading one lands back on it, and a link to the loop
being discussed can be pasted into a chat. It is still **one bundle with client-side
routing** — moving between the pages does not reload the document, because the signalling
channel is one socket per tab and a full navigation would drop it. A page whose record is
gone shows the server's *there is no such loop* rather than a blank, and a page opened
without the flag says which flag was not held rather than showing empty tables.

**The lobby and the operating console have no URL, and that is the point of the split.**
Which of the two somebody is looking at is not a place they navigated to: it is whether they
hold a role, which is live state the server resolves. A bookmark to it would be a claim about
a session, and the console would have to bounce whoever followed it whenever the server
disagreed — a URL asserting a state nobody observed
([ADR-0016](docs/adr/0016-displayed-state-is-observed-or-asserted.md)). Reloading asks the
server where you are and lands you there. An administration page claims nothing live: it is a
read of configuration, it cannot go stale that way, and it is the thing somebody wants to
send to a colleague.

### Users

Users are created here and set their own password from an enrolment code, because VoxLoop
has no mail path — so a user created today cannot sign in until somebody issues them a code.
The account list says which users are awaiting enrolment and which already have a code
outstanding. Locking an account and forcing a password reset both end every sign-in the user
holds, immediately; issuing a code does neither, which is why forcing a reset is the separate
act it is. A user's **Roles** page is which positions they may assume, which is the only
authority a user carries — see [Eligibility](#eligibility).

**The last system administrator cannot be locked, deleted or stripped of the flag.** *Last*
counts flag holders and nothing else, deliberately: narrowing it to the ones who could sign
in today would let a box be emptied of administrators one permitted act at a time. Forcing a
password reset is not one of the three — the record and the flag both survive it — so
forcing one on a sole administrator leaves a deployment nobody can sign into to administer.
The bootstrap code is not re-minted, because somebody still holds the flag. Recovering from
that is shell access to the box, which is what the on-box CLI is for.

### Roles and loops

The console's other two pages are the configuration objects voice authority is expressed
over. A **role** is a staffable position with a limit on how many may occupy it at once —
single-occupant and multi-occupant roles are one concept under different limits, and a role
with no limit set admits anybody eligible for it. A **loop** is an audio conference and the
only thing voice can be addressed to. There is no loop kind, type or naming convention
anywhere, deliberately: a private room is an ordinary loop somebody configured
([ADR-0055](docs/adr/0055-there-is-no-conference-loop.md)).

Install seeds the `Observer` role and nothing else. Its reach is seeded only against the
loops present at install, and a fresh deployment has none — so a loop created afterwards
gets no Observer cell, and **a loop created after install arrives `unreviewed`** and says so
until an administrator has ruled on its column. Absent-because-denied and
absent-because-nobody-ruled render identically otherwise
([ADR-0015](docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md)).

The loop list is the deployment's **base loop order**, and it is administered rather than
derived — not alphabetical, and not creation order
([ADR-0053](docs/adr/0053-the-loop-order-is-complete-and-a-new-loop-lands-at-the-end.md)).
Arrange it with the arrows and save it: it is sent whole, as one decision and one audit
entry, and an order that does not name every loop exactly once is refused rather than
half-applied. A new loop lands at the end, because appending is the only honest placement
for something VoxLoop has been told nothing about.

Nothing on those two pages says which role may hear or say what on which loop. That is the
grid, below; who may assume a role is eligibility, below that.

### The grid

Voice authority is **one value per (role, loop) pair**, from an ordered four — `none`,
`monitor`, `emit`, `control` — each rung carrying everything below it
([ADR-0011](docs/adr/0011-a-permission-is-one-cell-on-the-grid.md)). An absent cell is
`none`. There is no second layer anywhere: no per-user grant, no per-user deny, no override,
no exception and no precedence rule, so evaluating a permission is one lookup. Granting one
person one extra loop always costs a role, deliberately.

It is administered **one row or one column at a time**
([ADR-0015](docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md)). A role's **Reach**
page is its row — every loop with what this role holds on it, in the base order — and a
loop's **Permissions** page is its column — every role with what it holds on that loop. Both
are lists at full size, because that is how administrators were found to read this: a
realistic pilot grid fills 167 of 300 cells, and past roughly thirty loops a row's header and
its far end cannot share a screen. Taking a permission away is setting `none`; there is no
separate act for it.

The **Grid** page is the whole matrix, and it is a reference view: the only place a
whole-configuration read is possible, which is a reviewing act rather than an administering
one. Nothing is edited there.

A loop nobody has ruled on shows as `unreviewed`, and its cells are **enforced as `none` on
every rung whatever they are set to**. It is ruled on when every role's cell has been set, or
by dismissing the mark from the loop's own page — which records a deliberate `none` against
every role left alone. Either way the mark is cleared **per loop and never per cell**:
setting one cell does nothing to it while another role is unruled. It is a display state and
an administrator's prompt throughout: the evaluator cannot tell an unreviewed loop's cell
from a deliberate `none`, and does not try.

Every write is audited with the record before and after and the **blast radius** — what the
change does to anything live. No session exists yet, so that radius is empty; the shape is
there because the write and its audit entry commit in one transaction, and the radius is a
value the write is handed rather than a field it may omit
([ADR-0039](docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md)).

### Eligibility

**Eligibility is the unconditional grant permitting a user to assume a role**, and it carries
no permissions of its own. It says somebody may sit in a seat; what the seat can hear, say
and command is the grid, and nothing about a grant widens a cell. Revoking it from somebody
occupying the role ends their occupancy immediately, with the reason shown to them — the
configuration write is here, and the half that ends a live occupancy arrives with sessions.

**It is deliberately not a second matrix.** Rendered as one, 190 users against 15 roles was
the least legible object the console prototype produced
([ADR-0015](docs/adr/0015-the-admin-console-reads-one-row-at-a-time.md)), so it is
administered from **two directions and no third**: a role's **Eligible** page answers *who
may assume this*, and a user's **Roles** page answers *which roles may this person assume*.
Each lists the grants and nobody else — whoever is not on it is picked from a box, which is a
list to search rather than a wall to read. There is no whole-eligibility read anywhere in the
API, which is the difference between this and the grid: a matrix is a reviewing act at
fifteen roles by twenty loops, and it is not one at a hundred and ninety users.

**Every user record starts eligible for `Observer`**, seeded as part of creating it — by the
console, by the on-box CLI and by the bootstrap route alike. A deployment that renamed or
deleted `Observer` has decided what its listen-only position is, and nothing is seeded rather
than VoxLoop guessing which role replaced it.

**Reach is never composed across the roles somebody may assume.** A session is bound to one
role, so a person's reach is only ever one row's worth at a time, and a page that added them
up would display authority nobody can hold. Answering *what can this person do* is their
Roles page and then that role's Reach — one extra hop, taken knowingly.

Granting and revoking are separate audit events, unlike a grid cell's one: a cell always
holds one of four values and granting is setting it, but an eligibility is present or absent,
and revoking has a consequence granting cannot have.

## Working on the console

`scripts/dev` rebuilds the console and embeds it, which is the release path and always
truthful. For hot reload, development is two processes instead — run the binary as above,
then:

```sh
cd web && npm run dev
```

Vite serves the console on port 5173 with hot reload and proxies `/api` to the binary on
8443. Whether the `Secure` sign-in cookie survives that depends on your browser treating
`http://localhost` as a trustworthy origin; if signing in bounces you straight back to the
form, that is what happened, and `scripts/dev` is the way round it.

Formatting and lint:

```sh
cd web
npm run format        # Prettier, writing
npm run format:check  # Prettier, asking — this is what CI runs
npm run lint          # ESLint
```

Prettier is configured to the console as it was already written — tabs, single quotes, 100
columns — so it reflows nothing and every future diff is the change rather than the
whitespace around it.

ESLint's rule set is small on purpose, with one exception that is not a style opinion:
**nothing outside Input may import Input's internals.** Input is the only client seam with
real variation, and [ADR-0020](docs/adr/0020-the-browser-is-the-client.md) promises the Tauri
wrapper may only ever *add a source* to it, so
[ADR-0061](docs/adr/0061-module-privacy-is-the-seam-enforcement.md) makes that promise a
failing build rather than something review has to catch. Import `$lib/input`; nothing
beneath it.

### Styling

There is no CSS framework and no component library. Svelte scopes CSS per component, so the
usual case for a utility framework — escaping the cascade — does not apply; what a framework
would have bought is a fixed scale, and `web/src/app.css` is that scale, plus the palette and
the furniture every page shares. The console is dark only
([ADR-0069](docs/adr/0069-styling-is-scoped-css-over-one-token-file.md)).

The rules for writing a component — and for adding an icon to `icons.js`, which holds
hand-picked Lucide path data rather than a dependency — are in
[`docs/agents/styling.md`](docs/agents/styling.md). The ones a machine can read are enforced
by `npm test`: no literal spacing, type or radius values, no colour outside `app.css`, and no
`:global()` in a component.

## Tests

```sh
cargo test                      # the binary, without the console embedded
cargo test --features embed-web # the same, plus the embedded bundle (needs npm run build)
cd web && npm test              # the console: the Input seam and the lint rule behind it, the
                                # styling standard, the icons, the two views of the loop list,
                                # and what the client says over the signalling channel
```

Tests run against the real store: each one opens a temporary SQLite file, migrates it and
throws it away. There is no in-memory repository and there will not be one
([ADR-0064](docs/adr/0064-tests-run-against-the-real-store.md)).

**A rule about what happens when a write fails is tested by making that write fail.** The
personalisation write is best effort and must never be able to fail a live act, and there is
no in-memory store to break — so the test installs a trigger on the real one that refuses
that one table, the way the audit log's own triggers refuse an amendment. It is the only hole
in Configuration's seam, it is `#[cfg(test)]`, and it lives inside the module because the
connection it needs is that module's alone.

The **media plane** is one of exactly two seams with a fake, and the fake is a **recorder**
rather than a simulation: it writes down what it was told and does nothing else, so a test
asserts on the instructions rather than on a transport. That is what keeps every routing rule
testable with no worker running — and it is why the fake must never grow an opinion about
what it was handed. One test is the exception and runs a real Worker, Router and
`WebRtcServer` on whatever port is free, because a seam with nothing real behind it is a
reserved space rather than a proven boundary — it asserts the part the recorder cannot, which
is that a session gets two transports and that what its client needs in order to build the far
end is composed and put on that session's own channel.

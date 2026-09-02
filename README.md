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
bound to, its **media path state**, and the loops in reach with **which of them the session
is monitoring**; arms, staffing state, loop health and the audience land in it one ticket at
a time.

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

A session gets a **WebRTC transport of its own, bound to it at creation**, opened by the
assume that minted the session and closed by whatever ends it. One Worker, one Router and one
shared `WebRtcServer` port carry all of them, because a loop is not a transport primitive: a
transport belongs to one router, so a router per loop would give somebody monitoring six
loops six ICE and DTLS sessions.

**No audio is routed yet.** What exists is the pipe and a reading of its condition. The
media plane's interface names domain operations only — open a path, close a path, make this
audience hear this talker — and **it executes routing rather than computing it**
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

The client half of the report exists on the socket and nothing drives it yet: the peer
connection it would read is the Audio module's, and that arrives with the client's audio. So
in a running deployment today the merged answer is `lost`, honestly.

**One session's media path going is not the worker going**, and the two are opposite
decisions. A session whose transport has failed keeps its role indefinitely: the operator is
present, reading a working console that can say exactly what is wrong, and ending it for them
takes the decision from the person best placed to make it, possibly mid-fix. The worker going
is the deployment losing its purpose, with nobody left to leave the judgement with — so live
state moves first, so the last thing every console is told about itself is true, and then the
unit goes down.

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
wording. The armed set and the key state arrive with arming and keying, and the two audience
counts after that; what it carries today is **media path state**, because that is the first
thing it has to say that withdraws emission.

**Nothing renders optimistically**
([ADR-0016](docs/adr/0016-displayed-state-is-observed-or-asserted.md)). Neither view keeps any
state of its own: what is on screen came out of the last document and can be nothing else, so
a toggle will visibly lag a round trip and switching views loses nothing. The console
remembers exactly one thing, and it is which view is showing — a fact about the reader rather
than about the world.

**The console renders no motion.** Motion is permitted in exactly one place, the talking
indicator ([ADR-0033](docs/adr/0033-the-console-shows-that-someone-is-talking-never-who.md)),
which does not exist yet — so `npm test` refuses `animation`, `transition`, `@keyframes` and
Svelte's motion directives outright, and the indicator will be written into that check rather
than around it.

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
along with the rest of the personalisation rules — per-loop volume, loop order and the
default view.

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
cd web && npm test              # the console: the seam rule, the styling standard, the icons,
                                # the two views of the loop list, and what the client says
                                # over the signalling channel
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
reserved space rather than a proven boundary.

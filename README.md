# VoxLoop

A software voice loop system. Users assume roles, a role's reach is read off the (role,
loop) permission grid, and that grid is the only place voice authority is configured.

The design lives in [`docs/spec/v1.md`](docs/spec/v1.md); read
[`docs/spec/modules.md`](docs/spec/modules.md) first, because it says what the system *is*
rather than why. [`docs/adr/`](docs/adr/) holds the reasoning.

## What runs

One Rust binary — console, API, signalling, permission enforcement and TLS — beside the
mediasoup worker, the text-to-speech sidecar and one SQLite file. No reverse proxy, no Node
runtime, and one systemd unit ([ADR-0040](docs/adr/0040-one-binary-one-unit-four-moving-parts.md)).

## Building

A bare `cargo build` needs nothing but Rust. It does not embed the console, and does not
need Node or `web/dist` to exist.

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
([ADR-0023](docs/adr/0023-sign-in-is-to-the-application-and-a-role-is-assumed.md)). Assuming
one is a separate act, and it is the next ticket.

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

## The admin console

Signing in as a system administrator opens the console, reachable from the lobby. It is
gated on the user's system-administration flag and **never on a role**, so an operator who
is also a sysadmin reaches it without dropping off the air
([v1 §9](docs/spec/v1.md#9-the-admin-console)). The flag is read from the store on every
request rather than carried in the cookie, so taking it away closes the console at once.

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
                                # and what the client says over the signalling channel
```

Tests run against the real store: each one opens a temporary SQLite file, migrates it and
throws it away. There is no in-memory repository and there will not be one
([ADR-0064](docs/adr/0064-tests-run-against-the-real-store.md)).

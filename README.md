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

VoxLoop terminates TLS itself, so it needs a certificate before it will start. For local
work, any self-signed one will do:

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

## The admin console

Signing in as a system administrator opens the console. It is gated on the user's
system-administration flag and **never on a role**, so an operator who is also a sysadmin
reaches it without dropping off the air
([v1 §9](docs/spec/v1.md#9-the-admin-console)). The flag is read from the store on every
request rather than carried in the cookie, so taking it away closes the console at once.

Users are created here and set their own password from an enrolment code, because VoxLoop
has no mail path — so a user created today cannot sign in until enrolment lands. Locking an
account and forcing a password reset both end every sign-in the user holds, immediately.

**The last system administrator cannot be locked, deleted or stripped of the flag.** *Last*
counts flag holders and nothing else, deliberately: narrowing it to the ones who could sign
in today would let a box be emptied of administrators one permitted act at a time. Forcing a
password reset is not one of the three — the record and the flag both survive it — so
forcing one on a sole administrator leaves a deployment nobody can sign into to administer.
The bootstrap code is not re-minted, because somebody still holds the flag. Recovering from
that is shell access to the box, which is what the on-box CLI is for.

Every write is audited with the record before and after and the **blast radius** — what the
change does to anything live. No session exists yet, so that radius is empty; the shape is
there because the write and its audit entry commit in one transaction, and the radius is a
value the write is handed rather than a field it may omit
([ADR-0039](docs/adr/0039-live-state-is-in-process-behind-one-state-authority.md)).

## Working on the console

Development is two processes; release is one artefact. Run the binary as above, then:

```sh
cd web && npm run dev
```

Vite serves the console with hot reload and proxies `/api` to the binary on port 8443.

## Tests

```sh
cargo test                      # the binary, without the console embedded
cargo test --features embed-web # the same, plus the embedded bundle (needs npm run build)
```

Tests run against the real store: each one opens a temporary SQLite file, migrates it and
throws it away. There is no in-memory repository and there will not be one
([ADR-0064](docs/adr/0064-tests-run-against-the-real-store.md)).

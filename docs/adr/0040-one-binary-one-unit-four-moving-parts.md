# One binary, one service unit, four moving parts

A VoxLoop deployment is exactly four things: **the Rust binary** (console, API, signalling, permission enforcement, TLS), **the mediasoup C++ worker**, **the text-to-speech sidecar**, and **one SQLite file**. [#13](https://github.com/edwardhutchinson/voxloop/issues/13) asked for the minimum viable set of moving parts, and this is where that line is drawn.

## The binary terminates TLS

[ADR-0026](./0026-one-credential-and-the-media-path-carries-none.md) makes HTTPS mandatory even on a LAN, because a `Secure` cookie is not sent otherwise. The conventional answer is nginx or Caddy in front, and it buys real things: certificate reloading without a restart, HTTP/2, and an operational surface a customer's ops team already knows.

It is rejected because it is a fifth moving part — another process to install, configure and patch inside a possibly air-gapped network, and a second place a WebSocket upgrade can be misconfigured, on a system where losing the signalling channel withdraws the emission path entirely ([ADR-0018](./0018-no-signalling-channel-means-no-emission-path.md)). **TLS terminates in the Rust binary via rustls.**

**Where the certificate comes from — internal CA or self-signed — is not settled here.** That belongs with deployment and packaging; this ADR rules only on who terminates.

## The application supervises its own subprocesses; systemd supervises the application

mediasoup's crate already spawns and supervises the C++ worker as a child. *That premise is wrong, and [ADR-0070](./0070-the-mediasoup-worker-is-a-thread-of-this-process.md) corrects it: the Rust API links the worker in and runs it on a thread of this process. The rule below is unchanged — the health of a subprocess is observed rather than returned — and it now applies to the sidecar alone.* The text-to-speech sidecar ([ADR-0030](./0030-speech-synthesis-is-a-swappable-sidecar.md)) follows the same pattern rather than becoming a second service unit: **the Rust binary spawns and supervises it, and systemd supervises the Rust binary.** One unit file to install.

The deciding argument is not tidiness. The map records that a dead sidecar means **every announcement in the deployment is silently lost** — [ADR-0029](./0029-an-announcement-is-an-ordinary-transmission.md) gives callers only a synchronous accept, so synthesis failure is invisible to them. A parent that owns the process knows its health as a matter of course and can surface it in the admin console; a sibling service unit would need a separate probe that someone has to remember to write.

## Configuration splits on auditability, which coincides with bootstrapping

Two kinds of configuration were being run together, and the line between them is [ADR-0028](./0028-the-audit-log-records-decisions-not-traffic.md).

**Anything an administrator changes through the console is in the database**, because ADR-0028 requires configuration changes to be recorded with before and after, and a file edit cannot be audited. That covers users, roles, loops, the grid, eligibility, service principals and the pronunciation dictionary.

**Anything needed to reach the database is in a file** — listen ports, TLS certificate and key paths, mediasoup's announced address and its single `WebRtcServer` port, the sidecar's loopback address, the SQLite path, log level. A **TOML file with environment-variable overrides**, read once at startup.

The rule holds at the awkward case, which is why it is trustworthy: ADR-0030 makes the pronunciation dictionary "a configuration change… audited by ADR-0028 with no special handling", and the rule puts it in the database, correctly.

## Consequences

- **A certificate change requires a restart** in v1, since configuration is read once at startup — and by [ADR-0039](./0039-live-state-is-in-process-behind-one-state-authority.md) a restart ends every session. Renewal is therefore a scheduled maintenance act, which is an argument for a long-lived internal CA certificate over anything short-dated, and a constraint the deployment work inherits.
- **The firewall ask stays as ADR-0006 left it**: one inbound TCP port for HTTPS and the signalling WebSocket, and one inbound UDP port for media with TCP on the same port.
- **Nothing about this stack forecloses the recording seam.** [ADR-0009](./0009-recording-taps-plain-rtp-on-loopback.md)'s sink attaches to a loopback RTP port and is a separate process in whatever language suits it; it becomes a fifth moving part only if and when it is built, deliberately.
- **The shipped artefact set is a binary, a worker, a sidecar with its model file, one unit file and one TOML file.** Signing, installation and the update path belong to the deployment and packaging work, which now knows exactly what it is packaging. *[ADR-0070](./0070-the-mediasoup-worker-is-a-thread-of-this-process.md) takes the worker out of that list: it is inside the binary, so there are three artefacts and not four.*

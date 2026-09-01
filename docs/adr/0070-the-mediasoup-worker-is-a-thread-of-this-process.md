# The mediasoup worker is a thread of this process, not a child of it

[ADR-0006](./0006-mediasoup-carries-the-audio.md) chose mediasoup and named its Rust API as the way in. [ADR-0040](./0040-one-binary-one-unit-four-moving-parts.md) then wrote the deployment down as **four moving parts** — the binary, the mediasoup C++ worker, the sidecar, one SQLite file — with *the binary supervises its own subprocesses; systemd supervises the binary*.

**The mediasoup C++ worker is not a subprocess of the Rust binary, and it cannot be made into one.** That was assumed from mediasoup's Node.js implementation, which spawns `mediasoup-worker` as a child and talks to it over a socketpair. The Rust crate does something else: `mediasoup-sys` builds `libmediasoup-worker` as a **static library**, `mediasoup` links it, and `WorkerManager::create_worker` runs `mediasoup_worker_run` on an **OS thread of this process**, with the channel that would have been a pipe replaced by two function pointers. There is no pid, no exec, no exit status.

This was found while building the media plane rather than reasoned about in advance, and it is recorded because two documents currently say otherwise.

## What supervision means now

**The same thing, one layer in.** What ADR-0040 wanted was that the audio path's health is *observed rather than assumed*, and that nothing else in the deployment has to know how to restart it. Both still hold:

- `Worker::on_dead` is the signal, and it fires once. It replaces waiting on a pid.
- The report goes onto the media plane's channel like every other fact about the audio path ([ADR-0062](./0062-the-call-graph-is-acyclic-and-effects-modules-are-sinks.md)), and something above decides what it means. **The health of a sink is observed, never returned**, and that rule did not depend on where the sink lived.
- Every session's media path state drops to `lost` ([ADR-0042](./0042-the-media-path-has-its-own-ladder.md)) **first**, and then the unit goes down. The order matters: the shutdown is graceful, so the last thing every console is told about itself is true rather than stale.

**A dead worker stops the binary, and a dead media path for one session does not.** The two look alike on the console and are opposite decisions. One session's transport failing is announceable, survivable and the operator's to judge — ADR-0042 accepts a permanently `lost` path rather than reaping the session, because the operator is present, reading a working console, possibly mid-fix. The worker going is not one session's problem: every transport is gone, no new one can be built, and there is nobody to leave the judgement with because the judgement is *this deployment cannot do its job*.

**What is lost is isolation, and it is worth saying plainly.** A segfault in the C++ worker takes the Rust binary with it, where a subprocess would have died alone and been restarted under a console that stayed up. Against that: a restart ends every session anyway ([ADR-0039](./0039-live-state-is-in-process-behind-one-state-authority.md)), because occupancy restored without an audio path is the lie this product exists to avoid — so the console that stayed up would have had nothing true to show. The blast radius of a worker crash was already *everything*; this makes it visibly so rather than subtly so, and systemd restarts one unit either way.

**No in-process restart is attempted.** A new Worker means a new Router, and every transport, producer and consumer belonged to the old one — so recovering in place is rebuilding every session's media path from a process that has just proved it can lose them. Letting the unit go down and come back is the same outcome, arrived at by a path that is exercised on every deployment, and it is what the binary does: it stops, non-zero, and systemd brings it back.

**The exit code tells the two endings apart.** A deliberate stop is a success and the unit stays down; a worker's death is a failure and systemd restarts it. A binary that exited zero either way would make *restart on failure* mean *restart always*.

## Consequences

- **The shipped artefact set loses an item.** There are three moving parts with a separate life, not four: the binary (with the worker inside it), the sidecar with its model file, and the SQLite file. The unit file is still one, and the firewall conversation is unchanged.
- **Building VoxLoop now needs a C++ toolchain and Python**, because `mediasoup-sys` compiles the worker at `cargo build` time and bootstraps meson and ninja to do it. `cargo build` no longer *needs nothing but Rust*, and the README says so.
- **The sidecar is genuinely a subprocess and the worker is genuinely not**, so ADR-0040's supervision sentence is true of one of them. Anything written about *the subprocesses* should name which.
- **`libmediasoup-worker` is linked into the binary**, so its licence travels with the artefact and an upgrade of the pinned crate is a rebuild rather than a file swap. That is the same scheduled work ADR-0006's exact pin already implied, with one fewer way to get it wrong.
- **A load test measures one process, not two.** The worker's CPU is this process's CPU, so the ~3,000–4,000 allocated consumers ADR-0006 wants cleared before v1 will show up in the binary's own numbers, and a thread pinned by `WorkerSettings::thread_initializer` is the knob if that becomes the problem.

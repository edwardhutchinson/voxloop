# Speech synthesis is a swappable sidecar behind a text-to-audio seam

VoxLoop must run inside a customer's network with no external runtime dependency, possibly air-gapped, so hosted speech synthesis is not available to us. That constraint is satisfiable: neural text-to-speech that runs on a plain CPU with no GPU is mature, and the candidates synthesise roughly five to ten times faster than real time on ordinary desktop hardware. Establishing that a viable local option *exists* was [#11](https://github.com/edwardhutchinson/voxloop/issues/11)'s requirement; naming one irreversibly was explicitly not.

**Synthesis therefore sits behind a seam, in the same posture as the identity front door of [ADR-0024](./0024-identity-is-a-replaceable-front-door.md):** a long-running sidecar process on loopback, which VoxLoop hands text and which returns 48 kHz mono PCM — the format [ADR-0010](./0010-opus-mono-and-the-latency-budget.md) already encodes everything into. Swapping engines means writing a process that answers that call. Nothing in VoxLoop changes, which is what makes it a seam rather than a stated intention.

A subprocess per announcement — text on stdin, PCM on stdout, non-zero exit for failure — is a simpler contract and was preferred until the cost showed up: every candidate engine loads a neural model at startup, roughly a second, which a per-announcement subprocess pays **every single time**. That turns a 200 ms synthesis into a 1.2 s one and makes ADR-0029's backlog arithmetic considerably less pleasant. A supervised process and a loopback port are things the single-box deployment already has.

**The engine is not pinned; its licence posture is.** The shipped default must be permissively licensed — Apache-2.0 or MIT — because an engine's licence is not cheaply reversible once a customer has the box, and a proprietary product shipping GPL code is a conversation with a customer's legal team that a permissive licence simply never has. Kokoro-82M (Apache-2.0) is the current candidate. Piper is the better-known option and is a legitimate substitution for a customer who wants it, but active Piper is **GPL-3.0** — the MIT `rhasspy/piper` repository went read-only in October 2025 — so it is a sidecar-only engine, which the seam happens to make painless.

**The whole clip is rendered before emission begins.** Streaming as it synthesises would shave perhaps 200 ms off an announcement that is by definition not conversational, and would cost two things worth more: the **duration is known before anything goes on air**, which is what makes ADR-0029's backlog-in-seconds computable at all, and a synthesis failure happens before the loop hears anything rather than halfway through a sentence.

**One voice, hardcoded in the sidecar.** No per-role voice, no per-request selection. A caller that could choose a voice could choose one that sounds like a human position, and operators do not identify a source by its timbre — they identify it by the words, which is what ADR-0029's server-composed `"Ground Alarms: ..."` prefix is for.

## The pronunciation dictionary

Operations vocabulary is acronyms, and a neural engine will mangle most of them: `T-5 minutes` comes out as *"tee five minutes"*, and `GS/OD` as noise. Leaving this to callers — write it phonetically, it's your problem — was the v1 default until it became clear that a pilot site hits it in the first week and that role names are acronyms too, so the problem arrives on **every** announcement whether the caller is careful or not.

So VoxLoop carries **one deployment-wide pronunciation dictionary**: literal whole-token replacements, longest match wins, applied left to right. Regular expressions were rejected — they would let one admin handle `T-5`, `T-10` and `T-90` with a single rule, and would equally let one bad pattern make every announcement in the deployment unintelligible at a moment nobody is testing.

**It is applied to the whole composed string, prefix included, immediately before the text crosses the seam.** One transform in one place, engine-independent, and nothing for an administrator to reason about twice. Editing it is a configuration change and is therefore audited by [ADR-0028](./0028-the-audit-log-records-decisions-not-traffic.md) with no special handling.

## Consequences

- **Announcement text is written to be spoken, not read.** The dictionary handles the deployment's standing vocabulary; anything else is the caller's to phrase.
- **SSML is not accepted.** It drags in a specification and per-engine support differences, and the dictionary covers the case it would have been used for.
- **The sidecar is a second process to build, ship and supervise inside the customer's network**, alongside the mediasoup worker that [ADR-0006](./0006-mediasoup-carries-the-audio.md) already requires. It carries a model file, which is the largest artefact in the deployment.
- **A synthesis failure is invisible to the caller**, since ADR-0029 gives only a synchronous accept. Rendering first at least guarantees the failure is total rather than a half-spoken sentence on a live loop.

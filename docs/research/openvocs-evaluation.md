# openvocs as a foundation for VoxLoop

Research note for [issue #15](https://github.com/edwardhutchinson/voxloop/issues/15), against the constraints on
the wayfinder map ([issue #1](https://github.com/edwardhutchinson/voxloop/issues/1)). Follows on from the prior-art
survey ([issue #4](https://github.com/edwardhutchinson/voxloop/issues/4)) that surfaced openvocs, and is read
alongside the audio-transport note ([issue #3](https://github.com/edwardhutchinson/voxloop/issues/3)).

**Researched:** 2026-08-15, against a clone of `github.com/openvocs/openvocs` at
[`83ad2da`](https://github.com/openvocs/openvocs/commit/83ad2daf12a13504b5285af951e230c73dec137b)
(`main`, 2026-05-08, version 2.6.2), plus the branches `development` (2026-08-06),
`release_candidate` (2026-08-10), `aes`, `spawn`, `mixer_autostart`, `cleanup`, `recorder-fix`.
Every claim below cites the file it came from, the GitHub API, or a first-party web page. Where the code and
the documentation disagree, both are reported and the code wins.

---

## 0. Verdict

**Reference, not foundation — but a far more valuable reference than #4 realised, and one that ships two
specific mechanisms VoxLoop should copy outright.**

Three findings drive this, in order of weight:

1. **The recorder is not in the open-source repository, and it is not missing by accident — it is the paid
   module.** `src/service/makefile` builds six services; none of them is a recorder or a SIP gateway
   ([`src/service/makefile:29-38`](https://github.com/openvocs/openvocs/blob/main/src/service/makefile)). The
   repository contains only the *controller* that tells an external recorder to start, the event schema, and
   the admin GUI. [openvocs.com](https://openvocs.com/) sells the "openvocs Recorder Module" and "openvocs SIP
   Gateway" as **"Commercial Enterprise Extensions … licensed annually per Operations Center"**, and its
   [imprint](https://openvocs.com/imprint.html) names a single natural person — **Markus Töpfer**, author of
   425 of the 472 commits on `main`. VoxLoop's *one* named hard architectural constraint beyond self-hosting is
   the decoded-audio recording seam. In openvocs, that seam terminates in a closed binary sold by the project's
   own lead maintainer. Adopting openvocs as a foundation means either paying that vendor for the compliance
   feature, or writing the missing daemon yourself against an event schema you do not control.

2. **Its transport answer is the one VoxLoop's own transport research already ruled out — and it costs one OS
   process per connected user.** openvocs mixes server-side, per user: `ov_mc_backend_acquire_mixer` is called
   once per media session at session creation, before any loop is joined
   ([`src/lib/ov_vocs/src/ov_vocs.c:1528`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs/src/ov_vocs.c)),
   and the shipped systemd target statically pre-starts **50** mixer processes
   ([`resources/systemd/global/ov_mc_mixer.target`](https://github.com/openvocs/openvocs/blob/main/resources/systemd/global/ov_mc_mixer.target)).
   The README states the consequence plainly: *"**BEWARE** The maximum number of users is determined by the
   number of mixer processes started."* VoxLoop's envelope is ~200 connected. That is 200 processes on the
   default architecture, against a stated minimum spec of *"at least 4 users that speak in parallel"* on 4
   cores / 8 GB. Dynamic mixer spawning landed on `development` on 2026-06-29 and is not on `main`.

3. **Nothing VoxLoop calls a differentiator exists in it.** Mid-event revocation does not propagate to live
   sessions; there is no request-access, no occupancy or vacancy concept, no TTS or event-injection API, no
   unfocused desktop PTT, and no per-participant listen/talk state in the participant list. Each of these is
   established by absence in §6 below, not assumed.

**What it *is* worth**, and this is not a consolation prize:

- Its **user / role / loop** model and its `none | recv | send` per-(role, loop) permission enum are exactly
  the model #4 recommended, implemented and flown. VoxLoop should adopt the shape without adopting the code
  ([§4.3](#43-data-model-and-persistence)).
- Its **internal multicast RTP bus** is a materially better decoded-audio seam than any SFU tap surveyed in #3:
  a loop *is* an IP multicast group, so any number of sinks — recorder, transcriber, ALSA gateway, another
  site — attach by joining a group, with zero changes to the media path ([§4.2](#42-audio-transport)). This
  design idea is free to borrow and is the single best thing in the codebase.
- **WebHID** for headset push-to-talk buttons
  ([`src/HTML/app/vocs/views/loops/js/ui/ptt_bar.js:214-252`](https://github.com/openvocs/openvocs/blob/main/src/HTML/app/vocs/views/loops/js/ui/ptt_bar.js))
  directly contradicts #5's conclusion that *"headset PTT buttons are uncatchable as keystrokes"*. They are not
  uncatchable — they are catchable as HID input reports, in a browser, today. This changes the
  desktop-client calculus and is written up in [§7](#7-the-desktop-problem).

The honest summary is that openvocs is a working, operationally-proven, single-maintainer research artefact
with a commercial layer bolted onto its most important feature. It is the best available evidence that
VoxLoop's model is right. It is not a base to build on.

---

## 1. Method and source quality

| Source | Type | Confidence | Note |
|---|---|---|---|
| openvocs source tree at `83ad2da` (`main`, v2.6.2) | Primary — the code itself | **High** | Cloned and read. Every architectural claim traces to a file:line. |
| openvocs `README.md`, `Handbook_DRAFT/User_Handbook_v1_2.md`, service `README`s | First-party documentation | **High** for intent, **Medium** for accuracy | Both are marked ***DRAFT***. The README's client-API section documents events that do not exist in the code (§4.6). |
| `git log` / `git shortlog` / GitHub REST API for `openvocs/openvocs` | Primary metadata | **High** | Caveat: the GitHub history is truncated — see §3.1. |
| [openvocs.com](https://openvocs.com/) and its [imprint](https://openvocs.com/imprint.html) | First-party commercial site | **High** | The imprint is a legally-required German *Impressum*; treat the named person as authoritative. |
| [DLR press release](https://www.dlr.de/en/rb/latest/news/2025/gsoc-publishes-open-source-software-platform-openvocs-for-communication-in-mission-control) and [DLR project page](https://www.dlr.de/en/rb/research-operation/research-projects/control-center-technology/openvocs-r) | Official institutional | **High** | DLR classifies openvocs under *research projects*, not products. |
| [DLR elib 201558](https://elib.dlr.de/201558/) — Schiffner, Bertard, Beer & Töpfer, *Openvocs, a light-weight Voice Communication System for Space Mission Control*, SpaceOps 2023 | Conference paper | **High** for existence, **Low** for content | **Full text is DLR-internal only.** Only the abstract was readable. No claim below rests on it. |

**What I could not reach.** `openvocs.org` refused TLS to every fetch attempt from this environment
(`TLSV1_ALERT_INTERNAL_ERROR`, both via WebFetch and `curl -k`); its research page is therefore known only
through search-engine indexing and is not cited for any load-bearing claim. The SpaceOps 2023 paper full text
is access-restricted. No pricing was published for the commercial modules. There is no public benchmark of
openvocs at any user count.

**A note on branches.** `main` is not the development frontier. The GitHub API reports `pushed_at`
2026-08-10 while `main`'s tip is 2026-05-08; the live work is on `development` (2026-08-06) and
`release_candidate` (2026-08-10). Judging the project by `main` alone would have understated its activity, and
judging the *architecture* by `development` alone would have overstated what a third party actually gets from
a release. Both are reported.

---

## 2. Licence

### 2.1 What it is

**Apache License 2.0**, unambiguously and consistently.

- `LICENSE` at the repository root is the verbatim Apache-2.0 text.
- The GitHub API reports `"license": {"spdx_id": "Apache-2.0"}` for the repository.
- **898 of the 904 `.c`/`.h` files carry the full Apache-2.0 header** with
  `Copyright (c) <year> German Aerospace Center DLR e.V. (GSOC)`. The only files without it are vendored
  third-party (`sqlite3.c`, `sqlite3.h`), one generated file (`ov_version.c`) and three files under
  `src/tools/ov_rtp_cli/`. There is no copyright-holder other than DLR on any first-party file.
- `README.md` and the handbook carry SPDX headers: `SPDX-FileCopyrightText: 2025 German Aerospace Center (DLR)`,
  `SPDX-License-Identifier: Apache 2.0`.

### 2.2 What it permits for a product deployed into a paying customer's site

Everything VoxLoop would need. Apache-2.0 grants perpetual, worldwide, royalty-free rights to reproduce,
prepare derivative works of, publicly display, sublicense and **distribute** the work and derivative works, in
source or object form (§2), plus an express patent grant (§3). Deploying a modified build into a customer's
site is a distribution, and it is permitted.

**Modifications need not be published.** Apache-2.0 §4 requires only that you (a) give recipients a copy of
the licence, (b) mark changed files as carrying your changes, (c) retain existing copyright/patent/attribution
notices, and (d) propagate any `NOTICE` file. There is no source-disclosure obligation of any kind, and no
network-use clause. A closed VoxLoop containing Apache-2.0 openvocs code is lawful provided the notices ride
along.

**There is no copyleft reach into VoxLoop's own code** — not by linking, not by forking. Apache-2.0 is
permissive. The reverse direction is the constrained one, and openvocs' own maintainers have vendored the ASF's
policy notes about it into
[`LICENSES/APACHE_copyright_statements/`](https://github.com/openvocs/openvocs/tree/main/LICENSES/APACHE_copyright_statements)
(*"Apache 2 software can therefore be included in GPLv3 projects … However, GPLv3 software cannot be included
in Apache projects"*). Those files are **ASF guidance the DLR team copied for its own use, not terms imposed on
downstream users** — they create no obligation on VoxLoop. Worth knowing only because they signal the team
takes licence hygiene seriously.

**Third-party components** are all inbound-compatible and all vendored, with licences enumerated under
`LICENSES/`: Opus (BSD-3-Clause), SQLite (public domain), normalize.css, fluent-ui-system-icons,
simple-keyboard 3.8.0, loading.io (CC0), the Abel font (SIL OFL). The one to note is **systemd (LGPL-2.1)**:
`libsystemd-dev` is a build dependency for journal logging
([`resources/debian/debian-packages.yml`](https://github.com/openvocs/openvocs/blob/main/resources/debian/debian-packages.yml)).
LGPL-2.1 via dynamic linking to a distro-provided shared library imposes no source obligation on VoxLoop, but
if you ever statically link or ship a modified libsystemd you inherit LGPL relinking duties. Same story for
`libldap2`/`libsasl2` (OpenLDAP Public License) and `libsrtp2` (BSD-3-Clause).

### 2.3 Contribution terms, trademark, DLR-specific conditions

- **No CLA, no DCO, no `CONTRIBUTING.md`, no `CODE_OF_CONDUCT.md`, no `GOVERNANCE.md`, no `SECURITY.md`, no
  `CODEOWNERS`, and no `NOTICE` file.** I searched the whole tree; the only `CONTRIBUTING.md` present belongs
  to the vendored `simple-keyboard` package. Inbound contributions are therefore governed by GitHub's default
  terms and Apache-2.0 §5 (contributions are licensed inbound under the same licence absent a separate
  agreement). This is *convenient* for a contributor and *a red flag* for anyone planning to depend on the
  project: nobody has written down who decides anything.
- **Trademark is a real constraint.** The mark is written `OPENVOCS®` throughout the README and handbook, and
  Apache-2.0 §6 explicitly grants **no** trademark rights. You may fork the code; you may not call the result
  openvocs, and given the same name is now used by a commercial venture (§3.4) the mark is being actively
  used in trade. VoxLoop's naming is already distinct, so this bites only on attribution wording.
- **No DLR-specific condition exists in the repository** — no export-control notice, no non-commercial
  restriction, no field-of-use limit, no institutional clause. The README's Citation section is an unfilled
  `***TODO*** ... here Bibtex or Zenodo Link`, so even the academic-citation request is unenforceable and
  unstated.

**Licence conclusion: clean.** The licence is the *least* of the problems here. Anything the rest of this note
recommends is legally available.

---

## 3. Project health

### 3.1 Cadence — read the caveat first

The GitHub repository was created 2024-11-05 with a single `Initial commit`, then received a bulk
`transfer to github` commit on 2025-06-11. Copyright headers in the source date from **2018 and 2019**. The
public git history is therefore **an import, not the project's real history** — openvocs is roughly a decade
old (the DLR project page dates the idea to 2014) and its first eight years are not in this repo. Cadence
figures below describe the public repository only.

Commits on `main` by month (472 total):

| 2024-11 | 2025-06 | -07 | -08 | -09 | -10 | -11 | -12 | 2026-01 | -02 | -03 | -04 | -05 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | 37 | 30 | 29 | 44 | 27 | 68 | 58 | 106 | 12 | 39 | 7 | 14 |

`main` has been quiet since 2026-05-08, but that is a release-branch artefact: `development` was pushed
2026-08-06 and `release_candidate` 2026-08-10. **This project is alive.** Recent unreleased work includes
dynamic mixer/recorder spawning (2026-06-29/30), LDAP import updates (2026-07-07), and an AES-256-GCM stream
format for encrypting recorded audio at rest (`aes` branch, 2026-07-06, commit message *"Not tested yet"*).

### 3.2 Contributors — effectively one person, entirely one institution

`git shortlog -sne --all`:

| Commits | Author |
|---|---|
| 425 | Töpfer, Markus `<markus.toepfer@dlr.de>` |
| 180 | Anja Bertard `<anja.bertard@dlr.de>` |
| 83 | Michael J. Beer `<michael.beer@dlr.de>` |
| 12 | klaf `<frschiffner@gmail.com>` (Falk Schiffner, DLR) |
| 6+4 | chrisen / chris-dlr `<christopher.endres@dlr.de>` |
| 3+2+2 | duplicate identities of the above |

**Every contributor is DLR staff.** No commit in the public history is authored by anyone outside DLR/GSOC.
The bus factor is one: Töpfer alone is 59% of commits and is the sole author of the core `ov_vocs` business
logic and the ICE proxy.

### 3.3 Releases, issues and PR responsiveness — the weak point

- **Releases: one.** `Release 2.0.1` (2025-06-26) is the only GitHub Release, despite the shipping version
  being 2.6.2. Four tags exist: `release`, `release2.0.1`, `gsoc_release_20250704`, `gsoc_release_20250826` —
  the latter two named for internal GSOC deliveries, and the newest is nearly a year old.
- **`changelog.md` has exactly one entry** (v2.6.2, 2026-05-08). The file's own text says *"This file will
  track changes between Openvocs releases"* — future tense, one entry, eleven months after the first release.
- **Issues: 2 ever.** #1 (Sept 2025) was diagnosed and closed by the *reporter* — a multicast join failure the
  user fixed himself with `ip addr add 224.0.0.1/24 dev eth0 autojoin`, with no maintainer troubleshooting in
  the thread. #3 (Jan 2026, "REQUEST: hotkey for activating loops") got one maintainer reply — *"That is not
  implemented yet, but shouldn't be a bigger issue"* — and has been open ever since.
- **Pull requests: this is the damning number.** Four PRs from **SpaceCargoUnlimited** — a genuine outside
  party, not DLR — opened 2026-07-27, all still **open**, all unreviewed. They are exactly the contributions a
  healthy project would merge in a week: a STUN pair-handling fix in the ICE proxy, an off-by-one in the
  webserver range handler that breaks the speaker test on Chrome, a WebKit slider CSS fix, and support for
  binding a specific IP / domain name / existing certificate. That last one is *deployment-blocking* for
  third-party operators. Three weeks of silence on four small, obviously-correct-shaped PRs from the only
  outside contributor the project has ever had is the clearest available signal about how upstream
  contribution would go.
- **Reach:** 15 stars, 4 forks, 2 watchers, 1 repo in the org.

### 3.4 Supported product or published artefact? — **artefact, with a commercial layer on top**

The evidence is unambiguous and it is the most consequential finding in this note.

- **DLR's own classification.** The [DLR project page](https://www.dlr.de/en/rb/research-operation/research-projects/control-center-technology/openvocs-r)
  files OPENVOCS® under *research projects → control centre technology*. There is no support statement, no
  roadmap, no SLA, no release policy, no third-party-user commitment anywhere on DLR's pages or in the
  repository.
- **It is genuinely operational, though.** DLR reports operational use at GSOC and a mission debut with the
  MORABA sounding-rocket team in Kiruna in 2023, and the work has been published at SpaceOps 2021 and
  SpaceOps 2023. This is not vapourware — it flies. It is simply not *supported* as a product for outsiders.
- **The commercial layer.** [openvocs.com](https://openvocs.com/) — distinct from the DLR-linked openvocs.org —
  markets an open-source core plus **"Commercial Enterprise Extensions"**: the *openvocs Recorder Module*
  ("high-throughput, tamper-proof voice loop recording and archiving … cryptographic signing") and the
  *openvocs SIP Gateway*, **"licensed annually per Operations Center with unlimited users"**, with
  *"enterprise-grade, mission-critical 24/7 SLAs"*. Its [imprint](https://openvocs.com/imprint.html) names
  **Markus Töpfer** as a natural person at a private address — no GmbH, no registered company.

So the project's dominant maintainer, still committing from a `@dlr.de` address in May 2026, personally sells
the two modules that are conspicuously absent from the repository. The handbook confirms the split from the
other side: *"Telephony Panel – is deactivated in the opensource version of OPENVOCS®"*, and the README's
`get_recordings` documentation carries *"**Note**: Depending on the version, this may not available."*

This is an **open-core** project. That is a legitimate business model and no criticism is implied — but it
means the open-source repository is deliberately incomplete in precisely the dimension VoxLoop's map calls a
likely compliance requirement, and the incompleteness will not be fixed upstream, because fixing it upstream
would compete with the maintainer's product.

**Bluntly:** as a foundation, openvocs is a single-institution, single-maintainer, one-release, four-stale-PR
project whose missing pieces are missing on purpose. That is a liability, not a base.

---

## 4. Architecture

### 4.1 Process decomposition

C throughout (~342 kLOC excluding the vendored SQLite amalgamation; 587 `.c` and 317 `.h` files), plus a
vanilla-JavaScript ES-module browser client (~17.5 kLOC across 106 files, no framework, no build step, no
`node_modules`). Built with `make`; no package manager anywhere.

`src/service/makefile` builds exactly these binaries:

| Service | Role |
|---|---|
| `ov_mc_vocs` | Core. Business logic, permissions, sessions, the WebSocket signalling API, **and the TLS webserver that serves the HTML client**. |
| `ov_mc_ice_proxy` | WebRTC/ICE/DTLS-SRTP termination. Bridges browsers to the internal multicast bus. |
| `ov_mc_mixer` | Per-user audio mixer. **One process per connected user.** |
| `ov_mc_vad` | Voice-activity detection, so the UI can show who is talking. |
| `ov_mc_interconnect` | Links two openvocs instances' loops across sites. |
| `ov_alsa_gateway` | Maps a Linux sound device to a loop, bidirectionally. |
| `ov_api_echo_success` | Test stub. |

`ov_mc.target` requires `ov_mc_mixer.target`, `ov_mc_ice_proxy.service`, `ov_mc_vocs.service`,
`ov_mc_vad.service`. **No recorder, no SIP gateway** — see §0 and §6.1.

### 4.2 Audio transport

This is the interesting part, and it is not an SFU.

**A loop is an IP multicast group.** `ov_vocs_db_get_multicast_group(db, loop)` returns the `host:port` for a
loop name; `ov_mc_loop_data` is `{name, socket, volume}`
([`src/lib/ov_vocs/include/ov_mc_loop.h`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs/include/ov_mc_loop.h)).
The default cluster address is `224.0.0.1:60000`.

**Inbound path.** The browser opens one WebRTC peer connection to the ICE proxy, offering a single Opus stream
(`a=rtpmap:100 opus/48000/2`, `useinbandfec=1`, stereo
— [`src/lib/ov_ice_proxy/src/ov_ice_proxy_vocs.c:39-48`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_ice_proxy/src/ov_ice_proxy_vocs.c)).
The proxy terminates DTLS-SRTP, then forwards the **decrypted RTP payload verbatim** — no decode — to every
multicast group in the session's `talk` dict:

```c
static bool send_to_loop(const void *key, void *val, void *data) {
    ...
    ssize_t bytes = sendto(session->socket, container->ptr, container->len, 0,
                           (struct sockaddr *)&sa, sock_len);
```
— [`ov_ice_proxy_vocs.c:229-262`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_ice_proxy/src/ov_ice_proxy_vocs.c),
driven by `ov_dict_for_each(session->talk, &container, send_to_loop)` at line 309.

**Multi-destination emission is therefore free at the transport layer** — the same buffer is `sendto`'d to N
groups. This matters for VoxLoop and is picked up in §6.6.

**Outbound path.** Each user's mixer process joins the multicast groups for the loops that user is subscribed
to (`ov_mc_mixer_core_join`), decodes each incoming Opus RTP stream to PCM16, scales it by the **per-loop**
volume, sums, re-encodes to Opus, and sends one stream to a unicast socket owned by the ICE proxy, which
relays it into the user's peer connection
([`src/lib/ov_vocs/src/ov_mc_mixer_core.c`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs/src/ov_mc_mixer_core.c);
`ov_mc_mixer_core_forward` = `{socket, ssrc, payload_type}`).

So the shape is: **SFU-like on the internal bus, MCU at the edge.** Per-talker streams exist, with distinct
SSRCs, on the internal multicast bus. They are collapsed into one mix only in the last hop to each client.

**Codec:** Opus 48 kHz throughout, with G.711, PCM16-signed and raw codecs also implemented in `ov_codec` for
gateway use.

**Does the server have decoded audio?** Yes — twice over, and this is the strongest thing in the design:

1. Every mixer process holds fully decoded PCM16 for every loop it is joined to.
2. More usefully, **the internal bus carries plaintext, unencrypted, per-talker Opus RTP on multicast
   groups**. Any process that joins the group gets the audio. That is exactly how the recorder is designed to
   work: the start event carries `{loop, mc_ip, mc_port, roll_after_secs, vad, silence_cutoff_interval_msecs}`
   ([`src/lib/ov_core/include/ov_recorder_events.h:40-51`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_core/include/ov_recorder_events.h))
   — the controller does not stream audio to the recorder, it *tells the recorder which multicast group to
   join*. The ALSA gateway works identically: its config maps `{"device":"0,0", "loop":"224.0.0.4", ...}`
   ([`resources/config/ov_alsa_gateway_config.json`](https://github.com/openvocs/openvocs/blob/main/resources/config/ov_alsa_gateway_config.json)).

This is a better answer to VoxLoop's R4 (decoded-audio tap) than anything in #3, including LiveKit's PCM-over-
WebSocket egress. Sinks are additive and out-of-band: adding a recorder, a transcriber or a second site costs
zero changes to the media path and imposes zero load on the SFU-equivalent.

**The security counterpart, stated plainly:** that bus has no authentication and no encryption. Anyone who can
send UDP to the multicast group can inject audio into a loop; anyone who can join it can listen to every loop.
On a single box this is contained to the host. On the multi-machine deployment the README explicitly invites
(*"A mixer may be started on different hardware or cloud based systems"*), the internal network becomes the
security boundary. No `NOTICE`, README or handbook mentions this.

### 4.3 Data model and persistence

Five entity types, hierarchical:

```c
typedef enum ov_vocs_db_entity {
    OV_VOCS_DB_ENTITY_ERROR = -1,
    OV_VOCS_DB_DOMAIN  = 0,
    OV_VOCS_DB_PROJECT = 1,
    OV_VOCS_DB_LOOP    = 2,
    OV_VOCS_DB_ROLE    = 3,
    OV_VOCS_DB_USER    = 4
} ov_vocs_db_entity;
```
— [`src/lib/ov_vocs_db/include/ov_vocs_db.h:145-154`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs_db/include/ov_vocs_db.h)

Permission is a three-valued enum, and it is defined **per (role, loop)** — never per user:

```c
typedef enum ov_vocs_permission {
    OV_VOCS_NONE = 0,
    OV_VOCS_RECV = 1,
    OV_VOCS_SEND = 2
} ov_vocs_permission;
```
— [`src/lib/ov_vocs_db/include/ov_vocs_permission.h:38-44`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs_db/include/ov_vocs_permission.h)

`ov_vocs_db_get_permission(db, role, loop)` takes a **role**, not a user
([`ov_vocs_db.h:447`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs_db/include/ov_vocs_db.h)).
A user's authority is entirely derived from the role they authorise as. `SEND` implies `RECV`
(`ov_vocs_permission_granted(reference, check)`). Volume, by contrast, *is* per user:
`ov_vocs_db_get_volume(db, user, role, loop)` — a per-user preference, not a permission.

**This is #4's recommendation, implemented.** VoxLoop was about to derive user / role / loop from prior art;
openvocs is proof the derivation lands somewhere that works in flight.

**Persistence is a JSON tree, not a database.** The whole config lives in memory as `ov_json_value` and is
snapshotted to disk on a timer — `state_snapshot_seconds: 60`, `auth_snapshot_seconds: 300`
([`ov_vocs_db_persistance.h`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs_db/include/ov_vocs_db_persistance.h),
`src/service/ov_mc_vocs/config/default_config.json`). The default config sets `"git": true`, versioning the
config directory in a git repo. SQLite is vendored but used only by `ov_database`, for the recorder's
recording index — not for the core model. Passwords are hashed (`ov_password_hash_parameter`), and optional
LDAP import exists (`ov_vocs_db_persistance_ldap_import`, disabled by default).

`domain` and `project` are a two-level tenancy scoping. VoxLoop's map says *"no tenancy concept anywhere in
the data model"* — adopting openvocs means adopting two levels of it, or carrying dead scoping.

### 4.4 How a loop state change actually works

The state machine in `perform_switch_loop_request`
([`src/lib/ov_vocs/src/ov_vocs.c:830-920`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs/src/ov_vocs.c)):

| From → To | Action |
|---|---|
| `none` → `recv`/`send` | `ov_mc_backend_join_loop` — mixer joins the loop's multicast group |
| `recv` → `none` | `ov_mc_backend_leave_loop` |
| `recv` → `send` | `ov_mc_frontend_talk(..., true, data)` — **the ICE proxy starts forwarding this user's inbound audio to the loop's group** |
| `send` → `recv`/`none` | `ov_mc_frontend_talk(..., false, data)` |

**Emission is enforced server-side, at the media plane.** A client cannot reach a loop it has not been armed
for, because the proxy simply does not have that group in its `talk` dict. This is a *stronger* property than
LiveKit's publisher-driven subscription permission, which #3 flagged as its one soft spot on R3.

The permission check happens at request time
([`src/lib/ov_vocs/src/ov_vocs_api.inc:1804-1816`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs/src/ov_vocs_api.inc)):

```c
ov_vocs_permission requested  = ov_vocs_permission_from_string(state);
ov_vocs_permission permission = ov_vocs_db_get_permission(vocs->config.db, role, loop);
if (!ov_vocs_permission_granted(permission, requested)) { /* error 'auth permission' */ }
```

### 4.5 Push-to-talk is not what the README says it is

`switch_ptt` **does not exist** — the string appears nowhere in the source. The real event is `talking`
(`OV_EVENT_API_TALKING`), and reading its handler
([`ov_vocs_api.inc:2004-2098`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs/src/ov_vocs_api.inc))
shows it does nothing to the audio path: it re-checks `permission == OV_VOCS_SEND`, notifies the recorder
(`ov_vocs_recorder_ptt`), sends a success response, and broadcasts `talking` to the loop. That is all.

**PTT is enforced entirely client-side**, by muting the local track:
`this.#stream.getAudioTracks()[0].enabled = !value`
([`src/HTML/lib/ov_media/ov_audio.js:63`](https://github.com/openvocs/openvocs/blob/main/src/HTML/lib/ov_media/ov_audio.js)),
called from `activate_mic_soft` / `release_mic_soft` in `ptt_bar.js`. While a loop is in `send`, the ICE proxy
forwards continuously; whether anything audible arrives depends on the browser having muted the track.

The two-stage model from #4 (*arm, then key*) is therefore real in openvocs — but only the **arm** stage is
server-enforced. The **key** stage is advisory. For VoxLoop, where "state shown must always be factual" is a
map constraint, this is a design decision to make consciously, not inherit.

The `SECURE_VOICE_PTT` config flag ([`src/HTML/config_vocs.js:52`](https://github.com/openvocs/openvocs/blob/main/src/HTML/config_vocs.js))
switches this off entirely, for sites where a hardware PTT button physically gates the microphone — the
honest answer to the same problem.

### 4.6 Signalling protocol shape — and its documentation drift

WebSocket carrying JSON, one flat envelope:

```json
{"event": "name", "uuid": "...", "type": "unicast", "version": 1,
 "parameter": {...}, "response": {...}, "error": {"code": n, "description": "..."}}
```
— [`src/lib/ov_core/include/ov_event_api.h`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_core/include/ov_event_api.h)

Dispatch is a flat `ov_dict` from event name to handler
(`vocs_event_callback`, [`ov_vocs.c:3176`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs/src/ov_vocs.c));
there is no auth middleware — each handler authenticates itself (see §5.4).

**The published client API in the README is not accurate.** Documented-but-nonexistent: `switch_state` (real
name `switch_loop_state`), `switch_volume` (real: `switch_loop_volume`), `switch_ptt` (real: `talking`). The
`SIP Status` section documents an event named `role_loops` by copy-paste error. Undocumented-but-real:
`get_clients`, `get_all_loops`, `get_recorded_loops`, `state_mixer`, `state_connections`, `state_session`,
`get_mixer_state`, `broadcast`, `start_record`, `stop_record`, `ping`/`pong`, `authenticate`, `user_loops`,
`is_ldap_enabled`, and ~20 database-administration events. The `version` field exists in the envelope but
`ov_event_api_message_create(name, uuid, 0)` is called with version `0` at every site I read, so it is not
actually used for versioning.

**Consequence:** the protocol is *stable in practice* (it has a real client and real deployments) but
**unspecified**. Writing a third-party client means reading `ov_vocs_api.inc` and `src/HTML/lib/`, and having
no contract if either changes.

### 4.7 How this compares to what #6 is about to decide

| #3's requirement | openvocs | Verdict |
|---|---|---|
| **R1** self-host, no external runtime dep, air-gappable | Own STUN server on `0.0.0.0:33533`, no TURN needed in multiplexing mode, no phone-home, OS-package deps only | **Pass, strongly** — see §5 |
| **R2** one stream per publisher, client-side mixing at per-source volume | **Fail.** Server mixes per user; volume is per **loop**, applied in the mixer | Deliberate opposite |
| **R3** server-enforced selective subscription | **Pass, better than LiveKit.** Enforced in the media plane at the ICE proxy (§4.4) | |
| **R4** decoded-audio tap | **Pass, best of any candidate** — multicast bus, additive sinks (§4.2) | …but the sink itself is the paid module |
| **R5** ~200 connected | **Fail as shipped.** 1 mixer process per user, 50 pre-started; unpublished scaling above that | |
| **R6** TURN / VPN | Dynamic mode supports external STUN/TURN and a configurable port range for firewalls | Pass |
| **R7** SvelteKit + Tauri clients | No SDK of any kind, in any language. Raw WebSocket + WebRTC only | Fail |
| **R8** operational weight | 4 systemd units minimum + N mixer processes | Heavy |

**The single most useful thing openvocs does to #6 is challenge R2.** #3 treats "one stream per publisher" as
load-bearing because it enables per-entity volume sliders. But #4's own survey found that what operators
actually use — NASA MCC, NASA HOSC, openvocs, Motorola, Clear-Com, ARIA — is **per-loop** volume, and *that*
is satisfiable by a server-side per-user mixer. openvocs is a decade-old flying counter-example to R2 as
stated.

That does **not** make R2 wrong; it makes its justification wrong. The real arguments for separate streams
survive: client-side ducking and interruptive priority (#4 §4.5), per-talker identification without a
signalling side-channel, and not foreclosing per-stream AI processing (map constraint). And openvocs pays for
its choice at a rate of one OS process per user.

But note that openvocs *keeps* per-talker separation where it matters for the future — on the internal bus.
Recording, transcription and AI processing all consume per-talker streams there. Only the client is denied
them. **#6 should therefore separate two questions it may currently be conflating: "does the client receive
separate streams?" and "does the architecture preserve separate streams?" openvocs answers no and yes.**

---

## 5. Deployment

### 5.1 Can it self-host cleanly, air-gapped? — yes, and this is its second-best quality

**Build:** `source env.sh && make`. Dependencies are all Debian 12 packages — `libssl-dev`, `libopus-dev`,
`libsystemd-dev`, `libpulse-dev`, `libsrtp2-dev`, `libldap2-dev`, `libsasl2-dev`, `libasound2-dev`, `gcc`,
`make`, `pkg-config`
([`resources/debian/debian-packages.yml`](https://github.com/openvocs/openvocs/blob/main/resources/debian/debian-packages.yml)).
**There is no npm, no cargo, no go modules, no pip, no CDN.** The browser client is hand-written ES modules
with every third-party asset vendored into `src/HTML/plugin/`. Nothing is fetched at build time beyond
distro packages, and nothing at all at runtime. For an air-gapped site this is close to ideal, and it is a
sharper story than any candidate in #3.

**Install:** `make deb` produces `openvocs_<version>-<build>_amd64.deb`; `dpkg -i` installs config to
`/etc/openvocs` and the client to `/srv/openvocs/HTML`. `sudo ./ov_config.sh <ip>` rewrites the four service
configs for the site's IP **and generates a certificate**. Then `systemctl start ov_mc.target`.

**Runtime externals: none.** The ICE proxy in multiplexing mode needs no STUN or TURN; `ov_mc_vocs` runs its
own STUN listener on `0.0.0.0:33533`. LDAP is `"enabled": false` by default. No licence check, no telemetry, no
update channel — I looked for outbound calls and found none.

**Container support: none.** No `Dockerfile`, no `docker-compose.yml`, no Helm chart, no OCI publishing
anywhere in the tree. Packaging is `.deb` (Debian 12 supported; Ubuntu and Raspbian package lists present) and
an RPM spec for openSUSE Leap. Deployment is systemd-on-a-VM, full stop. That happens to match VoxLoop's
"single box for v1", so it is not a blocker — it is a signal about the intended operating model.

### 5.2 TLS and browser requirements

TLS is mandatory: the webserver binds `0.0.0.0:443`, and browsers will not grant `getUserMedia` on an insecure
origin anyway. `ov_config.sh` generates a self-signed certificate, and the service README's own walkthrough
ends with *"The new certificate needs to be accepted!"* — i.e. the default path leaves every operator clicking
through a browser security interstitial. Using a real internal-CA certificate is possible but is not
documented; note that "support an existing certificate and a domain name" is precisely what the unmerged
outside PR #8 adds. **The out-of-the-box TLS story is not deployment-ready for a customer site.**

Browser: *"Firefox is tested and recommended, other browser also work, may lead to unexpected behavior"*
(README §1, repeated in the handbook). The two open outside PRs fixing a Chrome speaker test and a WebKit
slider confirm that non-Firefox is under-tested. WebHID (§7) is Chromium-only, so the recommended browser and
the best PTT input path are mutually exclusive — an unremarked contradiction in the project.

### 5.3 Configuration surface and documentation quality

Config is JSON per service under `/etc/openvocs/<service>/config.json`, plus a client-side
`config_vocs.js` served to the browser
([`src/HTML/config_vocs.js`](https://github.com/openvocs/openvocs/blob/main/src/HTML/config_vocs.js)) carrying
`SIP`, `RECORDER`, `SECURE_VOICE_PTT`, `PTT`, `MUTE_KEY_DEF`, `MULTI_TALK`, `SCREEN_KEYBOARD`,
`SIGNALING_SERVERS` (with backup-server support) and more. It is coherent and legible.

Documentation for a non-DLR operator, honestly assessed:

- **README** — good install walkthrough; a *stale and partly wrong* API section (§4.6); an unfilled citation
  TODO; marked `Status: ***DRAFT***`, last updated 2025-07-01.
- **User Handbook** — genuinely good, screenshot-driven, covers both the operator and admin clients. Also
  marked `***DRAFT***`, dated 2025-06-30, at "v1_2" while the software is 2.6.2.
- **Service READMEs** — three of six have real content; `ov_mc_vad` has an empty README; `ov_mc_vocs`'s is one
  sentence. `src/service/readme.md` documents four services (`ov_mixer`, `ov_resource_manager`,
  `ov_webgateway`, `ov_webserver`) **none of which exist any more**, with a walkthrough referencing a binary
  (`ov_webserver_auth_example`) that is not built.
- **No architecture document, no protocol specification, no operations runbook, no upgrade guide, no backup
  guidance, no sizing guidance beyond "4 users that speak in parallel".**

A competent third-party operator could get openvocs running from this. They could not run it in production
without reading C.

### 5.4 A security finding worth recording

Handler dispatch has no authorisation middleware; each handler checks its own. The **database administration**
API does this properly — `ov_vocs_db_authorize_domain_admin` / `..._project_admin` appear at ~20 sites in
`ov_vocs_api.inc`. But the **runtime-state administration** API registered by `enable_api_admin`
([`ov_vocs_api.inc:612-641`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs/src/ov_vocs_api.inc))
does not check anything. `admin_get_clients` reads straight from `vocs->connections` and replies; it does not
look at the socket's user at all:

```c
static bool admin_get_clients(ov_vocs *vocs, int socket, ov_json_value *input) {
    val = ov_json_array();
    if (!ov_socket_json_for_each(vocs->connections, val, add_connection_data)) goto error;
    ...
```

The same is true of `admin_broadcast` (push arbitrary JSON to every registered client), `start_record` /
`stop_record`, `get_all_loops`, `get_recorded_loops`, `state_mixer`, `state_connections` and `state_session`.
On the code at `83ad2da`, **any party that can open a WebSocket to the server appears able to enumerate
connected clients, list all loops, broadcast to every client and start or stop recording, without
authenticating.** There is no `SECURITY.md` and no disclosure policy; I have not reported this. Treat this as
"read the code before trusting it", not as a claim of exploitability in a specific deployment — I have not
run the server.

---

## 6. The gaps

Assessed against the list in #15. **Has** / **Partial** / **Lacks**, each with the evidence.

| Capability | Verdict | Evidence |
|---|---|---|
| Admin-controlled speak/listen vectors | **Has** | `ov_vocs_permission {none,recv,send}` per (role, loop); admin GUI links user↔role and role↔loop graphically (Handbook §3.2) |
| Mid-event revocation | **Lacks** | §6.2 |
| User-initiated request-access | **Lacks** | §6.3 |
| Generic event injection | **Partial** | §6.4 |
| Text-to-speech | **Lacks** | §6.4 |
| Unfocused desktop PTT | **Lacks** (but see §7) | §6.5 |
| "Who is listening to me" | **Partial** | §6.7 |
| Vacant-role / occupancy detection | **Lacks** | §6.8 |
| Multi-destination emission | **Has**, unenforced | §6.6 |
| Recording | **Not in the open-source build** | §6.1 |
| Latched emission | **Lacks** | §6.9 |

### 6.1 Recording — the disqualifying one

`src/service/makefile` builds no recorder. What the repository *does* contain: `ov_vocs_recorder.c` (the
controller that sends start/stop to an external recorder over TCP on port 10010), `ov_recorder_events.c` (the
wire schema), `ov_database_events` (the recordings index), the admin GUI under `src/HTML/extensions/recorder/`,
a config file at `resources/config/ov_recorder_config.json` pointing at
`repository_root_path: /tmp/ov_recorder_repo`, and a Python test harness
(`test/05-recorder/ss_recorder.py`) that imports a `rich.python3` package which is also not in the repository.

The daemon that writes audio to disk is absent, and openvocs.com sells it. `resources/debian/conffiles` lists
config directories for alsa, ice_proxy, mixer, vocs and vad — no recorder.

**What that costs a VoxLoop that forked openvocs:** writing a multicast-joining, Opus-decoding, rolling-file,
VAD-gated recorder daemon in C against an undocumented event schema, and maintaining it against an upstream
whose maintainer sells the competing article. That is not a small module — the commercial version advertises
synchronous multi-channel capture, cryptographic signing and lifecycle management, and the unmerged `aes`
branch shows the encryption half is being built *now*, unreleased and "not tested yet".

**What it costs a VoxLoop that only borrows the model:** nothing. The multicast-bus idea is free.

### 6.2 Mid-event revocation — lacks

The map requires *"An admin can demote a talker mid-event."* In openvocs, `ov_vocs_db_get_permission` is called
at exactly two sites: the `switch_loop_state` handler and the `talking` handler
(§4.4, §4.5) — both **request-time** checks. When the database changes, the only listener is
`process_trigger` ([`ov_vocs.c:2920-2943`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs/src/ov_vocs.c)),
and on `update_db` it does exactly one thing:

```c
if (0 == ov_string_compare(event, "update_db")) {
    ov_json_value *proc = ...;
    ov_json_object_for_each(proc, vocs, update_sip_backend);
}
```

It updates **SIP call permissions only**. No live session is re-evaluated, no `talk` forward is torn down, no
mixer is told to leave a loop. A user already armed to `send` on a loop keeps sending until they themselves
change state or disconnect. (The `ldap_update` branch immediately below it is an empty block.)

Revocation-of-a-live-session is architecturally *easy* here — the server owns the forward — but it is not
implemented, and implementing it upstream means touching the core session lifecycle.

### 6.3 Request-access — lacks

I grepped the C, the `.inc` files and the JavaScript for `request_access`, `join_request`, `ask_for`, and every
near-variant. There is nothing. Permission changes flow one way: an admin edits the database in the admin
client. There is no channel by which a user asks for `send` on a loop.

### 6.4 Event injection and TTS

- **Text/system-event injection: partial.** `admin_broadcast` takes an arbitrary JSON object, stamps it
  `type: "system broadcast"` and pushes it to every client that has sent `register`
  ([`ov_vocs_api.inc`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs/src/ov_vocs_api.inc)).
  That is a usable on-screen notification channel. It is unauthenticated (§5.4), undocumented in the README,
  and has no schema.
- **Audio injection: has, structurally, in a form VoxLoop should note.** Because a loop is a multicast group,
  injecting audio into a loop means sending RTP to a UDP address — no API, no permission, no session. The
  `ov_alsa_gateway` is exactly this: a config file mapping a sound device to `224.0.0.4`. A TTS engine writing
  to an ALSA device would be heard on that loop. This is the cheapest audio-injection seam of any architecture
  in either research note — and the same property is the security hole in §4.2.
- **TTS itself: lacks.** No speech synthesis anywhere in the tree. (DLR did publish separate research pairing
  openvocs with Mozilla DeepSpeech for *recognition* in 2020; nothing from that is in this repository.)

### 6.5 Unfocused desktop PTT — lacks

openvocs is browser-only. There is no desktop application, no Electron shell, no Tauri wrapper. PTT input
paths in `ptt_bar.js` are: on-screen button (`mousedown`/`mouseup`/`touchstart`/`touchend`), a full-screen PTT
area, middle mouse button, a configurable key (default space bar, `MUTE_KEY_DEF = " "`), the **Gamepad API**,
and **WebHID**. The `document.addEventListener("keydown"/"keyup")` and mouse paths require document focus by
definition; the Gamepad API requires the document to be focused for `getGamepads()` to return live state.

Issue #3 upstream — *"REQUEST: hotkey for activating loops"* — is the same problem, answered with *"the
spacebar may be used as PTT"* and left open since January.

**openvocs does not solve #5, and never claimed to.** But it does contain an input path #5 did not consider —
see §7.

### 6.6 Multi-destination emission — has, but only the server does

`send_to_loop` fans one buffer out to every group in `session->talk` (§4.2), and the server-side state machine
places no cap on how many loops are in `send`. The restriction that #4 recorded — *"only one for talking mode
… the former voice loop will switch back to monitoring automatically"* — is a **client-side default**:

```js
MULTI_TALK = false; //allow to talk in several loops at the same time
```
— [`src/HTML/config_vocs.js:62`](https://github.com/openvocs/openvocs/blob/main/src/HTML/config_vocs.js),
enforced at exactly one place:
`if (!MULTI_TALK && current_talk_loop && current_talk_loop.loop_id !== loop.loop_id)`
— [`loop_view.js:267`](https://github.com/openvocs/openvocs/blob/main/src/HTML/app/vocs/views/loops/js/ui/loop_view.js).

**This corrects #4.** The map records openvocs as enforcing one-talk-loop-at-a-time alongside NASA; in truth
openvocs *supports* multi-destination emission and ships it disabled, with the choice exposed as a
deployment-time flag. That is one more system in the "permits multi-destination" column, and evidence that a
per-site policy switch is a defensible way to ship it. It also means openvocs offers VoxLoop **no compensating
safety mechanism** to borrow — there are no talk-count caps, no solo/exclusive action, no warnings. The flag
is the whole feature.

### 6.7 "Who is listening to me" — partial, and thinner than it looks

Each loop maintains a participant dict keyed by socket
([`src/lib/ov_vocs/src/ov_vocs_loop.c`](https://github.com/openvocs/openvocs/blob/main/src/lib/ov_vocs/src/ov_vocs_loop.c)),
added on mixer-join (`ov_vocs.c:1026`) and dropped on leave. `ov_vocs_loop_get_participants` serialises it and
it rides on every `switch_loop_state` broadcast to that loop's subscribers, so a client does learn **who is in
each loop it is in** — with `{client, user, role}`.

Two limits, both from the code:

1. **The participant record carries no state.** `ov_vocs_loop.c:152-165` writes `client`, `user` and `role`
   only. There is no `recv`/`send` per participant in the list. A client can *infer* state by accumulating the
   individual `switch_loop_state` broadcasts (which do carry `state`), but the snapshot it joins with does
   not tell it who is listening versus who is armed.
2. **Visibility is per-loop, not per-person.** You see participants of loops *you* are in. There is no view of
   "everyone currently receiving my voice across all loops I am emitting to" — which is #4's finding that no
   surveyed system provides it, and it holds here.

The handbook's *"Icon with number => shows the number of active users in the loop"* is this dict's count. It
is a real primitive and VoxLoop should copy it. It is not reverse visibility.

### 6.8 Vacant-role / occupancy — lacks

No `occupancy`, `vacant`, `unmanned` or presence concept exists (the one `OV_KEY_PRESENCE` constant in
`ov_config_keys.h` is unused by the vocs services). Nothing prevents two users authorising as the same role
simultaneously, and nothing surfaces that a role has nobody signed into it. This confirms #4's conclusion:
**vacant-role detection is unsolved everywhere, including here, and remains VoxLoop's to invent.**

### 6.9 Latched emission — lacks, and this is a per-site either/or

`config_vocs.js` has `PTT = true; // if ptt is false an mute/unmute button is displayed instead`. The two
modes are mutually exclusive at the deployment level, not per-key and not per-user. VoxLoop's settled
"momentary **plus** latched" model has no analogue.

---

## 7. The desktop problem

**Does openvocs' architecture admit a desktop client?** Architecturally yes, practically expensive.

**In favour.** The audio path is cleanly separable from the UI. `src/HTML/lib/` (`ov_vocs.js`, `ov_auth.js`,
`ov_media/ov_webrtc.js`, `ov_websockets`) is a UI-free protocol library; `src/HTML/app/vocs/views/` is the UI on
top. Signalling is a plain WebSocket carrying flat JSON — no XMPP, no protobuf, no SDK. The server has no
knowledge of the browser: the ICE proxy offers a bog-standard single-Opus SDP and speaks ordinary ICE and
DTLS-SRTP. Any WebRTC stack that can do that can be a client. The `ov_mc_interconnect` service is itself proof
that a non-browser peer can attach to loops.

**Against, in order of severity.**

1. **No SDK, in any language, and no protocol specification.** You reverse-engineer `ov_vocs_api.inc` and
   `src/HTML/lib/`, against a README whose API section is demonstrably wrong for three of the events an
   operator client most needs (§4.6). Compare LiveKit, which ships a Rust crate (#3 §3.8). The `version` field
   in the envelope is always `0`, so there is not even a compatibility signal to check against.
2. **A native client still needs a full WebRTC stack.** ICE + DTLS-SRTP + Opus. `webrtc-rs` or `libwebrtc` in
   a Tauri sidecar carries the audio outside the webview — which is exactly the workaround #3 §9 and #5 already
   identified for Linux. openvocs neither helps nor hinders here; the cost is the same either way, minus the
   SDK LiveKit would have given you.
3. **Unfocused PTT is not addressed by anything upstream.** A Tauri shell around the openvocs client hits
   `global-hotkey`'s missing Wayland backend and WebKitGTK's compiled-out WebRTC in exactly the way #5
   documented. Wrapping openvocs does not move that wall one inch.

**The finding that does move something.** `ptt_bar.js:214-252` opens HID devices and keys PTT from raw input
reports:

```js
if (!!navigator.hid) {
    ...
    device.addEventListener("inputreport", (event) => {
        const value = event.data.getUint8(0);
        if (value === 32) button_press_down(); else button_release();
    });
```

#5 concluded that *"headset PTT buttons are uncatchable as keystrokes"*. That is true and beside the point:
they are catchable as **HID input reports**, and openvocs catches them, in a browser, in shipped code. This
matters to VoxLoop for two reasons: it is a genuine PTT path for the *browser* tier (which #1 currently limits
to on-screen PTT and latch), and if it survives testing while the page is unfocused, it is a path to
unfocused PTT that does not require a desktop application at all. Constraints to verify before relying on it:
WebHID is **Chromium-only** (no Firefox, no Safari — and openvocs recommends Firefox), requires a secure
context, and requires a one-time user-gesture permission grant per device. **Whether `inputreport` continues
to fire while the page lacks focus or is backgrounded is not established here and should be spiked**, because
it is the whole question. This is the single most actionable thing in this note and belongs on #9 / #5.

---

## 8. The four options, honestly assessed

### Option A — Fork openvocs

**Cost.** You inherit ~342 kLOC of C — an entire in-house standard library (`ov_base`, `ov_core`, `ov_value`,
`ov_json`, `ov_format`, its own event loop, its own WebSocket and HTTP server, its own ICE/STUN/SRTP stack,
its own dictionary and JSON types). Almost none of it is off-the-shelf. Then you write the missing recorder,
add mid-event revocation, request-access, latched emission, occupancy and reverse visibility, and rebuild the
client, because the map wants SvelteKit and openvocs' client is framework-free vanilla JS with a per-role
tile-grid layout engine.

**What it forecloses.** Svelte on the front end is only preserved by throwing the whole client away. The Tauri
desktop tier still has to be built from nothing. The server language is C, decided for you, forever — for a
solo developer working with coding agents, in a domain (network-facing audio, hand-rolled crypto transport)
where memory-safety bugs are security bugs. It forecloses the per-source-stream architecture permanently, and
with it client-side ducking and interruptive priority.

**Maintenance burden.** Upstream releases once a year; `main` lags `development` by three months; there is one
changelog entry. Rebasing a fork against that is guesswork. And the divergence is not cosmetic — the features
VoxLoop needs sit in the session lifecycle and the permission model, i.e. the same files Töpfer commits to
most.

**If it turns out wrong.** Catastrophic. You would be months into C you did not write, with a product
architecture you cannot change, and a rewrite is the only exit.

### Option B — Contribute upstream

**Cost.** Four PRs from the project's only outside contributor have sat unreviewed since 2026-07-27, one of
them fixing a deployment blocker. There is no CONTRIBUTING file, no governance document, no review SLA, no
roadmap and no public design discussion. You would be proposing changes to session lifecycle and permission
semantics — the maintainer's core — with no forum in which to agree them first.

**What it forecloses.** Your roadmap becomes someone else's queue. Every VoxLoop differentiator becomes a
public feature your competitors get free, including the maintainer's own commercial product.

**The structural problem.** The features VoxLoop most needs upstream (recording, and by extension the
compliance story) are the features the lead maintainer sells. Contributing an open-source recorder to openvocs
is asking a vendor to accept a free replacement for their paid module. That may be received graciously; it is
not a plan.

**If it turns out wrong.** Cheap to abandon — you have lost time, not architecture. This is the least risky
option and also the least useful.

### Option C — Depend on it as-is

**Cost.** You ship openvocs and build VoxLoop's differentiators around it. This dies immediately on facts, not
judgement: no recording in the open-source build (§6.1), no mid-event revocation (§6.2), no request-access
(§6.3), no latch (§6.9), 50-user default ceiling with one process per user (§4.1), an unauthenticated
runtime-admin API (§5.4), a self-signed-certificate install path (§5.2), and no desktop client (§7). The
product would be openvocs with a different logo, minus the recorder that makes it saleable.

**What it forecloses.** Everything. There is no VoxLoop product left — you are a systems integrator for
someone else's research project, and the pilot customer's compliance requirement is a purchase order to a
sole trader in Thuringia.

**If it turns out wrong.** You find out at the pilot, which is the worst possible time.

### Option D — Borrow the model, not the code

**Cost.** Build VoxLoop. openvocs contributes design, not source: the user/role/loop model with per-(role,
loop) `none|recv|send`; per-role loop set **and tile layout** loaded at sign-in; the two-stage arm-then-key
emission model with the arm stage enforced server-side in the media plane; per-loop volume in the recipient's
mix; per-loop participant counts; a fan-out bus with additive sinks; WebHID for headset PTT; and — a real
lesson — the deployment-time `MULTI_TALK` flag as a way to ship multi-destination emission without betting the
product on it.

**What it forecloses.** Nothing. Every one of #6's transport candidates stays open, and openvocs' bus design
is now on the table as a pattern to demand from whichever one wins.

**Maintenance burden.** Yours entirely, which is what a product company signs up for.

**If it turns out wrong.** The failure mode is "we rebuilt something openvocs already did". Given §6, the
overlap is the *undifferentiated* half of the product, and the differentiated half — permissions with
revocation and request-access, occupancy, reverse visibility, event injection, unfocused desktop PTT — would
have had to be built anyway.

---

## 9. Recommendation

**Option D — borrow the model, not the code.** The ticket predicted this outcome; the evidence supports it, and
it is worth being explicit about *which* evidence, because two of the four reasons were not visible before
reading the source.

1. **Recording is the paid module.** Not missing, not unfinished — withheld. `src/service/makefile` builds no
   recorder; openvocs.com sells one, annually, per operations centre; the imprint names the repository's
   dominant maintainer. VoxLoop's map calls the decoded-audio seam a likely compliance requirement. A
   foundation that puts your compliance feature behind a commercial gate held by a single individual is not a
   foundation.
2. **The scaling model does not reach VoxLoop's envelope as shipped.** One mixer process per connected user
   (`ov_vocs.c:1528`), 50 pre-started (`ov_mc_mixer.target`), against a target of ~200 connected. Dynamic
   spawning exists only on an unreleased branch. There is no published benchmark at any user count.
3. **The differentiators are all absent, and each was checked by absence in the code, not assumed** (§6):
   mid-event revocation, request-access, occupancy, reverse visibility, TTS, latch, unfocused PTT.
4. **The stack forecloses the map's stated preferences.** C server, framework-free vanilla-JS client,
   two-level tenancy in a data model the map says must have none, systemd-VM-only packaging, no SDK for any
   client, and a client API document that is wrong about three of the events that matter most.

**And the counter-evidence, weighed and rejected as insufficient.** openvocs' air-gap story is better than any
candidate in #3 (§5.1). Its server-side enforcement of emission is better than LiveKit's (§4.4). Its multicast
bus is the best decoded-audio seam surveyed anywhere (§4.2). Its data model is the one VoxLoop wants (§4.3). If
the recorder were in the box and the mixer were not per-user, this would be a much closer call, and the honest
statement is that **the architecture is not what disqualifies openvocs — the project shape does.**

**What to do concretely:**

- **Copy the model into `CONTEXT.md`** as corroboration for #2: user / role / loop, permission as a
  three-valued enum on (role, loop), volume as a per-user preference on (user, role, loop). openvocs is a
  flying implementation of the vocabulary #4 recommended.
- **Take the bus idea into #6 as a requirement, not an option.** Ask each transport candidate: *can a new
  audio sink be added without touching the media path or loading the SFU?* openvocs answers yes by making a
  loop an addressable multicast group. Nothing in #3 was evaluated against that question.
- **Spike WebHID before #9 closes.** Specifically: does `inputreport` fire while the page is unfocused or
  backgrounded, in Chromium, on Windows and on Linux? A yes materially reshapes the client tiers.
- **Do not vendor any openvocs code.** Not the ICE stack, not `ov_base`. The licence permits it; the
  maintenance does not.
- **Keep watching it.** It is 15 stars and one maintainer today, but it flies real missions, DLR published it
  deliberately, and an outside company (SpaceCargoUnlimited) is already fixing bugs in it. If a foundation or
  a second institution ever picks it up, this conclusion is worth revisiting. Re-check when `main` next moves
  or when those four PRs are resolved.

---

## 10. What this unblocks, and what it changes on the map

#15 blocks **#2**, **#6** and **#9**. All three are unblocked; two of them change.

### #2 — loop / user / role model: **confirmed, with two corrections and one addition**

- **Confirmed.** user / role / loop with `none|recv|send` per (role, loop) is not a hypothesis — it is running
  in a system that flies. Permission attaches to the **role**, not the user; volume attaches to the
  **(user, role, loop)** triple. That split is worth adopting verbatim.
- **Correction to #4.** The map records openvocs as enforcing one-talk-loop-at-a-time. It does not — that is a
  client-side default (`MULTI_TALK = false`) over a server that permits any number. openvocs belongs in the
  "permits multi-destination emission" column, which weakens the map's framing of multi-destination as *"a
  conscious divergence from mission control"* — it is a divergence from mission-control *procedure*, not from
  every system built for it. NASA remains the only true one-talk-loop system surveyed.
- **Correction on reverse visibility.** #4 credits openvocs with per-loop participant counts. True, and worth
  copying — but the participant record carries `{client, user, role}` with **no state**, so it is *presence in
  a loop*, not *who can hear me*. #4's conclusion that no surveyed system provides reverse visibility stands
  intact.
- **Addition.** openvocs has **no occupancy or vacancy concept at all**, and nothing stops two users occupying
  one role. #2 should treat single-occupancy as a constraint VoxLoop invents, and note that the "a loop that
  is one person" case #4 dissolved into "a role with a single-occupant constraint" has no prior implementation
  anywhere.
- **Do not adopt** `domain` and `project`. That is two levels of tenancy the map explicitly excludes.

### #6 — audio transport: **the requirement list changes**

- **R2 needs re-justifying, not deleting.** #3 treats "one stream per publisher" as load-bearing *because* of
  per-entity volume sliders. openvocs shows that what operators actually use — per-**loop** volume — is
  satisfiable by a server-side per-user mixer, and #4's own survey shows per-loop is the universal mechanism.
  R2 must now be argued on its surviving merits: client-side ducking and interruptive priority, per-talker
  identification without a signalling side-channel, and not foreclosing per-stream processing.
- **Split R4 into two questions.** "Does the *client* receive separate streams?" and "does the *architecture*
  preserve separate streams for sinks?" openvocs answers **no** and **yes**, and the second is the one the map
  actually constrains. Every candidate in #3 should be re-scored on: *can a new decoded-audio sink be attached
  without modifying the media path or adding load to the media server?* openvocs' multicast bus is the
  benchmark answer and nothing in #3 was measured against it.
- **R3 gets a benchmark.** openvocs enforces emission in the media plane — the proxy holds a per-session set of
  destination groups and a client cannot reach a loop that is not in it. #3 flagged LiveKit's publisher-driven
  permission model as its one soft spot on R3; openvocs shows what the strong form looks like and that it is
  cheap.
- **A quantified warning about server-side mixing.** One OS process per connected user, 50 by default. If any
  mixing architecture is considered for VoxLoop, the per-user cost model is the thing to interrogate first.
- **openvocs is not a transport candidate.** Do not add it to #6's comparison table.

### #9 — client tiers and PTT: **one open question changes, one stays shut**

- **Stays shut.** openvocs does not solve unfocused PTT and offers no desktop client. Wrapping it in Tauri
  would meet #5's Linux walls unchanged. The map's *"blocked on a single fact from Ed: what OS do the operator
  consoles run?"* is untouched by this ticket, and the map's note that openvocs *"may reshape the client story
  entirely"* is now answered: **it does not.** The Tauri-plus-browser two-tier plan survives.
- **Changes.** **WebHID.** #5 concluded headset PTT buttons are uncatchable; openvocs catches them from
  `navigator.hid` `inputreport` events in shipped browser code. That is a PTT input path for the *browser*
  tier that #1 currently rules out (it limits browsers to on-screen PTT and latch), and possibly a path to
  unfocused PTT with no desktop application at all. Chromium-only, secure-context, one-time permission grant
  per device. **Whether it fires while unfocused is unestablished and is a one-hour spike.** If it does, the
  case for a Tauri tier weakens considerably; if it does not, nothing is lost.
- **Also worth carrying into #9:** the Gamepad API as a PTT input (a £15 USB footswitch or button box that is
  not a keyboard), and the `SECURE_VOICE_PTT` posture — sites where a hardware button gates the microphone
  physically and the software only *reports* the state. Both are cheap, and the second is the only honest
  answer to "the UI must never misrepresent whether your mic is open".

---

## 11. Sources

**Primary — source code** (all at `openvocs/openvocs` commit `83ad2da`, `main`, v2.6.2, unless noted):

- `LICENSE`, `LICENSES/`, `README.md`, `changelog.md`, `Handbook_DRAFT/User_Handbook_v1_2.md`
- `src/service/makefile`, `src/service/*/README`, `src/service/ov_mc_vocs/config/default_config.json`
- `src/lib/ov_vocs_db/include/ov_vocs_permission.h`, `.../ov_vocs_db.h`, `.../ov_vocs_db_persistance.h`
- `src/lib/ov_vocs/src/ov_vocs.c`, `.../ov_vocs_api.inc`, `.../ov_vocs_loop.c`, `.../ov_mc_mixer_core.c`,
  `.../ov_vocs_recorder.c`
- `src/lib/ov_vocs/include/ov_mc_loop.h`, `.../ov_mc_mixer_core.h`, `.../ov_vocs_loop.h`, `.../ov_vocs_recorder.h`
- `src/lib/ov_ice_proxy/src/ov_ice_proxy_vocs.c`
- `src/lib/ov_core/include/ov_event_api.h`, `.../ov_recorder_events.h`
- `src/HTML/config_vocs.js`, `src/HTML/VERSION.js`, `src/HTML/lib/ov_vocs.js`, `.../ov_media/ov_audio.js`,
  `src/HTML/app/vocs/views/loops/js/ui/ptt_bar.js`, `.../loop_view.js`
- `resources/systemd/global/*`, `resources/debian/*`, `resources/config/ov_alsa_gateway_config.json`,
  `resources/config/ov_recorder_config.json`
- Branch `aes`: `src/lib/ov_format/include/ov_format_aes.h` (2026-07-06)

**Primary — repository metadata:** `git log`, `git shortlog -sne --all`, `git for-each-ref`, and the GitHub
REST API for `repos/openvocs/openvocs` (issues, pulls, releases, forks, org). Retrieved 2026-08-15.

**Primary — first-party web:**

- <https://github.com/openvocs/openvocs>
- <https://openvocs.com/> and <https://openvocs.com/imprint.html> (commercial extensions; imprint)
- DLR. *GSOC publishes open source software platform "openvocs" for communication in mission control* (2025).
  <https://www.dlr.de/en/rb/latest/news/2025/gsoc-publishes-open-source-software-platform-openvocs-for-communication-in-mission-control>
- DLR. *OPENVOCS®* project page.
  <https://www.dlr.de/en/rb/research-operation/research-projects/control-center-technology/openvocs-r>
- Schiffner, Bertard, Beer & Töpfer. *Openvocs, a light-weight Voice Communication System for Space Mission
  Control.* SpaceOps 2023, Dubai. <https://elib.dlr.de/201558/> — **abstract only; full text DLR-internal.**

**Not reached:** `openvocs.org` (TLS handshake failure from this environment, both WebFetch and `curl -k`);
SpaceOps 2023 full text; any published benchmark; any pricing for the commercial modules.

**VoxLoop internal:** [issue #1](https://github.com/edwardhutchinson/voxloop/issues/1) (map),
[issue #3](https://github.com/edwardhutchinson/voxloop/issues/3) and `docs/research/audio-transport-options.md`,
[issue #4](https://github.com/edwardhutchinson/voxloop/issues/4) and `docs/research/voice-loop-prior-art.md`,
[issue #5](https://github.com/edwardhutchinson/voxloop/issues/5), `inital-ramble.md`.

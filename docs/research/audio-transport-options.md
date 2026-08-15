# Self-hostable audio transport options

Research note for [issue #3](https://github.com/edwardhutchinson/voxloop/issues/3), against the constraints on the
wayfinder map ([issue #1](https://github.com/edwardhutchinson/voxloop/issues/1)).

**Status:** research only. This note deliberately does **not** pick a transport — that is a separate ticket
(#6, which this blocks). It surfaces evidence and trade-offs.

**Researched:** 2026-08-15. Every claim below is cited to a primary source — official project documentation,
the project's own source tree, licence files, or first-party API references. Where a source could not be found
or contradicted itself, that is stated explicitly rather than papered over. No blog posts or secondary
write-ups were used as evidence.

---

## 1. The requirements this is judged against

Pulled from the map and the ticket. These are the columns of the comparison.

| # | Requirement | Why it's hard |
|---|---|---|
| **R1** | **Hard self-hosting.** Runs entirely inside the customer network, no external runtime dependency, possibly air-gapped. | Any phone-home, licence server or cloud-coupled control plane is close to disqualifying. |
| **R2** | **One stream per publisher.** A subscriber must receive each publisher as a *separate* stream so it can mix locally at per-source volume. | Rules out anything that mixes conference audio server-side. This is the load-bearing product requirement (per-entity volume sliders). |
| **R3** | **Server-enforced selective subscription.** The server decides which publishers a given subscriber receives, as a permission boundary — not a client-side filter. | A client-side filter is not a permission model. |
| **R4** | **Decoded-audio tap.** The server must be able to hand decoded audio to a sink. Recording is not v1, but must not be foreclosed. | Rules out E2EE and P2P mesh. |
| **R5** | **Scale.** ~200 connected, ~5 simultaneous publishers, large listen-only population, spike load. | Modest by SFU standards — but listen-only fan-out is the shape that matters. |
| **R6** | **TURN / NAT / VPN.** Single-site LAN primary, remote users over corporate VPN. | Don't assume LAN conditions. |
| **R7** | **Clients.** SvelteKit web app + Tauri desktop app. Back end open. | Tauri is a *system webview*, not Chromium everywhere — see §9. |
| **R8** | **Operational weight.** Single box, v1, solo developer with coding agents. | Every extra daemon is a tax. |

---

## 2. Comparison

Legend: **Y** = verified from a primary source · **N** = verified absent/contradicted · **~** = partial or
conditional · **?** = not verified from a primary source.

| | LiveKit | mediasoup | Janus (VideoRoom) | Jitsi (JVB) | Pion (custom SFU) | Mumble |
|---|---|---|---|---|---|---|
| **Licence** | Apache-2.0 | ISC | **GPL-3.0** | Apache-2.0 | MIT | BSD-3-Clause |
| **What it is** | Complete SFU product + SDKs | Node/Rust *library* | Standalone C server + plugins | Component of the Jitsi Meet stack | WebRTC *library* | Complete VoIP product (non-WebRTC) |
| **R1 self-host** | Y | Y | Y | Y | Y | Y |
| **R1 phone-home** | N (see §3.2) | N (nothing to phone) | N | ? | N | N by default (§8.2) |
| **R2 separate streams** | Y | Y | Y | Y | Y | Y |
| **R3 server-enforced** | ~ (§3.4 — real, but publisher-driven API) | Y (you write it) | ~ | ? | Y (you write it) | Y (channel ACLs) |
| **R4 decoded tap** | **Y — PCM over WebSocket, first-party** | ~ (encoded RTP out; you decode) | ~ (encoded RTP out; AudioBridge decodes but mixes) | ~ (Jibri = Chrome + ffmpeg) | ~ (you write it) | N (no first-party server-side tap) |
| **R5 scale headroom** | Y (published audio benchmark, §3.5) | ~ (~500 consumers/worker, §4.4) | ? | ? | ? | ? |
| **R6 TURN** | **Y — embedded TURN + STUN** | N (bring coturn) | N (bring coturn) | ? | N (bring your own) | N/A (own TCP/UDP protocol) |
| **R7 web client** | Y (`client-sdk-js`) | Y (`mediasoup-client`) | Y (`janus.js`) | Y (`lib-jitsi-meet`) | you write it | **N** (no first-party web client) |
| **R7 native Tauri path** | **Y (`livekit` Rust crate)** | ~ (`libmediasoupclient`, C++) | ? | ? | Y (Go/Rust) | ~ (C++ client; no first-party Rust lib) |
| **R7 server SDKs** | Go, Node, Python, Rust, Ruby, Kotlin | Node, Rust (it *is* the server) | HTTP/WS API, any language | XMPP/Colibri | Go | gRPC/Ice control plane |
| **R8 daemons on the box** | livekit-server (+ Redis for egress, + egress) | your app process only | janus (+ your signalling) | prosody + jicofo + jvb + nginx | your app process only | mumble-server |
| **You must build signalling** | No | **Yes** | Partly | No (XMPP) | **Yes** | No |

**Headline:** on the literal requirement set, **LiveKit** is the only candidate where every one of R1–R7 is
satisfied by a *first-party, documented* feature rather than by something VoxLoop would have to build. Its costs
are operational weight (§3.8) and one soft spot on R3 (§3.4). **Mumble** is the surprise: it already implements
the voice-loop product model almost exactly (§8), but has no web client, which collides head-on with R7.

---

## 3. LiveKit

### 3.1 Licence

`livekit/livekit` is **Apache License 2.0** —
[LICENSE](https://raw.githubusercontent.com/livekit/livekit/master/LICENSE). Same for the surrounding stack:
`livekit/egress`, `livekit/client-sdk-js` and `livekit/rust-sdks` all report `Apache-2.0` via the GitHub API
(`gh api repos/livekit/egress --jq '.license.spdx_id'` etc.). No copyleft obligation on VoxLoop's own code.

### 3.2 Self-hosting and phone-home (R1)

The self-hosting deployment guide describes the server, an optional TURN, and Redis, and never mentions a
licence server, activation, or a required connection to LiveKit Cloud —
[docs.livekit.io/home/self-hosting/deployment](https://docs.livekit.io/home/self-hosting/deployment/). Redis is
not required for a single node: the sample config says *"when redis is set, LiveKit will automatically operate
in a fully distributed fashion"* —
[config-sample.yaml](https://raw.githubusercontent.com/livekit/livekit/master/config-sample.yaml). Authentication
is a local API key/secret pair in that same config file (`keys: key1: secret1`), not a remote check.

I went to the source rather than take the docs' silence as proof. `pkg/telemetry/analyticsservice.go` defines an
`AnalyticsService` interface with a `NullAnalyticService` implementation, and the constructor in the open-source
build hard-codes an empty key:

```go
func NewAnalyticsService(_ *config.Config, currentNode routing.LocalNode) AnalyticsService {
	return &analyticsService{
		analyticsKey: "", // TODO: conf.AnalyticsKey
		nodeID:       string(currentNode.NodeID()),
	}
}
```

and each send method early-returns when its stream client is nil (`if a.stats == nil { return }`) —
[analyticsservice.go](https://raw.githubusercontent.com/livekit/livekit/master/pkg/telemetry/analyticsservice.go).
The `pkg/telemetry` package is otherwise Prometheus-backed and local. **Conclusion: no phone-home in the
open-source server.**

**Two air-gap gotchas, both configurable, both real:**

1. The sample config comments say *"by default LiveKit clients use Google's public STUN servers"* and
   `use_external_ip: true` *"attempts to discover the host's public IP via STUN"* —
   [config-sample.yaml](https://raw.githubusercontent.com/livekit/livekit/master/config-sample.yaml). In an
   air-gapped deployment both must be turned off / repointed. The embedded TURN server *"also serves as a STUN
   server"* when enabled — [ports & firewall](https://docs.livekit.io/home/self-hosting/ports-firewall/) — so
   there is an in-network answer.
2. The recommended single-VM install generates a Caddy config that provisions TLS *"by using Let's Encrypt or
   ZeroSSL"* — [self-hosting/vm](https://docs.livekit.io/home/self-hosting/vm/). That is the *recommended*
   path, not a server dependency; an internal CA certificate replaces it. Worth budgeting as packaging work.

### 3.3 Separate stream per publisher (R2)

Confirmed. LiveKit models media as `TrackPublication` + `Track` per remote participant, individually
addressable; with `autoSubscribe` disabled *"Only explicitly subscribed tracks are delivered to the
participant"*, and a client calls `setSubscribed(true)` per `RemoteTrackPublication` —
[client/tracks/subscribe](https://docs.livekit.io/home/client/tracks/subscribe/). No server-side mixing
anywhere in the path. Per-source volume is then a client concern.

### 3.4 Server-enforced selective subscription (R3) — **read this one carefully**

There are three distinct mechanisms and they are not equivalent:

1. **Token grant `canSubscribe`** — *"Allow participant to subscribe to other tracks"*, an all-or-nothing
   boolean in the signed JWT video grant. The full grant table (`roomJoin`, `roomAdmin`, `canPublish`,
   `canPublishSources`, `canSubscribe`, `hidden`, …) is at
   [tokens & grants](https://docs.livekit.io/frontends/reference/tokens-grants/). Tokens are *"JWT-based and
   signed with your API secret to prevent forgery"*. This gives a per-room boundary, not a per-publisher one.

2. **`RoomService.UpdateSubscriptions`** — a server-side RPC, *"Subscribes or unsubscribe a participant from
   tracks. Requires `roomAdmin`"* —
   [livekit_room.proto](https://raw.githubusercontent.com/livekit/protocol/main/protobufs/livekit_room.proto).
   This *pushes* subscription changes from the backend but, on its own, does not stop a client with
   `canSubscribe: true` from resubscribing.

3. **`SubscriptionPermission` / `TrackPermission`** — the actual per-publisher allow-list:

   ```protobuf
   message TrackPermission {
     // permission could be granted either by participant sid or identity
     string participant_sid = 1;
     bool all_tracks = 2;
     repeated string track_sids = 3;
     string participant_identity = 4;
   }

   message SubscriptionPermission {
     bool all_participants = 1;
     repeated TrackPermission track_permissions = 2;
   }
   ```
   — [livekit_rtc.proto](https://raw.githubusercontent.com/livekit/protocol/main/protobufs/livekit_rtc.proto).

   **This is enforced inside the SFU, not in the client.** `UpTrackManager.hasPermissionLocked` returns `false`
   for any subscriber identity absent from `subscriberPermissions`, and `maybeRevokeSubscriptions()` tears down
   subscriptions that a permission change has invalidated —
   [uptrackmanager.go](https://raw.githubusercontent.com/livekit/livekit/master/pkg/rtc/uptrackmanager.go).

   **The catch:** the only path that *sets* it is a signal message from the **publishing participant** —
   `ParticipantImpl.HandleUpdateSubscriptionPermission` →
   `Room.onUpdateSubscriptionPermission` → `UpTrackManager.UpdateSubscriptionPermission`
   ([participant.go](https://raw.githubusercontent.com/livekit/livekit/master/pkg/rtc/participant.go),
   [room.go](https://raw.githubusercontent.com/livekit/livekit/master/pkg/rtc/room.go)) — surfaced in the client
   SDKs as `LocalParticipant.setTrackSubscriptionPermissions(allParticipantsAllowed, participantTrackPermissions)`
   ([JS SDK reference](https://docs.livekit.io/reference/client-sdk-js/classes/LocalParticipant.html)). I found
   **no `RoomService` RPC** that sets `SubscriptionPermission` from a trusted backend — it is not in the RPC list
   in `livekit_room.proto`.

   > **Trade-off to carry into the decision ticket.** LiveKit's per-publisher subscription boundary is genuinely
   > server-enforced but **publisher-driven**. A VoxLoop backend gets it either by (a) making rooms the
   > permission boundary and issuing per-room `canSubscribe` tokens, (b) driving `setTrackSubscriptionPermissions`
   > from a trusted client the server controls, or (c) patching the server. (a) is the natural fit if a *loop* maps
   > to a *room*, but a talker publishing to N loops then needs N publications. This is an architecture question,
   > not a capability gap — but it is the sharpest edge in the LiveKit story.

Runtime permission changes are supported: `UpdateParticipant` *"Update participant metadata … Requires
`roomAdmin`"* carries a `ParticipantPermission`, and *"connected clients are notified through a
`ParticipantPermissionChanged` event"* — [managing participants](https://docs.livekit.io/home/server/managing-participants/).
That maps cleanly onto the map's "admin can demote a talker mid-event" requirement.

### 3.5 Decoded-audio tap (R4) — LiveKit's strongest differentiator

Track egress exports individual tracks, and for audio: *"The tracks will be exported as raw PCM data"*,
`pcm_s16le`, *"Sample rate: matches incoming, typically 48kHz"*, and *"WebSocket streaming is only available for
audio tracks"* — [egress/track](https://docs.livekit.io/home/egress/track/). The overview lists *"Streaming audio
tracks to captioning services via WebSocket, exporting raw track data for processing"* as the use case, and notes
per-track egress *"started once per track"* gives separate audio files per participant —
[egress overview](https://docs.livekit.io/home/egress/overview/).

This is exactly R4: **decoded PCM, per publisher, streamed to a sink we choose, with no processing on our side.**
Of every candidate researched, LiveKit is the only one that ships this as a first-party service rather than as
"forward the RTP somewhere and decode it yourself".

Cost: egress is a **separate service**. *"If you're self-hosting LiveKit, egress must be deployed separately"*
([overview](https://docs.livekit.io/home/egress/overview/)) and *"The Egress service uses Redis messaging queues
to load balance and communicate with your LiveKit server"*, with *"at least 4 CPUs and 4 GB of memory"* per
instance — [self-hosting/egress](https://docs.livekit.io/home/self-hosting/egress/). Chrome is required only for
composite recording; *"track exports consume minimal resources"*. No external service is required. Since
recording is not v1, this cost is deferrable — the seam exists and stays open.

### 3.6 Scale (R5)

LiveKit publishes a benchmark run with `lk load-test` on a 16-core `c2-standard-16`. The **audio-only** row:
**10 publishers, 3000 subscribers**, 23 MBps out, 959,156 packets/s, **80% CPU** —
[self-hosting/benchmark](https://docs.livekit.io/home/self-hosting/benchmark/). VoxLoop's envelope (5 publishers,
200 connected) is more than an order of magnitude inside that. The listen-only fan-out shape — the thing the map
worries about — is precisely the shape LiveKit benchmarked.

*Caveat:* this is the vendor's own benchmark on a compute-optimised cloud instance, not an independent one, and
not on a customer's box. It bounds the answer; it doesn't replace a load test.

### 3.7 TURN / NAT (R6)

Embedded TURN, no separate coturn: TURN/UDP on 3478 and TURN/TLS on 5349, both optional, and *"To use the
embedded TURN/UDP server. When enabled, it also serves as a STUN server"* —
[ports & firewall](https://docs.livekit.io/home/self-hosting/ports-firewall/). The config struct confirms it is
first-class (`TURNConfig` with `Enabled`, `Domain`, `CertFile`, `KeyFile`, `TLSPort`, `UDPPort`, relay port range,
per-user relay allocation limit) —
[pkg/config/config.go](https://raw.githubusercontent.com/livekit/livekit/master/pkg/config/config.go). ICE/TCP
fallback on 7881 and a UDP-mux mode are also available. For corporate-VPN users this is the difference between
one daemon and two.

### 3.8 Clients and SDKs (R7)

- **Web:** `livekit/client-sdk-js`, Apache-2.0 — drops into SvelteKit.
- **Tauri desktop:** `livekit/rust-sdks` publishes the `livekit` crate (`livekit-api` for server APIs and token
  generation, `livekit` for the realtime SDK), with Windows/macOS/Linux listed as supported platforms and
  "Receiving tracks" / "Publishing tracks" ticked —
  [rust-sdks README](https://raw.githubusercontent.com/livekit/rust-sdks/main/README.md). This matters more
  than it looks — see §9 on Tauri webviews.
- **Server:** first-party SDKs for Go, Node, Python, Rust, Ruby and Kotlin, plus a CLI — visible as maintained
  repos in the `livekit` GitHub org (`server-sdk-go`, `node-sdks`, `python-sdks`, `rust-sdks`,
  `server-sdk-ruby`, `server-sdk-kotlin`, `livekit-cli`). The back end is genuinely open.

### 3.9 Operational weight (R8)

One Go binary for v1 (no Redis needed single-node). Add Redis + an egress instance when recording lands. There
is a `deploy` repo and a documented single-VM generator. Heaviest of the "product" options, lightest of the
"assemble it yourself" options.

---

## 4. mediasoup

### 4.1 Licence and self-hosting

**ISC**, © Iñaki Baz Castillo — [LICENSE](https://raw.githubusercontent.com/versatica/mediasoup/v3/LICENSE).
`mediasoup-client` is likewise ISC. Permissive; nothing to phone home to, because there is no product — see next.

### 4.2 It is a library, not a server (R8, and the dominant fact)

*"mediasoup is not a standalone server but an unopinionated Node.js module which can be integrated into a larger
application"* — [design](https://mediasoup.org/documentation/v3/mediasoup/design/). It is a JS/Rust API over a
C/C++ media worker subprocess, the two communicating by IPC.

And critically: *"mediasoup does not provide any signaling protocol to communicate clients and server. It's up to
the application communicate them by using WebSocket, HTTP or whichever communication means"* —
[communication between client and server](https://mediasoup.org/documentation/v3/communication-between-client-and-server/).

**Every one of R3, presence, permissions, room state and reconnection is code VoxLoop writes.** For a solo
developer that is the whole trade: maximum control, maximum surface area.

### 4.3 Streams, subscription and the tap (R2, R3, R4)

R2 is satisfied by construction: a `Consumer` is created by `transport.consume()` against a specific `producerId`,
so a client creates one Consumer per Producer it wants, each independently pausable/resumable —
[API](https://mediasoup.org/documentation/v3/mediasoup/api/). There is no mixing anywhere.

R3 is satisfied *because* you write the signalling: the server decides whether to create a Consumer at all, so
the boundary is absolute and trivially auditable. This is arguably a **cleaner** answer to R3 than LiveKit's
(§3.4) — at the cost of building everything around it.

R4: `PlainTransport` *"represents a network path through which RTP, RTCP (optionally secured with SRTP) and SCTP
(DataChannel) is transmitted"* — the documented route to non-WebRTC endpoints (ffmpeg/GStreamer) —
and `DirectTransport` delivers RTP packets into the Node application itself
([API](https://mediasoup.org/documentation/v3/mediasoup/api/)). Both give you **encoded Opus RTP**; decoding to
PCM is on VoxLoop or on the sink. The seam exists, but it is not the turnkey PCM-over-WebSocket LiveKit ships.

### 4.4 Scale (R5)

*"A Worker represents a mediasoup C++ subprocess that runs in a single CPU core"* and *"a mediasoup C++
subprocess can typically handle over ~500 consumers in total"*; the docs warn that with more than *"200-300
viewers (so 400-600 consumers), the capabilities of a single mediasoup router could be exceeded"*, and recommend
launching one worker per core and distributing routers, joined by `PipeTransport` —
[scalability](https://mediasoup.org/documentation/v3/scalability/).

**This bites at VoxLoop's stated envelope.** 200 subscribers × 5 publishers ≈ 1000 consumers — above a single
worker. It is a solved problem (multiple workers, `PipeTransport` between routers, all on one box) but it is
*design work in v1*, not a config flag. Contrast LiveKit, where the same shape sits inside a single benchmarked
process.

### 4.5 TURN and clients (R6, R7)

No TURN, no STUN, no ICE-server management — mediasoup handles its own ICE as the WebRTC endpoint, but a relay
for restrictive networks is not in scope; coturn is a separate daemon you run. I did not find a first-party
mediasoup statement on TURN, so treat "bring your own coturn" as inferred from the absence of the feature rather
than as a cited claim.

Clients: `mediasoup-client` (browser JS — fine for SvelteKit), `libmediasoupclient` (C++ on libwebrtc),
`mediasoup-client-aiortc` (Python) — [documentation index](https://mediasoup.org/documentation/v3/). There is a
first-party **Rust** implementation of the *server* side, but the native *client* story for Tauri is C++
(`libmediasoupclient`), not Rust. For Tauri that means either the webview path (see §9) or FFI work.

---

## 5. Janus

### 5.1 Licence — the headline risk

**GPL-3.0**, with a Meetecho OpenSSL linking exception —
[COPYING](https://raw.githubusercontent.com/meetecho/janus-gateway/master/COPYING), badge confirmed in the
[README](https://raw.githubusercontent.com/meetecho/janus-gateway/master/README.md).

VoxLoop's model is **shipping software to customers to run on-premise**, i.e. conveying it. GPLv3 obligations
therefore engage on whatever is a derivative work. Talking to a stock Janus over its HTTP/WebSocket API from a
separate process is the conventional arm's-length arrangement; **writing a custom Janus plugin** (which a
voice-loop product would very plausibly want, for server-side loop semantics) produces a work that links against
Janus internals and is much harder to argue is not derivative.

> **I am not offering a legal conclusion — I am flagging that one is needed.** This is the only candidate where
> the licence is a live commercial question rather than a formality, and it should be resolved before Janus is
> costed, not after.

### 5.2 AudioBridge vs VideoRoom (R2 — a trap worth naming)

Janus ships two relevant plugins and only one of them can satisfy R2.

- **AudioBridge** is *"a plugin implementing an audio conference bridge for Janus, specifically mixing Opus
  streams"* — [audiobridge docs](https://janus.conf.meetecho.com/docs/audiobridge.html). It is superficially the
  obvious choice for a voice product, and it is **disqualified**: it mixes server-side, so a subscriber gets one
  combined stream and cannot apply per-source volume. Its `volume` parameter (*"percent value, <100 reduces
  volume, >100 increases volume"*) is per-participant gain *into the mix*, identical for every listener — not the
  per-listener control R2 demands. Its spatial-audio panning is likewise a property of the mix.
- **VideoRoom** is *"a videoconferencing SFU (Selective Forwarding Unit)"* / *"an audio/video router"* —
  [videoroom docs](https://janus.conf.meetecho.com/docs/videoroom.html) — and does satisfy R2. Despite the name it
  handles audio-only rooms (`audiocodec = opus|g722|pcmu|pcma|isac32|isac16`).

**So: Janus is a candidate only via VideoRoom, used audio-only.**

### 5.3 Subscription control and access (R3)

VideoRoom subscribers use `subscribe` (*"allows you to add more streams to subscribe to"*), `unsubscribe`
(*"instructs the plugin to remove streams you're currently subscribe to"*) and `update`, naming publisher feeds
and `mid`s. Subscriptions can be bulk (one PeerConnection, many publishers) or one PeerConnection per publisher:
*"subscriptions can be done either in 'bulks' … or separately"*. Access control exists at room level — PINs,
token ACLs (*"Array of strings (tokens users might pass in 'join')"*) and `require_pvtid = true|false`
(*"whether subscriptions are required to provide a valid private_id"*) —
[videoroom docs](https://janus.conf.meetecho.com/docs/videoroom.html).

**Not verified:** whether VideoRoom can enforce a *per-subscriber, per-publisher* allow-list within a single room
(the R3 shape), as opposed to room-level admission via PIN/token/`private_id`. The documentation describes
room-scoped access control; I found no primary statement of an intra-room per-publisher permission matrix.
Treat R3-within-a-room as **unverified** for Janus.

### 5.4 Tap (R4)

`rtp_forward` *"forwards in real-time the media sent by a publisher via RTP (plain or encrypted) to a remote
backend"*, and per-publisher recording is a room/publisher flag (`record = true|false`) producing `.mjr` files —
[videoroom docs](https://janus.conf.meetecho.com/docs/videoroom.html). AudioBridge additionally forwards *the
mix* and can record participants individually. So: a per-publisher **encoded RTP** seam, yes; decoded PCM, no —
that's on the sink.

### 5.5 Everything else

Scale, TURN and operational figures for Janus at VoxLoop's shape were **not verified from primary sources** in
this pass. Janus has no bundled TURN; a coturn deployment is the usual arrangement, but I am not citing that as a
Janus claim. Client side, `janus.js` is the first-party browser library; I found no first-party Rust client, so
Tauri would go through the webview (§9). Server side, Janus is language-agnostic — it is driven over
HTTP/WebSocket/RabbitMQ/etc., which suits any back end.

---

## 6. Jitsi / JVB

Jitsi Videobridge *"is a WebRTC-compatible Selective Forwarding Unit (SFU), i.e. a multimedia router. It is one
of the backend components in the Jitsi Meet stack"* —
[JVB README](https://raw.githubusercontent.com/jitsi/jitsi-videobridge/master/README.md). Apache-2.0 (both
`jitsi-videobridge` and `jitsi-meet`). As an SFU it forwards separate streams, so R2 is fine in principle.

**The problem is that JVB is not designed to be driven by someone else's application.** Jicofo is the signalling
server: it joins an XMPP MUC and *"is then responsible for initiating a Jingle session with each participant"*,
and *"manages the set of videobridges for the conference with the COLIBRI protocol (colibri version 2 …)"* —
[Jicofo README](https://raw.githubusercontent.com/jitsi/jicofo/master/README.md). A self-hosted install is
jitsi-meet + jicofo + jitsi-videobridge + **Prosody** (XMPP) as separate components —
[Jitsi architecture](https://jitsi.github.io/handbook/docs/architecture/). The
[requirements page](https://jitsi.github.io/handbook/docs/devops-guide/devops-guide-requirements/) suggests
*"8 GB"* RAM and notes Prosody *"can only use ONE (1) core"*.

For R8 that is four daemons and an XMPP dependency to carry a voice-loop app — the heaviest operational footprint
of any candidate, in service of a meeting product VoxLoop is not building.

R3 is **unverified and looks unpromising**: JVB's per-endpoint forwarding controls are video-oriented. The
bridge's constraints document describes `SenderSourceConstraints`, where *"The bridge sends the following message
to a sender to notify it that resolutions higher than the specified need not be transmitted for a specific video
source"* — [doc/constraints.md](https://raw.githubusercontent.com/jitsi/jitsi-videobridge/master/doc/constraints.md).
I found **no primary evidence** of a per-endpoint audio-subscription permission boundary. Absence of evidence is
not evidence of absence, but for a hard product requirement it is the wrong shape of unknown.

R4 exists via Jibri — *"a set of tools for recording and/or streaming a Jitsi Meet conference that works by
launching a Chrome instance rendered in a virtual framebuffer and capturing and encoding the output with ffmpeg"*
([architecture](https://jitsi.github.io/handbook/docs/architecture/)), *"One Jibri instance = one meeting"* at
8–12 GB RAM ([requirements](https://jitsi.github.io/handbook/docs/devops-guide/devops-guide-requirements/)). That
is a *mixed conference* recording via a headless browser, not a per-publisher decoded tap — the wrong seam for
VoxLoop, and very expensive.

**Assessment: not disqualified on capability, but poorly aligned.** VoxLoop would be adopting a meeting product's
signalling stack to get an SFU, and the R3/R4 seams point the wrong way.

---

## 7. Pion (custom SFU)

`pion/webrtc` is **MIT**, © The Pion community —
[LICENSE](https://raw.githubusercontent.com/pion/webrtc/master/LICENSE) — *"A pure Go implementation of the
WebRTC API"* ([README](https://raw.githubusercontent.com/pion/webrtc/master/README.md)). It is what LiveKit is
built on: the `livekit` GitHub org carries forks `webrtc-pion` (*"Pure Go implementation of the WebRTC API"*),
`ice` and `dtls`.

R1, R2, R3 and R4 are all achievable and all **entirely VoxLoop's code**: no licence friction, no phone-home, no
imposed model, total control of the permission boundary and the tap. Nothing about Pion is disqualifying.

The question is not capability, it's cost, and there is a concrete signal about the off-the-shelf shortcut:
- `pion/ion-sfu` (*"Pure Go WebRTC SFU"*, MIT, ~1.1k stars) — **last commit 2022-11-15**, last push
  2023-07-21 (`gh api repos/pion/ion-sfu/commits`, `gh repo view pion/ion-sfu`). Effectively unmaintained.
- `pion/ion` describes itself as *"A work-in-progress ION SFU remake"* (MIT), actively pushed as of 2026-08-15
  ([README](https://raw.githubusercontent.com/pion/ion/master/README.md)). **Work-in-progress**, by its own
  description.

**Assessment:** Pion is the right *substrate* and the wrong *v1 deliverable* for a solo developer — you would be
writing the SFU, the signalling, the permission enforcement, the congestion control tuning, the reconnection
logic and the egress path that LiveKit already ships under the same Apache-family licensing. Worth keeping in
view as the escape hatch if a chosen product blocks on something (e.g. LiveKit's §3.4 edge), not as the starting
point.

**A lightweight middle option: Galène** — *"an efficient, low-resource videoconference system (web client and
server) that is easy to host and easy to administer"*, MIT, Go/Pion-based, with an SFU architecture, group
permissions, *"recording to disk"* and a *"built-in TURN server"* ([galene.org](https://galene.org/); repo
`jech/galene`, MIT, pushed 2026-07-28). It is a complete *application* with its own web client rather than an
embeddable SFU, and I did **not** verify any server SDK, embedding story, or per-subscriber permission API. Noted
for completeness; not researched to decision depth.

---

## 8. Mumble — the non-WebRTC candidate that already models the product

This one was not on the ticket's candidate list and it should have been. Mumble is BSD-3-Clause
([LICENSE](https://raw.githubusercontent.com/mumble-voip/mumble/master/LICENSE)), *"an Open Source, low-latency
and high-quality voice-chat program written on top of Qt and Opus"*, shipping *"the client (mumble) and the
server (mumble-server formerly known as murmur)"*
([README](https://raw.githubusercontent.com/mumble-voip/mumble/master/README.md)).

### 8.1 It implements the voice-loop model natively (R2, R3)

From the server's own voice-routing function, `Server::processMsg` in
[src/murmur/Server.cpp](https://raw.githubusercontent.com/mumble-voip/mumble/master/src/murmur/Server.cpp):

- The server **adds receivers for an incoming audio payload and forwards it** — it never mixes. `buffer.addReceiver(...)`
  is called per destination user; the payload is passed through.
- Users can **listen to channels they are not in**: `m_channelListenerManager.getListenersForChannel(c->iId)`,
  with audio delivered in `AudioContext::LISTEN`. The 1.4 release announcement describes the feature: *"This
  feature allows a user to 'listen to' a channel. In that case all audio that is heard by people in this
  particular channel (be it by direct communication, shouts or via linked channels) is also heard by the
  listening user"* ([mumble.info/blog/mumble-1.4.230](https://www.mumble.info/blog/mumble-1.4.230/)). That *is*
  loop subscription.
- **Per-source volume is in the protocol.** `Mumble.proto` carries
  `repeated uint32 listening_channel_add`, `listening_channel_remove`, and
  `repeated VolumeAdjustment listening_volume_adjustment` where
  `VolumeAdjustment { optional uint32 listening_channel = 1; optional float volume_adjustment = 2; }` —
  [src/Mumble.proto](https://raw.githubusercontent.com/mumble-voip/mumble/master/src/Mumble.proto). The server
  passes `getListenerVolumeAdjustment(...)` per receiver in `processMsg`; the client applies the gain. Per-user
  local volume adjustment has existed since 1.3.0 (*"Individual user volume adjustment (local)"* —
  [1.3.0 announcement](https://www.mumble.info/blog/mumble-1.3.0-release-announcement/)).
- **Permissions are server-enforced ACLs, per channel, with a dedicated `Listen` right.** `ChanACL::Perm` is
  `Write, Traverse, Enter, Speak, MuteDeafen, Move, MakeChannel, LinkChannel, Whisper, TextMessage,
  MakeTempChannel, Listen` plus root-only `Kick, Ban, Register, SelfRegister, ResetUserContent` —
  [src/ACL.h](https://raw.githubusercontent.com/mumble-voip/mumble/master/src/ACL.h) — and `processMsg` gates
  linked-channel forwarding on `ChanACL::hasPermission(u, l, ChanACL::Speak, &acCache)`. ACL rules are
  per-channel with inheritance, evaluated top to bottom
  ([ACL documentation](https://www.mumble.info/documentation/administration/acl/)).
- Whisper/shout gives directed talk to a channel or an individual (`AudioContext::SHOUT` / `WHISPER` in
  `processMsg`), and priority-speaker attenuation exists (*"Support attenuate others on priority speaker"*,
  [1.3.0 announcement](https://www.mumble.info/blog/mumble-1.3.0-release-announcement/)).

R2 and R3 are satisfied more completely, and more natively, than by any WebRTC option — VoxLoop's product model
is close to a re-implementation of Mumble's.

### 8.2 Self-hosting (R1)

Nothing external by default. Public-server-list registration (`registerName`, `registerPassword`, `registerUrl`,
`registerHostname`) and `bonjour` are all **commented out** in the shipped
[mumble-server.ini](https://raw.githubusercontent.com/mumble-voip/mumble/master/auxiliary_files/mumble-server.ini);
registration only happens when explicitly configured, and *"leaving this setting blank will disable registration
with the public server list"*
([config file docs](https://www.mumble.info/documentation/administration/config-file/)). Clean for air-gapped.

### 8.3 Where it fails (R4, R7)

- **R4 — no first-party server-side tap.** The server forwards *encoded* Opus payloads and never decodes them
  (evident from `processMsg`, which manipulates `audioData.payload` and receiver lists only). There is no
  recording feature in `mumble-server`; the control interfaces (Ice/gRPC) are for administration, not media. A
  tap would mean a headless bot client that joins and receives per-user streams — workable in principle,
  **unverified**, and outside anything the project supports first-party.
- **R7 — this is the disqualifier.** There is **no first-party web client**. The project ships a Qt desktop
  client; nothing in the official documentation or repository offers a browser client. VoxLoop's map requires a
  SvelteKit web app for the admin console and listen-only users. Third-party browser bridges exist but were not
  researched, are not first-party, and would put an unmaintained component on the critical path of a
  critical-path application. Similarly, there is no first-party Rust client library for the Tauri app.

> **Assessment: rules itself out on R7, but it is the best available reference design.** Mumble's channel /
> listener / ACL model — especially the `Listen` permission and per-listening-channel volume adjustment carried
> in the protocol — is worth mining for VoxLoop's domain model regardless of which transport wins. Worth a
> pointer from `CONTEXT.md` when the loop and permission models are pinned down.

---

## 9. A cross-cutting risk: Tauri is not Chromium (R7)

This affects **every** WebRTC candidate and belongs in the decision, not in a footnote.

Tauri renders in the **system webview** — WebView2 (Chromium) on Windows, WKWebView on macOS, WebKitGTK on Linux.
`getUserMedia` and WebRTC therefore behave differently per platform. Tauri's own issue tracker is the primary
record:

- **[tauri-apps/wry#85 "WebRTC support on Linux"](https://github.com/tauri-apps/wry/issues/85) is still OPEN**
  (verified `gh issue view 85 --repo tauri-apps/wry` → `state=OPEN`, `closedAt=null`).
- Related open first-party issues cover macOS permission prompting
  ([tauri#11951](https://github.com/tauri-apps/tauri/issues/11951),
  [wry#1195](https://github.com/tauri-apps/wry/issues/1195)) and the Windows case where a user who clicks *block*
  cannot be re-prompted ([tauri#5042](https://github.com/tauri-apps/tauri/issues/5042)).

The map already requires push-to-talk to work while the Tauri app is **unfocused**, which pushes microphone
capture toward native code anyway. Combining the two: **for the Tauri client, a native SDK is materially safer
than WebRTC-in-the-webview.** That is a point in LiveKit's favour specifically — the `livekit` Rust crate
(§3.8) lets the desktop client bypass the webview entirely for media while SvelteKit still renders the UI. Pion
would give the same property in Go. mediasoup's native client is C++; Janus, Jitsi and Galène have no first-party
native client I could verify, so on those the Tauri app is betting on WebKitGTK.

I have **not** verified the current WebRTC behaviour of WebKitGTK in a Tauri app by testing it — that is a
prototype question, not a documentation question, and it is worth a spike before the decision is locked.

---

## 10. Considered and ruled out

| Option | Why it's out | Source |
|---|---|---|
| **LiveKit Cloud, Daily, Agora, Twilio, Cloudflare Realtime and other managed SFUs** | Managed service — fails R1 outright. The map already settles this: *"Self-hosting an open-source SFU satisfies this; a managed service does not."* | [issue #1](https://github.com/edwardhutchinson/voxloop/issues/1) |
| **Janus AudioBridge plugin** | Mixes server-side — fails R2. (Janus survives only via VideoRoom.) | [audiobridge docs](https://janus.conf.meetecho.com/docs/audiobridge.html) |
| **Asterisk ConfBridge / FreeSWITCH conferencing** | Classic voice-loop answer, and it fails R2: ConfBridge mixes the conference server-side (the docs discuss the *"internal native sample rate the conference is mixed at"* and recommend `dsp_drop_silence` so *"the audio of users that aren't speaking isn't mixed in with the bridge"*). One mixed stream per participant means no per-source volume. | [Asterisk ConfBridge docs](https://docs.asterisk.org/Configuration/Applications/Conferencing-Applications/ConfBridge/) |
| **Ant Media Server** | The WebRTC/SFU capability sits in the **Enterprise Edition**, which is a paid per-instance licence — the repo's `ENTERPRISE_EDITION_LICENSE` file contains only *"Please reach out to contact@antmedia.io for Enterprise Edition License"*. Air-gapped/offline MAC-bound licences are documented as *available*, but a licence key is still a runtime artefact and a commercial dependency, which is exactly the shape R1 warns about. **Partly unverified:** antmedia.io blocks automated fetching, so I could not read the pricing or offline-activation pages directly. | [ENTERPRISE_EDITION_LICENSE](https://raw.githubusercontent.com/ant-media/Ant-Media-Server/master/ENTERPRISE_EDITION_LICENSE) |
| **TeamSpeak** | Closed-source, commercially licensed server. **Not verified from a primary source** — teamspeak.com returned 404 for the licensing pages I tried. Listed for completeness; do not treat the characterisation as evidenced. | — |
| **Peer-to-peer mesh (plain WebRTC, no SFU)** | Fails R4 and R5 — the map rules it out directly: the decoded-audio-tap constraint *"rules out peer-to-peer mesh and end-to-end encryption."* | [issue #1](https://github.com/edwardhutchinson/voxloop/issues/1) |
| **Anything E2EE by default (e.g. Element Call / Matrix RTC)** | Same clause — end-to-end encryption forecloses the server-side tap. Not researched further. | [issue #1](https://github.com/edwardhutchinson/voxloop/issues/1) |
| **Icecast / SRT / HLS broadcast** | One-way broadcast with seconds of latency and no push-to-talk return path. Structurally wrong for a bidirectional loop system. Not researched — noted so the category isn't silently skipped. | — |

---

## 11. What the evidence settles, and what it does not

### Settled

1. **Nothing in the open-source shortlist phones home.** LiveKit verified at source level (§3.2); Mumble verified
   at config level (§8.2); mediasoup and Pion have no server to phone home; Janus and JVB are conventional
   self-hosted daemons. The only candidate with a licence-server shape is Ant Media Enterprise, and it is out.
2. **R2 is not the differentiator it looked like.** Every SFU forwards separate streams. R2 only eliminates
   *mixers* — Janus AudioBridge, Asterisk/FreeSWITCH conferencing. The real discriminators are R3, R4, R7 and R8.
3. **LiveKit uniquely ships the R4 seam as a product.** Per-track PCM (`pcm_s16le`, 48 kHz) over WebSocket,
   *"exported as is, without transcoding"*, no processing on our side (§3.5). Everyone else offers encoded RTP
   forwarding and leaves decoding to us, or (Jitsi) a headless-Chrome recording of a mixed conference.
4. **LiveKit's published audio benchmark brackets VoxLoop's envelope with an order of magnitude to spare**
   (10 publishers / 3000 subscribers / 80% CPU on 16 cores, §3.6), whereas **mediasoup's own documented per-worker
   limit (~500 consumers) sits below VoxLoop's ~1000-consumer shape** (§4.4) and forces multi-worker design in v1.
5. **Janus carries a licence question that no other candidate does** (GPLv3 + on-premise conveyance + likely
   custom plugin, §5.1).
6. **Jitsi is the heaviest operational footprint** — Prosody + Jicofo + JVB + web, XMPP-coupled, ~8 GB RAM guidance
   (§6) — for an SFU VoxLoop would be using outside its intended stack.
7. **Mumble already implements VoxLoop's product model** (channels-as-loops, cross-channel listening, per-source
   volume in the protocol, server-enforced `Listen`/`Speak` ACLs) **and cannot ship it**, because it has no
   first-party web client (§8).

### Not settled — needs the decision ticket, a spike, or a human

1. **How VoxLoop gets a server-authoritative per-publisher subscription boundary out of LiveKit** (§3.4). The
   enforcement is genuinely in the SFU, but the setter is publisher-side. Rooms-as-loops, a trusted control
   client, or a patch — this is an architecture choice with knock-on effects on the loop model, and it is the
   single most important open question in this note.
2. **Whether Janus VideoRoom can enforce per-subscriber, per-publisher permissions inside one room** (§5.3).
   Room-level PIN/token/`private_id` admission is documented; an intra-room permission matrix is not.
3. **Whether JVB can gate audio per endpoint at all** (§6). Its documented constraint machinery is video-shaped;
   I found no evidence either way for audio.
4. **Real-world WebRTC behaviour in the Tauri webview on Linux/WebKitGTK** (§9). `wry#85` is open; the answer is
   empirical, not documentary. Spike it.
5. **Scale figures for Janus, JVB, Galène and Mumble** at VoxLoop's shape. Not published, not verified. Only
   LiveKit and mediasoup publish numbers.
6. **TURN for mediasoup, Janus and Pion.** "Bring your own coturn" is an inference from feature absence, not a
   cited first-party claim. Only LiveKit's embedded TURN (§3.7) and Galène's *"built-in TURN server"* are
   documented as included.
7. **Whether Mumble's model should inform VoxLoop's domain model even though Mumble is not the transport** (§8.3).
   Recommend yes; that is a `/domain-modeling` question, not a transport one.
8. **The GPLv3 question on Janus** (§5.1) is a legal call, not an engineering one. Flagged, not answered.
9. **Ant Media's offline licensing specifics** (§10) — antmedia.io blocks automated fetching, so the offline/
   air-gapped licence claim was not read at source.
10. **Codec and latency behaviour under a corporate VPN** — nothing in this pass addresses jitter/loss behaviour of
    any candidate over VPN. The map warns *"Don't assume LAN conditions"*; no primary source answers it.

---

## 12. Sources

All fetched 2026-08-15.

**LiveKit** — [LICENSE](https://raw.githubusercontent.com/livekit/livekit/master/LICENSE) ·
[config-sample.yaml](https://raw.githubusercontent.com/livekit/livekit/master/config-sample.yaml) ·
[pkg/telemetry/analyticsservice.go](https://raw.githubusercontent.com/livekit/livekit/master/pkg/telemetry/analyticsservice.go) ·
[pkg/rtc/uptrackmanager.go](https://raw.githubusercontent.com/livekit/livekit/master/pkg/rtc/uptrackmanager.go) ·
[pkg/config/config.go](https://raw.githubusercontent.com/livekit/livekit/master/pkg/config/config.go) ·
[livekit_room.proto](https://raw.githubusercontent.com/livekit/protocol/main/protobufs/livekit_room.proto) ·
[livekit_rtc.proto](https://raw.githubusercontent.com/livekit/protocol/main/protobufs/livekit_rtc.proto) ·
[rust-sdks README](https://raw.githubusercontent.com/livekit/rust-sdks/main/README.md) ·
[self-hosting/deployment](https://docs.livekit.io/home/self-hosting/deployment/) ·
[self-hosting/vm](https://docs.livekit.io/home/self-hosting/vm/) ·
[self-hosting/egress](https://docs.livekit.io/home/self-hosting/egress/) ·
[self-hosting/benchmark](https://docs.livekit.io/home/self-hosting/benchmark/) ·
[ports & firewall](https://docs.livekit.io/home/self-hosting/ports-firewall/) ·
[client/tracks/subscribe](https://docs.livekit.io/home/client/tracks/subscribe/) ·
[server/managing-participants](https://docs.livekit.io/home/server/managing-participants/) ·
[egress/overview](https://docs.livekit.io/home/egress/overview/) ·
[egress/track](https://docs.livekit.io/home/egress/track/) ·
[tokens & grants](https://docs.livekit.io/frontends/reference/tokens-grants/) ·
[JS SDK LocalParticipant](https://docs.livekit.io/reference/client-sdk-js/classes/LocalParticipant.html)

**mediasoup** — [LICENSE](https://raw.githubusercontent.com/versatica/mediasoup/v3/LICENSE) ·
[design](https://mediasoup.org/documentation/v3/mediasoup/design/) ·
[API](https://mediasoup.org/documentation/v3/mediasoup/api/) ·
[communication between client and server](https://mediasoup.org/documentation/v3/communication-between-client-and-server/) ·
[scalability](https://mediasoup.org/documentation/v3/scalability/) ·
[documentation index](https://mediasoup.org/documentation/v3/)

**Janus** — [COPYING](https://raw.githubusercontent.com/meetecho/janus-gateway/master/COPYING) ·
[README](https://raw.githubusercontent.com/meetecho/janus-gateway/master/README.md) ·
[VideoRoom](https://janus.conf.meetecho.com/docs/videoroom.html) ·
[AudioBridge](https://janus.conf.meetecho.com/docs/audiobridge.html)

**Jitsi** — [JVB LICENSE](https://raw.githubusercontent.com/jitsi/jitsi-videobridge/master/LICENSE) ·
[JVB README](https://raw.githubusercontent.com/jitsi/jitsi-videobridge/master/README.md) ·
[JVB doc/constraints.md](https://raw.githubusercontent.com/jitsi/jitsi-videobridge/master/doc/constraints.md) ·
[Jicofo README](https://raw.githubusercontent.com/jitsi/jicofo/master/README.md) ·
[Handbook: architecture](https://jitsi.github.io/handbook/docs/architecture/) ·
[Handbook: requirements](https://jitsi.github.io/handbook/docs/devops-guide/devops-guide-requirements/)

**Pion / Galène** — [pion/webrtc LICENSE](https://raw.githubusercontent.com/pion/webrtc/master/LICENSE) ·
[pion/webrtc README](https://raw.githubusercontent.com/pion/webrtc/master/README.md) ·
[pion/ion README](https://raw.githubusercontent.com/pion/ion/master/README.md) ·
`gh repo view pion/ion-sfu` / `gh api repos/pion/ion-sfu/commits` ·
[galene.org](https://galene.org/)

**Mumble** — [LICENSE](https://raw.githubusercontent.com/mumble-voip/mumble/master/LICENSE) ·
[README](https://raw.githubusercontent.com/mumble-voip/mumble/master/README.md) ·
[src/murmur/Server.cpp](https://raw.githubusercontent.com/mumble-voip/mumble/master/src/murmur/Server.cpp) ·
[src/ACL.h](https://raw.githubusercontent.com/mumble-voip/mumble/master/src/ACL.h) ·
[src/Mumble.proto](https://raw.githubusercontent.com/mumble-voip/mumble/master/src/Mumble.proto) ·
[auxiliary_files/mumble-server.ini](https://raw.githubusercontent.com/mumble-voip/mumble/master/auxiliary_files/mumble-server.ini) ·
[ACL documentation](https://www.mumble.info/documentation/administration/acl/) ·
[config file documentation](https://www.mumble.info/documentation/administration/config-file/) ·
[1.3.0 announcement](https://www.mumble.info/blog/mumble-1.3.0-release-announcement/) ·
[1.4.230 announcement](https://www.mumble.info/blog/mumble-1.4.230/)

**Other** — [Asterisk ConfBridge](https://docs.asterisk.org/Configuration/Applications/Conferencing-Applications/ConfBridge/) ·
[Ant Media ENTERPRISE_EDITION_LICENSE](https://raw.githubusercontent.com/ant-media/Ant-Media-Server/master/ENTERPRISE_EDITION_LICENSE) ·
[tauri-apps/wry#85](https://github.com/tauri-apps/wry/issues/85) ·
[tauri-apps/tauri#5042](https://github.com/tauri-apps/tauri/issues/5042) ·
[tauri-apps/tauri#11951](https://github.com/tauri-apps/tauri/issues/11951) ·
[tauri-apps/wry#1195](https://github.com/tauri-apps/wry/issues/1195)

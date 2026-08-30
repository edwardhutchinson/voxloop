# Recording taps plain Opus RTP per loop, on loopback

Recording is not a v1 feature, but the map requires the architecture leave a seam for it. That seam is a mediasoup `PlainTransport` per (talker, destination loop), created with `enable_srtp: false` and `rtcp_mux: true` and connected to a loopback address and port allocated per loop. A sink — a recorder, a transcriber, anything later — attaches by listening on that port. Nothing in the media path changes to accommodate it.

**The seam carries encoded Opus, not decoded PCM.** The map's constraint says "decoded audio", and LiveKit's per-track PCM egress was scored as the gold standard against it. But [#15](https://github.com/edwardhutchinson/voxloop/issues/15) sharpened what the constraint actually binds: *a new sink must attach without touching the media path or loading the media server*. Decoding inside the media server violates exactly that — it puts per-loop DSP load on the box, permanently, for a feature that is not in v1. Decoding is the sink's job, which is also how openvocs does it: the controller does not stream audio to the recorder, it tells the recorder which group to join.

**Transports are created when a sink attaches, not held open.** With no sink, the seam costs nothing at all.

**The bus is loopback-only, and this is a deliberate divergence from openvocs.** Their internal multicast bus is the cleanest decoded-audio seam of anything surveyed, and it is also unauthenticated and unencrypted: anyone who can send UDP to a loop's group can inject audio into that loop, and anyone who can join it can listen to every loop. On a single box that is contained to the host; on the multi-machine deployment their README invites, the internal network becomes the security boundary, and no document of theirs mentions it. VoxLoop takes the pattern and not the hole. Moving a sink off-box is then a deliberate decision with its own authentication and transport, rather than a configuration change that silently opens every loop to the network.

**Per-talker streams survive at the seam.** Each (talker, loop) transport carries that talker's RTP with a distinct SSRC, so a sink can record per loop, per talker, or both, and decides for itself. That is the property [#15](https://github.com/edwardhutchinson/voxloop/issues/15) identified as the one the recording constraint actually binds — separate from whether the *client* receives separate streams, which [ADR-0007](./0007-the-client-emits-one-stream.md) answers differently.

## Consequences

- **Adding a sink costs zero changes to the media path** and near-zero load until it attaches. Recording, transcription, keyword triggers and stream processing all consume the same seam, which is what "later work must not be foreclosed" required.
- **The sink owns decoding.** Any recorder or transcriber specification must budget Opus decode; it is not provided.
- **Moving a sink off the box is a new decision**, deliberately, and must not be reachable by editing an address in a config file.
- **The seam is per (talker, loop), so a recording of a loop is reconstructed from several streams**, not read off one. Mixing for playback, retention, format and the per-loop-versus-per-source split all remain unspecified.

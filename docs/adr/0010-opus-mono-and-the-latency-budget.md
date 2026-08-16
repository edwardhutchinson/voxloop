# Opus mono at 48 kHz, and the latency budget

Audio is **Opus, 48 kHz, mono, 20 ms frames, a ceiling around 32 kbps, with inband FEC and DTX both on**.

**Mono is the decision worth arguing.** openvocs runs stereo (`opus/48000/2`). But with client-side mixing ([ADR-0007](./0007-the-client-emits-one-stream.md)), **panning is a presentation choice the console makes over mono sources** — if operators benefit from hearing Flight in the left ear and Sim in the right, that is the console's job and it costs nothing on the wire. Stereo carriage buys nothing for speech and forecloses nothing anyone wants.

**20 ms frames rather than 10 ms**, because a 40–80 ms adaptive jitter buffer dominates the budget; halving the frame doubles packet rate for a saving lost in the noise. **Inband FEC** because remote users on the corporate VPN are the stated loss risk and the map says not to assume LAN conditions. **DTX** because the topology holds thousands of streams open ([ADR-0006](./0006-mediasoup-carries-the-audio.md)) and an idle one must cost almost nothing.

The budget:

| | Target |
|---|---|
| Mouth-to-ear, single-site LAN | **≤ 150 ms** |
| Mouth-to-ear, remote over VPN | **≤ 300 ms** |
| Fault threshold — the operator is *told*, not left to notice | **> 400 ms** |
| **Key to first audio** | **< 100 ms** |

Key-to-first-audio is the one that makes this feel like a radio rather than a conference call, and it is what [ADR-0008](./0008-emission-is-armed-by-the-server-and-keyed-by-the-client.md)'s client-side keying buys.

**Stated honestly: VoxLoop will be measurably laggier than the hardware it replaces.** Professional IP intercom runs 30–60 ms and mission control's circuit-switched loops are effectively instant, against a realistic 80–150 ms here (20 ms frame, ~1 ms of LAN, 40–80 ms of jitter buffer). This is a cost of being software on general networks, and it should be said to the pilot rather than discovered by them. The band structure is shaped by ITU-T G.114's ≤150 / 150–400 / >400 ms guidance, which is **cited from memory and is telephony-shaped rather than intercom-shaped** — verify before leaning on it in a customer-facing document.

These are targets to be measured, not properties that fall out of the design. mediasoup's `min_playout_delay` / `max_playout_delay` equivalents are the knob if the jitter buffer proves too conservative on a quiet LAN.

## Consequences

- **DTX and loop health interact badly, and this is the trap in this ADR.** With no packets during silence, a client cannot distinguish *"this loop is quiet"* from *"I am deaf to this loop"* by watching packet arrival — and those are indistinguishable to the ear too. Loop health must be derived from server-pushed application state (*this loop has N armed talkers, none keyed*) plus transport health, **never** from media presence. This is the silent failure this system can least afford.
- **Panning is available to the console for free**, and any later request for stereo carriage should be met by asking what the console cannot already do.
- **The budget needs measuring on the pilot's actual network**, including a VPN client, before v1 is called done. No source in any research pass addresses VPN latency for any candidate transport.
- **A fault threshold implies a fault display**, which is why *"transport is degraded"* is one of the four things the console must show.

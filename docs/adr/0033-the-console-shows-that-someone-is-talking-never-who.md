# The console shows that someone is talking, never who

A loop being spoken on renders as a single indicator on that loop, identical for every talker and for any number of them. The operator is not told which role is speaking, and never which person. Voice carries that, and [attribution](./0029-an-announcement-is-an-ordinary-transmission.md) carries it in the model for recording — but the console does not draw it.

The alternative — the talker's role beside the loop — was recommended and rejected. The reasoning that beat it is that a loop is supposed to relieve you of tracking individuals: it is staffed by people who can speak for it ([ADR-0005](./0005-occupancy-means-listening-not-signed-in.md)), so *the loop* is the identity, and asking which of its occupants is currently talking is asking a question the loop exists to make unnecessary. It is also the same argument that reduced the audience to a count in [ADR-0034](./0034-the-transmit-bar-is-always-visible-and-the-audience-is-a-count.md), applied to the receiving direction.

**The indicator may animate, and may never imply amplitude.** A bar or waveform that moves with the voice would be inventing a signal: DTX means silence sends no packets at all ([ADR-0010](./0010-opus-mono-and-the-latency-budget.md)), the indicator is server-pushed state rather than client-side audio analysis, and the console's standing rule is that what it shows is factual. So the glyph animates at one fixed rate and one fixed shape, reading unambiguously as on or off.

## Consequences

- **Attribution becomes a model-and-recording concept.** It is still carried by every transmission and still always the role, but v1 ships no surface where an operator reads it. Anything later that wants "who said that" — a transcript, a recording index, a talk history — starts from the model, not from something already on screen.
- **An operator monitoring several loops identifies talkers by voice alone.** Accepted deliberately. It is the pre-existing condition in every surveyed system's audio, and the console is not making it worse than the room already is.
- **Motion was permitted here and nowhere else.** The brief bans movement that pulls the eye off telemetry; one small fixed-rate glyph is the exception, and it is justified only because binary presence is genuinely hard to read from a static change at the edge of vision.

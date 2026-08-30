# The receiver is never told where else a transmission went

Alice keys, armed on Flight, Sim and Recovery. Bob monitors Flight. He hears her, and VoxLoop tells him nothing about the other two.

[ADR-0007](./0007-the-client-emits-one-stream.md) required the opposite. Its consequences say the console must attribute a transmission to every loop it is on, not just the one that won the volume, written while the console still attributed transmissions at all. [ADR-0033](./0033-the-console-shows-that-someone-is-talking-never-who.md) removed the thing that obligation hung on, leaving it stranded rather than satisfied. This is the decision it was waiting for, and it goes the other way.

**The hazard is real.** Bob either relays to Sim redundantly, which costs a sentence, or assumes Sim heard it and stays quiet, which is the failure. Multi-destination emission has no prior-art precedent to borrow from: one radio cannot transmit on two nets at once, NASA enforces a single talk loop, and [#15](https://github.com/edwardhutchinson/voxloop/issues/15) found openvocs permits multi-destination emission while supplying no compensation for it. Whatever VoxLoop does here it invents.

**Every candidate signal is worse than the gap.** Naming the other loops leaks loops outside Bob's reach, against [ADR-0019](./0019-presence-is-one-versioned-document-scoped-to-reach.md). A count scoped to Bob's reach reads as complete when it is not, which breaks [ADR-0016](./0016-displayed-state-is-observed-or-asserted.md)'s factual-state rule by omission rather than by error, and omission is the harder kind to notice. A bare "also elsewhere" marker gives him nothing to act on. All of them have to fit a board card as a word ([ADR-0032](./0032-the-console-is-two-views-of-one-loop-list.md)), competing with staffing state for space that is already tight.

**What settles it is that Bob has no action the gap denies him.** Every reply is addressed to a loop ([ADR-0001](./0001-the-loop-is-the-only-destination.md)), so Alice transmitting wide does not change where Bob's answer goes, and there is no privacy asymmetry to correct. "Did Sim copy?" is one second of voice on a system built to carry exactly that.

## Consequences

- **ADR-0007's display obligation is dead rather than merely unbuildable.** It should not be revived as a fix when somebody notices the gap; the reasoning above is the answer.
- **Reach is invisible to the receiver, gain and urgency are not.** [ADR-0059](./0059-a-priority-transmission-is-marked-wherever-it-lands.md) marks priority on every loop it reaches, and the line between the two is the rule: a receiver is told what changes what they hear, never who else is hearing. A later proposal to surface a transmission's reach has to break that line, not extend it.
- **The recording is the only place a transmission's full reach is ever visible**, alongside being the only place attribution lands ([ADR-0033](./0033-the-console-shows-that-someone-is-talking-never-who.md)). Anything anyone ever wants to ask about who addressed what has to be answerable from the recorder or not at all.
- **The compensation for multi-destination emission is entirely on the emitting side.** [ADR-0058](./0058-the-transmit-bar-is-live-while-keyed.md) is where it lives, which makes the emitter solely responsible for knowing their own reach.

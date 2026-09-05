// The reading every source publishes into, and the OR over the live ones.
//
// **A level and a liveness flag, never events** (ADR-0021). Edges are lossy and their loss
// mode is an open mic: one dropped release and somebody is transmitting indefinitely with no
// later event to correct it. A level is self-correcting, because the next reading says what is
// true now.
//
// **Liveness is the part events cannot express at all.** *The headset was unplugged while you
// were holding the button* is a property of the source rather than of anything the source
// could have sent — by the time it is true the source is gone and can send nothing. Two things
// fall out of it and both are here: a source that is not live is not in the OR whatever it
// last published, so **a source that dies while keyed forces an unkey**; and the console can
// say *why* keying is unavailable rather than drawing a control that does nothing (ADR-0016).
//
// It is separate from `index.js` because this is the part with the rule in it and that is the
// part with the sources in it. Both are inside the seam and neither is reachable from outside
// it (ADR-0061).

/**
 * Start reading intent, and answer with the way a source registers.
 *
 * `onIntent(wants)` is called when the answer **changes** and not on every reading: it is a
 * level being sampled, and a caller told the same thing five times a second would be a caller
 * that had to remember what it was last told in order to act on it.
 */
export function levels({ onIntent }) {
	const sources = new Map();
	let told = false;

	function settle() {
		const wants = [...sources.values()].some(({ level, live }) => live && level);
		if (wants === told) return;

		told = wants;
		onIntent(wants);
	}

	return {
		/**
		 * Register a source, and answer with the two calls a source makes.
		 *
		 * `publish` is the whole interface: this is what I want, and this is whether I am
		 * here. `gone` is the source leaving, said as its own act because a source that has
		 * gone has nothing left to publish with.
		 */
		add(named) {
			sources.set(named, { level: false, live: false });

			return {
				publish: (level, live) => {
					sources.set(named, { level, live });
					settle();
				},
				gone: () => {
					sources.delete(named);
					settle();
				}
			};
		},

		/** Which sources are live right now, by name. */
		live: () => [...sources].filter(([, source]) => source.live).map(([named]) => named)
	};
}

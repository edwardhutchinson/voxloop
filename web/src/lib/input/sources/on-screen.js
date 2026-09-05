// The on-screen key control: the first source, and the only one until #42.
//
// It is here rather than in the component that draws it because **the seam is the thing
// being built** (ADR-0021), and a source that lived in a component would be a source the
// component could reach past the seam to read. What the component has is a button and two
// pointer events; what this has is a level and a liveness flag, which is what every source
// publishes and the only thing Input knows how to read.
//
// **It is always live.** An on-screen control cannot be unplugged: it is present exactly
// while it is on screen, and it says so when it goes. That is what makes it the honest first
// source rather than the trivial one — a keyboard binding (#42) is live only while the tab
// has focus and a native hotkey is live only while the wrapper is there, and the console can
// already tell the three apart because all three answer the same question.
//
// **It does not know which emission mode it serves** and there is nothing here that could
// find out. Momentary and latch are #42's, above the seam; this reports intent.

/** Name it goes by where the console has to say which source it is talking about. */
export const ON_SCREEN = 'the key control';

/**
 * Register the on-screen control with Input, and answer the three things a control does.
 *
 * `down` and `up` are pointer states rather than edges — the level is set from what the
 * pointer is doing now, so a `pointerup` that never arrives because the pointer left the
 * button is corrected by the next thing that happens rather than leaving an open mic.
 */
export function onScreen(input) {
	const publishing = input.add(ON_SCREEN);
	publishing.publish(false, true);

	return {
		down: () => publishing.publish(true, true),
		up: () => publishing.publish(false, true),
		/** The control has left the screen, so the source it was has gone with it. */
		gone: () => publishing.gone()
	};
}

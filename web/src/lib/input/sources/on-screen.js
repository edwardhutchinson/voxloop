// The on-screen key control: the first source, and the only one until #42.
//
// It is here rather than in the component that draws it because **the seam is the thing
// being built** (ADR-0021), and a source that lived in a component would be a source the
// component could reach past the seam to read. What the component has is a button and two
// pointer events; what this has is a level and a liveness flag, which is what every source
// publishes and the only thing Input knows how to read.
//
// **It is live exactly while its control is on screen**, which is the honest reading of an
// on-screen control's liveness and the reason this ticket needed the flag rather than merely
// reserving it. Emission is withdrawn when the audio path goes (ADR-0042), so the control
// goes with it — and a control that vanished under a held pointer delivers no release. Under
// an event-shaped seam that is a hung transmission; here the source stops being live, leaves
// the OR, and **the key drops**. That is [ADR-0021]'s forced unkey, arriving in the first
// case that can produce one.
//
// **It does not know which emission mode it serves** and there is nothing here that could
// find out. Momentary and latch are #42's, above the seam; this reports intent.
//
// [ADR-0021]: ../../../../docs/adr/0021-ptt-input-is-a-level-with-liveness.md

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
	let held = false;
	let present = false;

	const say = () => publishing.publish(held, present);

	return {
		down: () => {
			held = true;
			say();
		},
		up: () => {
			held = false;
			say();
		},
		/**
		 * Whether the control is on screen at all.
		 *
		 * Going away **drops whatever was held**, rather than leaving it to be picked up if
		 * the control comes back: a key an operator's hand has left is one that must not
		 * return on its own, which is the whole class of surprise this seam exists to
		 * prevent.
		 */
		present: (is) => {
			present = is;
			if (!is) held = false;
			say();
		}
	};
}

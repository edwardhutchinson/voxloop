// The ordered four, as the console reads them: `none`, `monitor`, `emit`, `control`, each
// rung carrying everything below it (ADR-0011).
//
// It is here rather than in either view because **both views ask the same question of a
// rung** and a second implementation is how they come to disagree — the same reason the loop
// order and the toggle decision are held above them. The grid is the only thing entitled to
// say what a role holds; this only reads the answer.
//
// A rung the console has no name for carries nothing. That is the safe direction and the
// honest one: it is a rung this build predates, and offering a control on the strength of a
// word nobody here recognises would be the console asserting a permission it was never given.

const ladder = ['none', 'monitor', 'emit', 'control'];

/** Whether a permission carries a rung, which is the whole of what the order is for. */
export function carries(permission, rung) {
	const held = ladder.indexOf(permission);

	return held !== -1 && held >= ladder.indexOf(rung);
}

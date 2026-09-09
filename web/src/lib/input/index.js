// Input — the seam every way of keying arrives through, and the client's one enforced one.
//
// **Every source publishes a level and a liveness flag, never events** (ADR-0021), and the
// client ORs the live ones. The rule is in `level.js` and the sources are in `sources/`; this
// is the interface, and it is the only thing anything above may name.
//
// **A source never knows which emission mode it serves.** Momentary and latch are decided
// above this line, and a source that decided its own could latch by accident (ADR-0022).
// v1 has one source, the on-screen key control; #42 adds the keyboard bindings and the modes,
// and the Tauri wrapper adds a native hotkey and changes nothing else (ADR-0020). That promise
// is the reason this is the one client seam with a failing build behind it (ADR-0061):
// `$lib/input` is the way in, and `eslint.input-seam.js` refuses everything underneath it.

import { levels } from './level.js';
import { onScreen } from './sources/on-screen.js';

/**
 * Start reading intent, and answer with what the console has to work with.
 *
 * **Registering a source is not on the answer**, deliberately: a source is a file in
 * `sources/` that this composes, so the console cannot invent one and the wrapper's promise —
 * *it may only ever add a source* — stays a claim about this file rather than about whatever
 * the console happened to register.
 */
export function keying({ onIntent }) {
	const reading = levels({ onIntent });

	return {
		/** The on-screen key control. #42 puts the keyboard bindings beside it. */
		onScreen: onScreen(reading)
	};
}

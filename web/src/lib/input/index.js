// Input — the seam every way of keying arrives through, and the client's one enforced one.
//
// **Every source publishes a level and a liveness flag, never events** (ADR-0021), and the
// client ORs the live ones. The rule is in `level.js`, the sources are in `sources/` and what
// may be bound is in `bindings.js`; this is the interface, and it is the only thing anything
// above may name.
//
// **A source never knows which emission mode it serves.** Momentary and latch are decided
// above this line, and a source that decided its own could latch by accident (ADR-0022). So
// **the names of the modes come from above too** — this is handed a binding per name, reads
// each of them separately, and reports each under the name it was given. There is no word in
// this directory for what any of them means, which is what makes that claim checkable rather
// than a habit.
//
// Adding a way to key is adding a file to `sources/` and a line to the loop below. The Tauri
// wrapper adds a native hotkey and changes nothing else (ADR-0020), and that promise is the
// reason this is the one client seam with a failing build behind it (ADR-0061): `$lib/input`
// is the way in, and `eslint.input-seam.js` refuses everything underneath it.

import { describe, fromEvent, refusal, stillReaching, theSame } from './bindings.js';
import { levels } from './level.js';
import { keyboard } from './sources/keyboard.js';
import { onScreen } from './sources/on-screen.js';

export { describe, fromEvent, refusal, stillReaching };

/**
 * Start reading intent, and answer with what the console has to work with.
 *
 * `bindings` is a key per name — the names are the caller's, and this attaches every source
 * it has to each of them. `onIntent(name, wants)` and `onDropped(name, source)` answer under
 * those same names.
 *
 * `on` is what key events arrive on, the window in a browser. It is a parameter rather than a
 * global so that nothing in here reaches for one: the console is rendered on the server at
 * build time, and a seam that touched a window would take the build down with it.
 *
 * **Registering a source is not on the answer**, deliberately: a source is a file in
 * `sources/` that this composes, so the console cannot invent one and the wrapper's promise —
 * *it may only ever add a source* — stays a claim about this file rather than about whatever
 * the console happened to register.
 */
export function keying({ bindings, onIntent, onDropped = () => {}, on = globalThis }) {
	const bound = { ...bindings };
	const onScreens = {};
	const keyboards = {};

	for (const named of Object.keys(bound)) {
		// One reading per name, so the OR is over the sources of *that* key. A single reading
		// with every source in it would be one level for two keys, which is the one thing
		// separate bindings may not collapse into (ADR-0022).
		const reading = levels({
			onIntent: (wants) => onIntent(named, wants),
			onDropped: (source) => onDropped(named, source)
		});

		onScreens[named] = onScreen(reading);
		keyboards[named] = keyboard(reading, { on });
		keyboards[named].bind(bound[named]);
	}

	// A key already bound to something else. Two names on one key is one press meaning two
	// things, and with the modes above that is a press that both opens and closes — which is
	// the derived latch ADR-0022 refuses, arriving by the back door of a rebind.
	const clash = (named, binding) =>
		Object.entries(bound).some(([other, was]) => other !== named && theSame(was, binding));

	const each = (act) => {
		for (const named of Object.keys(bound)) act(named);
	};

	return {
		/** The on-screen control for each name: what a button on the transmit bar publishes. */
		onScreen: onScreens,

		/** The keys as they currently stand, for a console that has to show them. */
		bound: () => ({ ...bound }),

		/**
		 * Whether keying stands at all: an assumed role, and an audio path to key over.
		 *
		 * One answer for every source, because it is one fact about the console rather than
		 * about any of them. What each source does with it is its own — the control is on
		 * screen or it is not, and the keyboard is listening for a key or it is not.
		 */
		available: (is) =>
			each((named) => {
				onScreens[named].present(is);
				keyboards[named].available(is);
			}),

		/**
		 * Bind a name to a key, or answer why not.
		 *
		 * The refusals are the seam's rather than the console's, because they are facts about
		 * how a key behaves (ADR-0022) and about what this table already holds. A console
		 * that wrote its own copy of them would be a console that could get them wrong.
		 */
		rebind: (named, binding) => {
			const no =
				refusal(binding) ??
				(clash(named, binding)
					? `${describe(binding)} is already in use. One key does one thing.`
					: null);
			if (no) return no;

			bound[named] = binding;
			keyboards[named].bind(binding);

			return null;
		},

		/**
		 * Everything this attached, released — and **every source publishes that it has gone**
		 * on the way out.
		 *
		 * A console torn down under a held key is a source dying while keyed, which is the
		 * one thing this seam may not let pass in silence (ADR-0021): without the publish the
		 * key would simply stop being watched, and the microphone would still be live on a
		 * page nobody is looking at.
		 */
		stop: () =>
			each((named) => {
				onScreens[named].present(false);
				keyboards[named].stop();
			})
	};
}

// The emission modes: **momentary** and **latched**, and no third (v1 §4).
//
// This is the whole of what sits above the Input seam, and it is above it on purpose. A
// source publishes a level and a liveness flag and nothing else (ADR-0021); what a level
// *means* — talk while I hold this, or talk until I say stop — is decided here, where there
// is no key and no device to confuse it with. Nothing under `input/` can name either mode,
// and `tests/input.test.js` fails the build if that stops being true.
//
// **Latch has its own binding and is never derived from a momentary press** (ADR-0022) — not
// by a short tap, not by a double tap, not by a held duration. So there is nothing here that
// times a press or counts one: there are two readings arriving under two names, and the only
// thing this file does with the second is flip a switch on its rising edge. Deriving it would
// make an open mic the failure mode of a hardware fault, because a button that stopped
// reporting its release would be indistinguishable from a deliberate tap.
//
// **A single-button device is therefore momentary-only**, which is a consequence rather than
// a limitation to work around: a footswitch and a headset button reach the key they are bound
// to and nothing reaches latch except the key bound to latch and the button on the bar.
//
// It is a module rather than something the console does inline because it is the piece with
// the failure modes in it, and a piece with failure modes wants to be run without a browser.
//
// The seam is imported by the path rather than as `$lib/input` because this is a plain module
// that Node loads directly in a test, where SvelteKit's alias is not a thing that exists. It
// is the same interface either way and the lint rule reads both spellings (ADR-0061).
import { keying } from './input/index.js';

/** The two modes, by the names they are known by here and nowhere below the seam. */
export const MOMENTARY = 'momentary';
export const LATCHED = 'latched';

/**
 * The keys they start on (ADR-0022), and what the console calls each of them.
 *
 * Called by the words the domain uses — *momentary* and *latched* — rather than by a friendlier
 * paraphrase, because they are the words the spec, the audit log and the operator's own
 * manual are written in, and a console that renamed them would be the one place they differ.
 *
 * `` ` `` has no browser binding, activates no control and conflicts only while typing. Space
 * is refused because it activates focused controls, scrolls and types, and `CapsLock` because
 * its release is unreliable on macOS — an unreliable release on a key you talk with is an
 * open mic. Both refusals are the seam's and are enforced on every rebind.
 */
export const modes = [
	{
		named: MOMENTARY,
		binding: { code: 'Backquote' },
		called: 'Momentary',
		means: 'Hold this key to talk, and let go to stop.'
	},
	{
		named: LATCHED,
		binding: { code: 'Backquote', shift: true },
		called: 'Latched',
		means: 'Press this key to start talking, and press it again to stop.'
	}
];

const defaults = Object.fromEntries(modes.map(({ named, binding }) => [named, binding]));

/**
 * Read the modes over Input, and answer with what a console does with them.
 *
 * - `onKeying(wants)` — whether this session is keying. It is the OR of the two modes and it
 *   is said when it changes, which is what the console sends down to Audio and to the server.
 * - `onLatched(is)` — whether the key is latched open. A fact about this console's own input
 *   rather than about the world, and the console has to render it as one (ADR-0016).
 * - `onDropped(source)` — that source went while the key was held, and the key went with it.
 */
export function keyingModes({ onKeying, onLatched, onDropped = () => {}, on }) {
	let held = false;
	let latched = false;
	let emitting = false;

	function settle() {
		const wants = held || latched;
		if (wants === emitting) return;

		emitting = wants;
		onKeying(wants);
	}

	function latch(is) {
		if (is === latched) return;

		latched = is;
		onLatched(is);
	}

	const input = keying({
		bindings: defaults,
		on,
		onIntent(named, wants) {
			if (named === MOMENTARY) {
				held = wants;
				settle();
				return;
			}

			// **The rising edge and nothing else.** The latch key is a level like every other
			// source, and what is read off it is the moment it goes up: a key held down does
			// not latch and unlatch while it is held, and a release that never arrives —
			// which is the failure the level exists for — leaves the latch exactly where the
			// press put it rather than toggling it a second time.
			if (!wants) return;

			latch(!latched);
			settle();
		},
		onDropped(named, source) {
			// Said only of the mode whose level *is* the emission, and only when nothing is
			// emitting any more. A latch button dying under a finger has taken nothing off
			// the air — the latch is this console's state and not that button's — and telling
			// an operator their key dropped when it did not is the same lie as the reverse.
			if (named !== MOMENTARY || emitting) return;

			onDropped(source);
		}
	});

	return {
		/** The on-screen control for each mode: what the buttons on the transmit bar publish. */
		onScreen: input.onScreen,

		/** The keys as they currently stand, for the console that has to show them. */
		bound: input.bound,

		/** Bind a mode to a key, or answer why not. */
		rebind: input.rebind,

		/**
		 * Whether keying stands at all: an assumed role, and an audio path to key over.
		 *
		 * **Withdrawal drops the latch.** A latched transmission is the console holding the
		 * key open, and the console holding it open across an outage is precisely the hot mic
		 * with a randomly-timed start that v1 §7 refuses — key state never returns, and this
		 * is the end of the outage where it is decided.
		 */
		available: (is) => {
			if (!is) latch(false);
			input.available(is);
			settle();
		},

		/**
		 * Everything this attached, released, and **the key stops with it**.
		 *
		 * A console going is keying stopping, whatever was holding it open: a role given up
		 * under a held key, or under a latch, has to reach Audio and the server as an unkey
		 * rather than as listeners quietly going away (ADR-0021).
		 */
		stop: () => {
			latch(false);
			input.stop();
			settle();
		}
	};
}

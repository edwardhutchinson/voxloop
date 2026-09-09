// The keyboard: the source almost everybody keys with, and the one a footswitch arrives as.
//
// **A keystroke footswitch works here for nothing** — it is a keyboard, it sends a keystroke,
// and nothing below this line can tell it from the one on the desk. That is the whole of v1's
// peripheral support and it is why it needed no code (v1 §4).
//
// It is one source per binding rather than one source reading a table. A source publishes one
// level, so a source serving two bindings would have to decide what its level meant when both
// were down — which is a decision about emission, and this file is on the wrong side of the
// seam to be making one (ADR-0021). Two bindings are two sources, each publishing about its
// own key, and what the two of them add up to is settled above.
//
// **It does not know which emission mode it serves.** It is handed a binding, it says whether
// that key is down, and there is nothing here that could find out what happens next.
//
// Three refusals, and each is one of ADR-0021's or ADR-0022's failures arriving in the one
// place that can produce it:
//
// - **Not while focus is in a text field or on an interactive control** (ADR-0022). The
//   refusal is on the press alone. A release is always taken, because focus can move under a
//   held key and a release nobody accepted is an open mic.
// - **Autorepeat may not raise a level that is low.** A key still physically held when the
//   window comes back delivers repeats and no fresh press, so a level raised by one is a
//   transmission starting at a moment nobody chose — v1 §7's rule for a key held across an
//   outage, arriving from the one case in the browser that can produce it.
// - **A window that loses focus drops the level** (ADR-0021). Holding a key and switching to
//   another application never delivers the release, and under an event-shaped seam that is a
//   hung transmission.

import { presses, releases } from '../bindings.js';

/** Name it goes by where the console has to say which source it is talking about. */
export const KEYBOARD = 'the keyboard';

/** Where a press is refused: a text field, or a control a key might operate (ADR-0022). */
const INTERACTIVE = 'input, textarea, select, button, a[href], [contenteditable], [tabindex]';

// Asked of the event's target rather than of `document.activeElement`, because they are the
// same element for a key event and only one of them is reachable from a test. Anything that
// cannot answer is read as *not a control*: the page body is where an operator's hands rest,
// and reading it as interactive would make the keyboard inert everywhere.
function guarded(node) {
	if (!node || typeof node.closest !== 'function') return false;
	return node.isContentEditable === true || node.closest(INTERACTIVE) !== null;
}

/**
 * Register a keyboard binding with Input, and start listening.
 *
 * `on` is what carries the key events — the window, in a browser. It is passed rather than
 * reached for so that this file has no global in it: the console is also rendered on the
 * server at build time, where there is nothing to listen on, and a source with no events
 * reaching it is a source that is **not live** rather than one that quietly keys nothing.
 */
export function keyboard(input, { on }) {
	const publishing = input.add(KEYBOARD);
	const listening = typeof on?.addEventListener === 'function';

	let binding = null;
	let down = false;
	let allowed = false;

	// Live means *this can actually report a key*: something to listen on, a key to listen
	// for, and a console that is in a position to key at all. Anything less and the console
	// may not draw this as a way to talk (ADR-0016).
	const say = () => publishing.publish(down, listening && allowed && binding !== null);

	function pressed(event) {
		if (!binding || !presses(event, binding)) return;
		if (guarded(event.target)) return;
		if (event.repeat && !down) return;

		// The key is being talked with rather than typed with, so it does not also do
		// whatever the browser would have done with it.
		event.preventDefault?.();
		down = true;
		say();
	}

	function released(event) {
		if (!binding || !releases(event, binding)) return;

		down = false;
		say();
	}

	function away() {
		if (!down) return;

		down = false;
		say();
	}

	if (listening) {
		on.addEventListener('keydown', pressed);
		on.addEventListener('keyup', released);
		on.addEventListener('blur', away);
	}

	return {
		/** The key this source is listening for. Rebinding is a change of key, not a restart. */
		bind: (to) => {
			// Whatever was held was held on the old key, and there is no release coming for
			// it: the operator is at a console changing a setting, not talking.
			binding = to;
			down = false;
			say();
		},
		/**
		 * Whether keying stands at all — an assumed role, and an audio path to key over.
		 *
		 * Going away **publishes what was held**, so a key still down when it went is a
		 * source dying with the key held, which is what forces the unkey and says so
		 * (ADR-0021). Coming back drops it, because a key an operator's hand has left must
		 * not return on its own — and if their hand has not left it, autorepeat is refused
		 * above until they let go.
		 */
		available: (is) => {
			allowed = is;
			if (is) down = false;
			say();
		},
		/** The listeners go when the console does. A role given up is not a role you can key. */
		stop: () => {
			if (!listening) return;

			on.removeEventListener('keydown', pressed);
			on.removeEventListener('keyup', released);
			on.removeEventListener('blur', away);
		}
	};
}

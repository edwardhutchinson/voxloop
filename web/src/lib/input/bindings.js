// What a binding is, which keys may be bound, and how a press is matched against one.
//
// A binding is a key **by its place on the keyboard** together with the modifiers that must
// be down with it. The place rather than the letter, because a modifier changes the letter
// and not the key: `` ` `` with shift held reports itself as `~`, so a table written in
// letters would hold two unrelated entries for one physical key and a release would match
// neither. The place is also what a footswitch has — it sends a keystroke and nothing knows
// which — so this is the reading that covers both.
//
// **A press is matched exactly and a release is matched on the key alone.** An exact press
// is what keeps two bindings on one key apart, which is the whole of ADR-0022's separate
// bindings. A loose release is the safe direction: an operator who presses a modifier while
// already holding the key would otherwise deliver a release that matched nothing, and a
// release that matches nothing is an open mic.
//
// **Two keys are refused outright** and the refusals are here rather than in the console,
// because they are facts about how a key behaves rather than about how the console looks.

/**
 * The keys nothing may be bound to, and what to tell somebody who tried (ADR-0022).
 *
 * Both are refused for the same reason in the end: a key that does something else while you
 * are talking, or that does not reliably say when it was let go.
 */
const never = new Map([
	[
		'Space',
		'Space activates whatever control has focus, it scrolls the page, and it types. Choose another key.'
	],
	[
		'CapsLock',
		'Caps Lock does not reliably report being released, and a release that never arrives leaves your microphone open. Choose another key.'
	]
]);

// A modifier is derived state and derived state loses releases (ADR-0022), so a modifier on
// its own is not a key to talk with — it is the thing held beside one.
const aModifier = /^(?:Shift|Control|Alt|Meta)(?:Left|Right)$/;

/** How the modifiers are read off a key event, and written into a binding. */
const held = (event) => ({
	shift: event.shiftKey === true,
	ctrl: event.ctrlKey === true,
	alt: event.altKey === true,
	meta: event.metaKey === true
});

// A binding may leave a modifier out, so every read of one goes through this rather than
// through the field: `{ code: 'Backquote' }` and a binding with four explicit `false`s are
// the same binding, and a comparison that did not say so would refuse a rebind to a key that
// was already bound to nothing.
const wanted = (binding) => ({
	shift: binding.shift === true,
	ctrl: binding.ctrl === true,
	alt: binding.alt === true,
	meta: binding.meta === true
});

const sameModifiers = (one, other) => {
	const [a, b] = [wanted(one), wanted(other)];
	return a.shift === b.shift && a.ctrl === b.ctrl && a.alt === b.alt && a.meta === b.meta;
};

/** Whether two bindings are the same key with the same modifiers. */
export const theSame = (one, other) => one.code === other.code && sameModifiers(one, other);

/** Whether this press is this binding: the key, and exactly these modifiers and no others. */
export const presses = (event, binding) =>
	event.code === binding.code && sameModifiers(held(event), binding);

/** Whether this release is this binding's key. Modifiers are not asked about — see above. */
export const releases = (event, binding) => event.code === binding.code;

/** The binding somebody just pressed, for a console offering to rebind. */
export const fromEvent = (event) => ({ code: event.code, ...held(event) });

/**
 * Whether this press is a modifier and nothing else.
 *
 * Somebody reaching for a modified binding presses the modifier first, and a console reading
 * that as the binding would refuse them before they got to the key. So it is a *not yet*
 * rather than a refusal, and it is here because what counts as a modifier is this file's.
 */
export const stillReaching = (event) => aModifier.test(event.code);

/**
 * Why this may not be bound, or `null` if it may.
 *
 * A sentence rather than a flag, because the console has nothing else to say to somebody who
 * pressed Space: *refused* on its own reads as the console being broken.
 */
export function refusal(binding) {
	if (!binding?.code) return 'That is not a key VoxLoop can read.';
	if (never.has(binding.code)) return never.get(binding.code);
	if (aModifier.test(binding.code))
		return 'A modifier on its own is not a key to talk with. Hold it and press another key.';

	return null;
}

// The keys whose place has a name that is not its code. Everything else either writes itself
// (`KeyA`, `Digit1`, `F5`, `ArrowLeft`) or is rare enough that the code is the clearest thing
// to show — a binding nobody can name is one nobody can check.
const written = new Map([
	['Backquote', '`'],
	['Minus', '-'],
	['Equal', '='],
	['BracketLeft', '['],
	['BracketRight', ']'],
	['Backslash', '\\'],
	['Semicolon', ';'],
	['Quote', "'"],
	['Comma', ','],
	['Period', '.'],
	['Slash', '/'],
	['Escape', 'Esc'],
	['Enter', 'Enter'],
	['Tab', 'Tab'],
	['Backspace', 'Backspace']
]);

const theKey = (code) =>
	written.get(code) ??
	code
		.replace(/^Key/, '')
		.replace(/^Digit/, '')
		.replace(/^Numpad/, 'Numpad ');

/** The binding as somebody would say it out loud: `Shift + \`` . */
export function describe(binding) {
	if (!binding?.code) return 'nothing';

	const on = wanted(binding);
	// Written in the order a hand takes them, which is also the order every keyboard shortcut
	// anybody has ever read is written in.
	const parts = [
		...(on.ctrl ? ['Ctrl'] : []),
		...(on.alt ? ['Alt'] : []),
		...(on.shift ? ['Shift'] : []),
		...(on.meta ? ['Meta'] : []),
		theKey(binding.code)
	];

	return parts.join(' + ');
}

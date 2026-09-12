// Staffing state, as the console says it: a word on the board and a sentence in the ledger
// and the lobby (v1 §8, ADR-0032).
//
// It is here rather than in either view for the reason `rungs.js` is: **both views say the
// same thing about the same field**, and a second implementation is how they come to
// disagree. The lobby is the third reader — it carries the reason in full, ledger-style,
// because it is read once and deliberately by somebody about to take the seat — and it gets
// the same sentence from the same place rather than one of its own.
//
// **A loop with no staffing roles has no staffing state** (ADR-0056). That arrives as `null`
// and leaves here as an empty string: blank where the word goes, never a fourth word. `n/a`
// and `unstaffed` both read as states, and this is the absence of one.
//
// A state or a reason this console has no name for is said as the word the document used.
// The server is the only thing entitled to judge a loop staffed, and rendering nothing would
// be the console dropping a fact it was given.

/** What each state is called on a card. */
const words = {
	staffed: 'Staffed',
	away: 'Away',
	vacant: 'Vacant'
};

/**
 * Each reason as it reads when it is the only one, which is the plain sentence v1 §8 gives:
 * `away — muted it`, and not `away — 1 muted` when the one occupant is the whole story.
 */
const alone = {
	unreachable: 'unreachable',
	'off-console': 'off console',
	'not-subscribed': 'not subscribed to it',
	'not-receiving': 'not receiving it',
	muted: 'muted it'
};

/** Each reason as it reads with a count in front of it: `1 muted, 2 not subscribed`. */
const counted = {
	unreachable: 'unreachable',
	'off-console': 'off console',
	'not-subscribed': 'not subscribed',
	'not-receiving': 'not receiving it',
	muted: 'muted'
};

/** The word the board carries, or nothing where the loop has no staffing roles. */
export function theWord(staffing) {
	if (!staffing) return '';

	return words[staffing.state] ?? staffing.state;
}

/**
 * The sentence the ledger and the lobby carry, or nothing where the loop has no staffing
 * roles.
 *
 * **The reason is a count over occupants and it ranks nothing** (ADR-0065): they are read
 * out in the order the document sends them, which is the order a reason is chosen in within
 * one occupant, and that is an order to read in rather than a precedence. Where they all
 * agree it collapses to the plain sentence, because *1 muted* is a count nobody needed.
 */
export function theSentence(staffing) {
	if (!staffing) return '';

	if (staffing.state === 'staffed') {
		return 'Staffed — an occupant of a role that staffs this loop is hearing it.';
	}
	if (staffing.state === 'vacant') {
		return 'Vacant — nobody occupies a role that staffs this loop.';
	}
	if (staffing.state !== 'away') return `${theWord(staffing)}.`;

	const away = staffing.away ?? [];
	if (away.length === 1) {
		return `Away — ${alone[away[0].reason] ?? away[0].reason}.`;
	}

	const each = away.map(({ reason, occupants }) => `${occupants} ${counted[reason] ?? reason}`);

	// An `away` with nothing behind it cannot be sent — the server only reaches that state by
	// counting somebody — and saying the bare word is the honest answer if one ever arrives.
	return each.length ? `Away — ${each.join(', ')}.` : 'Away.';
}

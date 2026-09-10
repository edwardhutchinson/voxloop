// Connection state: this tab's own standing with the signalling channel, and the one clock
// the console runs for itself.
//
// **A session with no signalling channel has no emission path** (ADR-0018). Every talking
// indicator anyone sees is a server broadcast, so an operator keying with no channel
// transmits into a system where nobody's console shows them, no loop attributes it and no
// authority holder can cut it — the audio arrives and the accountability does not. The server
// closes the fan-out at the same threshold; this is the client's half, and the two halves are
// both needed because the situation that motivates the rule is precisely the one where the
// client may be wedged.
//
// **It is measured here rather than pushed** — and it is the one state in the console that
// is. Everything else on screen is the server's answer (ADR-0016), because the server is the
// only thing entitled to say what is true of the world. This is a fact about the channel
// itself, and the one thing a server cannot do to a console it has lost is tell it that it
// has been lost. So the console counts the heartbeats it is not getting.
//
// **The clock is the deployment's, not this file's.** The four timers are startup settings
// with a hard ceiling (v1 §7) and they arrive on every heartbeat, so a console that missed
// the first one still runs the right ladder. Until one arrives this runs on the numbers v1
// fixes, which is what the server would have said anyway.
//
// **The rungs are not the same shape at both ends.** A socket that closed is a *fact* rather
// than a silence, so it is `disconnected` at once with no ladder to wait out; the rungs are
// for the case nobody reported, which is the wedged client and the flapping VPN.
//
// Nothing here renders anything and nothing here touches Input. It answers with where the
// channel stands, how long ago it was last confirmed, and whether a latched emission may
// still stand — and `Console.svelte` is what does something about each of those.

/** Heartbeats current: everything normal. */
export const CONFIRMED = 'confirmed';
/**
 * Heartbeats missed. What is on screen is frozen and is marked stale with a running age, and
 * **push-to-talk stays live**: *we cannot confirm your transmission right now* is a
 * materially different statement from *we know you are disconnected*, and cutting somebody
 * off mid-word for a half-second blip is the failure this rung exists to prevent.
 */
export const UNCONFIRMED = 'unconfirmed';
/** Past the threshold. Emission is withdrawn here and at the server both. */
export const DISCONNECTED = 'disconnected';

/**
 * The ladder v1 §7 fixes, in the shape a heartbeat carries it.
 *
 * It is the fallback rather than the rule: a deployment tunes its own against its own VPN,
 * and what it tuned arrives on the wire. A console with no heartbeat yet has to run
 * *something*, and running the spec's numbers is the only choice that is right on every
 * deployment that has not said otherwise.
 */
export const THE_LADDER_V1_FIXES = {
	heartbeat_ms: 2000,
	unconfirmed_ms: 5000,
	latch_dropped_ms: 2000,
	disconnected_ms: 12000
};

/**
 * How often the ladder is read again.
 *
 * Fine enough that a rung is reached within half a second of being true, and that the running
 * age moves every second without stuttering. It is not the heartbeat: one is how often the
 * channel is proved and the other is how often this console looks at its watch.
 */
export const SETTLES_EVERY = 500;

/**
 * Start reading where this tab stands with the signalling channel.
 *
 * `onConnection({ state, since, aLatchStands })` is called when the answer changes and not on
 * every reading — `since` moves every second while the state is not confirmed, because it is
 * the running age the console shows, and it moves nothing while it is.
 *
 * `aLatchStands` is the answer to *may this console still hold a key open for somebody*. It
 * goes false a couple of seconds into `unconfirmed`, which is well before emission itself is
 * withdrawn: a latch is an assertion made once, possibly minutes ago, and its entire safety
 * story is that the console will show it to you. That story is void the moment the console
 * cannot be trusted, so a latched transmission surviving a signalling drop is a hot mic that
 * by definition nobody can be told about. A momentary key is a human continuously asserting
 * intent with their thumb, so it survives — which is why this is one flag about latching
 * rather than a second emission predicate.
 */
export function theConnection({ onConnection }) {
	let ladder = THE_LADDER_V1_FIXES;
	// When the channel was last proved. Null until the socket is open: a console with no
	// socket at all has not lost one, and there is nothing yet to measure a gap from.
	let heardFrom = null;
	let gone = false;
	let told = null;

	function standing(at) {
		const since = heardFrom === null ? 0 : Math.max(0, at - heardFrom);
		// A socket that closed skips the ladder. The rungs measure a silence; this is not one.
		const state = gone
			? DISCONNECTED
			: since >= ladder.disconnected_ms
				? DISCONNECTED
				: since >= ladder.unconfirmed_ms
					? UNCONFIRMED
					: CONFIRMED;

		return {
			state,
			since,
			aLatchStands: !gone && since < ladder.unconfirmed_ms + ladder.latch_dropped_ms
		};
	}

	// Said again when the rung moves, when a latch stops standing, or when the age has
	// visibly aged. A caller told the same sentence twice a second would be a caller
	// redrawing a banner that has not changed a word.
	function say(now) {
		const worth =
			told === null ||
			told.state !== now.state ||
			told.aLatchStands !== now.aLatchStands ||
			(now.state !== CONFIRMED && Math.floor(told.since / 1000) !== Math.floor(now.since / 1000));
		if (!worth) return;

		told = now;
		onConnection(now);
	}

	return {
		/** The socket is open. The clock starts here rather than at nothing. */
		opened: (at) => {
			heardFrom = at;
			gone = false;
			say(standing(at));
		},

		/**
		 * A heartbeat arrived, carrying the clock this deployment runs on.
		 *
		 * It is the only thing that moves the ladder back down, and it moves it the whole
		 * way: the gap it is measured from starts again.
		 */
		confirmed: (at, said) => {
			if (said) ladder = { ...ladder, ...said };
			heardFrom = at;
			gone = false;
			say(standing(at));
		},

		/**
		 * The channel is known to have gone — the socket closed, rather than merely going
		 * quiet. There is nothing to wait out.
		 */
		gone: (at) => {
			gone = true;
			say(standing(at));
		},

		/** Where things stand now. The caller owns the timer that asks. */
		settle: (at) => say(standing(at))
	};
}

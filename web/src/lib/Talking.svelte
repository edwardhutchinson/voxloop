<script>
	// The talking indicator: **a loop is being spoken on, and never who** (ADR-0033).
	//
	// One indicator, identical for every talker and for any number of them. A loop is staffed
	// by people who can speak for it, so *the loop* is the identity, and asking which occupant
	// is talking asks the question the loop exists to make unnecessary. There is consequently
	// one thing to pass in here and it is not about anybody: whether the transmission is at
	// priority. A prop that varied it by talker would be the first step towards attribution,
	// and `tests/board-and-ledger.test.js` holds it to that one.
	//
	// **It is a component so that the motion is in one place.** The console renders no motion
	// anywhere else (v1 §8), and `tests/styling.test.js` refuses `animation`, `transition`,
	// `@keyframes` and Svelte's motion directives in every file but this one — which is what
	// makes *permitted in exactly one place* a failing build rather than a paragraph. It is
	// also what keeps the board's and the ledger's indicators the same indicator.
	//
	// **It may never imply amplitude.** DTX means silence sends no packets at all (ADR-0010),
	// so a bar or a waveform would be inventing a signal. The glyph steps between two states
	// at one fixed rate and reads unambiguously as on or off; there is no value in it.
	//
	// The word is beside the glyph because colour and motion are never what carries a state,
	// and because an operator glancing at twenty cards is reading words.
	//
	// **The priority mark is the indicator's one variant, and it is not attribution** (v1 §8,
	// ADR-0046). It says what *kind* of transmission is on the loop and still never whose. It
	// is a **declaration that somebody called this urgent** rather than an explanation of why
	// the audio got louder (ADR-0059), so whoever draws it draws it on every loop that says so
	// — at full volume, muted, or not monitored at all. It is the same glyph at the same rate,
	// because the one permitted motion is one shape and one rate; what differs is a word and the
	// colour v1 §8 keeps for *this is true and you should look at it*. It lives exactly as long
	// as the document says so, with no floor: a sub-second press may never be drawn at all, and
	// the audit log is then the only record of it.
	let { priority = false } = $props();
</script>

<span class="talking" class:priority>
	<span class="glyph" aria-hidden="true"></span>
	{priority ? 'Talking — Priority' : 'Talking'}
</span>

<style>
	.talking {
		display: inline-flex;
		align-items: center;
		gap: var(--space-1);
		font-size: var(--type-2);
	}

	/* The colour is the warning token's, and it is never what carries the state: the word does,
	   and would still say it in monochrome. The weight is what finds it at the edge of vision
	   without a second motion. */
	.priority {
		color: var(--warning);
		font-weight: 600;
	}

	.priority .glyph {
		background: var(--warning);
	}

	/* Sized in `em` so it sits with whatever text it is beside, the way `Icon.svelte` is, and
	   drawn as the one radius the console has rather than as a circle — `--radius` is the
	   whole of the scale here and a `50%` would be a second one. */
	.glyph {
		width: 0.55em;
		height: 0.55em;
		border-radius: var(--radius);
		background: var(--ink);
		/* **One fixed rate and two states.** `steps` rather than a smooth fade, deliberately:
		   a continuous ramp reads as a level, and a level is the one thing this may never
		   imply. A second is slow enough not to pull the eye off telemetry and fast enough to
		   read as live. */
		animation: talking 1s steps(2, jump-none) infinite;
	}

	@keyframes talking {
		from {
			opacity: 1;
		}
		to {
			opacity: 0.2;
		}
	}
</style>

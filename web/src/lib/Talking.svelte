<script>
	// The talking indicator: **a loop is being spoken on, and never who** (ADR-0033).
	//
	// One indicator, identical for every talker and for any number of them. A loop is staffed
	// by people who can speak for it, so *the loop* is the identity, and asking which occupant
	// is talking asks the question the loop exists to make unnecessary. There is consequently
	// nothing to pass in here: it is on or it is not, and a prop that varied it would be the
	// first step towards attribution.
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
	// and because an operator glancing at twenty cards is reading words. The priority mark is
	// the indicator's one variant and it is #45's — it says what *kind* of transmission is on
	// the loop and still never whose.
</script>

<span class="talking">
	<span class="glyph" aria-hidden="true"></span>
	Talking
</span>

<style>
	.talking {
		display: inline-flex;
		align-items: center;
		gap: var(--space-1);
		font-size: var(--type-2);
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

<script>
	// The cog's modal: how loud one loop plays in this operator's ears, and nothing else.
	//
	// **Scoped to the loop and holding only volume in v1** (v1 §8, ADR-0034). It is opened
	// from the loop, so there is no loop picker in it — a single settings panel would have to
	// rebuild the list the operator just clicked from. And it is a modal rather than a control
	// on the card because **per-loop volume is not a live operational control** (v1 §5): it is
	// personalisation, remembered per (user, role, loop), and it must not be nudgeable from the
	// main surface.
	//
	// **The level shown is the document's.** The slider starts where the presence document
	// says the loop is and the sentence under it says the same, and a change is sent when the
	// slider is let go — so what the sentence says moves when VoxLoop confirms it, not when the
	// hand moves (ADR-0016). Nothing here keeps a level of its own.
	//
	// It is a native `<dialog>` opened modally, so the page behind it is inert, focus lands on
	// the slider, and Escape closes it — all of which is the browser's rather than code here to
	// get wrong. It is a component because it holds a decision, which is where this console
	// draws that line (`docs/agents/styling.md`).
	import Icon from './Icon.svelte';

	let { loop, onSet, onClose } = $props();

	// Opened as a modal when it is put on the page and closed when it is taken off, so that
	// whether it is open is decided by whoever holds it rather than by two things agreeing.
	const modally = (dialog) => {
		dialog.showModal();
		return () => dialog.close();
	};
</script>

<dialog aria-labelledby="volume-of-{loop.id}" onclose={onClose} {@attach modally}>
	<h2 id="volume-of-{loop.id}">{loop.name} volume</h2>
	<label class="field">
		How loud {loop.name} plays in your ears
		<input
			type="range"
			min="0"
			max="100"
			step="5"
			value={loop.volume}
			onchange={(event) => onSet(Number(event.currentTarget.value))}
		/>
	</label>
	<p>
		{loop.volume < 100
			? `${loop.volume}% of full volume.`
			: 'Full volume, which is where every loop starts.'}
		Only you hear this, and nobody is told you have turned it down.
	</p>
	<button onclick={onClose}><Icon name="x" /> Close</button>
</dialog>

<style>
	/* A measure for one slider and two sentences: wide enough that the slider has travel to it,
	   narrow enough that the sentence under it is read as one line of thought. */
	dialog {
		max-width: 24rem;
		background: var(--raised);
		color: var(--ink);
		border: 1px solid var(--rule);
		border-radius: var(--radius);
		padding: var(--space-5);
	}

	/* The page stays visible behind it, dimmed rather than hidden: the operator opened this
	   from a loop they can still see, and the loops go on moving underneath while it is open. */
	dialog::backdrop {
		background: var(--ground);
		opacity: 0.7;
	}

	h2 {
		margin-top: 0;
	}

	input {
		width: 100%;
	}

	p {
		margin: var(--space-3) 0 var(--space-4);
		color: var(--quiet);
		font-size: var(--type-2);
	}
</style>

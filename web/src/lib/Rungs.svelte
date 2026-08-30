<script>
	// The ordered four, as one control: `none`, `monitor`, `emit`, `control`, each rung
	// carrying everything below it (ADR-0011). They are laid out in that order and always all
	// four, because the ladder is the model and a control that hid `none` would make taking a
	// permission away look like a different kind of act from granting one.
	//
	// There is nothing else on a cell. No per-user exception, no deny, no override — if this
	// control ever grows a fifth thing, the model has grown a second layer.
	let { held, of, onset, busy = false } = $props();

	const rungs = ['none', 'monitor', 'emit', 'control'];
</script>

<div class="rungs" role="group" aria-label="Permission for {of}">
	{#each rungs as rung (rung)}
		<button
			class:held={rung === held}
			aria-pressed={rung === held}
			disabled={busy}
			onclick={() => onset(rung)}
		>
			{rung}
		</button>
	{/each}
</div>

<style>
	.rungs {
		display: flex;
		gap: 0.25rem;
	}

	.rungs button {
		font-size: 0.8rem;
		padding: 0.2rem 0.55rem;
		color: var(--quiet);
	}

	.held {
		color: var(--ink);
		border-color: var(--ink);
	}
</style>

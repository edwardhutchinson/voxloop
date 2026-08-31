<script>
	// The ordered four, as one control: `none`, `monitor`, `emit`, `control`, each rung
	// carrying everything below it (ADR-0011). They are laid out in that order and always all
	// four, because the ladder is the model and a control that hid `none` would make taking a
	// permission away look like a different kind of act from granting one.
	//
	// There is nothing else on a cell. No per-user exception, no deny, no override — if this
	// control ever grows a fifth thing, the model has grown a second layer.
	import Icon from './Icon.svelte';

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
			<!-- The mark, and not only the brighter ink: which rung a cell holds is a state, and
			     a state is never carried by colour alone. -->
			<span class="mark"
				>{#if rung === held}<Icon name="check" />{/if}</span
			>
			{rung}
		</button>
	{/each}
</div>

<style>
	.rungs {
		display: flex;
		gap: var(--space-1);
	}

	.rungs button {
		font-size: var(--type-1);
		padding: var(--space-1) var(--space-2);
		color: var(--quiet);
	}

	/* One em wide whether or not it holds the mark, which is the width of the icon that goes
	   in it: setting a permission must not shuffle the other three rungs sideways under the
	   pointer that has just pressed one. */
	.mark {
		display: inline-block;
		width: 1em;
	}

	.held {
		color: var(--ink);
		border-color: var(--ink);
	}
</style>

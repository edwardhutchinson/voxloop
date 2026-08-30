<script>
	// Every act with a consequence beyond the record itself is confirmed against what it will
	// do, **in words**, before it is committed (ADR-0015). The words are the whole of the
	// panel: a dialog that says *are you sure* asks a question the administrator cannot
	// answer, and the consequences a live deployment adds — who is cut mid-word, whose
	// subscriptions drop — arrive as the blast radius the server computes, in this same place.
	let { consequence, oncommit, oncancel } = $props();
</script>

<div class="confirming" role="alertdialog">
	<p>{consequence}</p>
	<button onclick={oncommit}>Commit</button>
	<button onclick={oncancel}>Cancel</button>
</div>

<style>
	.confirming {
		position: fixed;
		inset: auto 1.5rem 1.5rem auto;
		max-width: 28rem;
		padding: 1rem;
		background: var(--raised);
		border: 1px solid var(--rule);
		border-radius: 0.25rem;
	}

	.confirming p {
		margin: 0 0 0.75rem;
	}
</style>

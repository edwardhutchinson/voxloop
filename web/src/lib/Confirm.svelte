<script>
	// Every act with a consequence beyond the record itself is confirmed against what it will
	// do, **in words**, before it is committed (ADR-0015). The words are the whole of the
	// panel: a dialog that says *are you sure* asks a question the administrator cannot
	// answer, and the consequences a live deployment adds — who is cut mid-word, whose
	// subscriptions drop — arrive as the blast radius the server computes, in this same place.
	let { consequence, oncommit, oncancel } = $props();
</script>

<div class="awaiting" role="alertdialog">
	<p>{consequence}</p>
	<button onclick={oncommit}>Commit</button>
	<button onclick={oncancel}>Cancel</button>
</div>

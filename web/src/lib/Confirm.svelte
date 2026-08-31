<script>
	// Every act with a consequence beyond the record itself is confirmed against what it will
	// do, **in words**, before it is committed (ADR-0015). The words are the whole of the
	// panel: a dialog that says *are you sure* asks a question the administrator cannot
	// answer, and the consequences a live deployment adds — who is cut mid-word, whose
	// subscriptions drop — arrive as the blast radius the server computes, in this same place.
	import Icon from './Icon.svelte';

	let { consequence, oncommit, oncancel } = $props();
</script>

<div class="awaiting" role="alertdialog">
	<p>{consequence}</p>
	<!-- The mark is on the act being committed and not on the way out of the panel: the
	     administrator has read a consequence and one of these two buttons accepts it. -->
	<button onclick={oncommit}><Icon name="check" /> Commit</button>
	<button onclick={oncancel}>Cancel</button>
</div>

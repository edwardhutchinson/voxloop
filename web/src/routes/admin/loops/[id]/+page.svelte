<script>
	// The loop's column: every role, by name, with the one value it holds on this loop.
	import { page } from '$app/state';

	import LoopPage from '$lib/LoopPage.svelte';
</script>

<LoopPage held={page.params.id} />

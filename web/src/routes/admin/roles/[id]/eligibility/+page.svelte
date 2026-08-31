<script>
	// Eligibility from the role's side: who may assume this position.
	import { page } from '$app/state';

	import WhoMayAssume from '$lib/WhoMayAssume.svelte';
</script>

<WhoMayAssume role={page.params.id} />

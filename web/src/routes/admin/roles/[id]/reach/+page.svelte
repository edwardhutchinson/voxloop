<script>
	// The role's row: what it may hear, say and command, one loop at a time.
	import { page } from '$app/state';

	import RolePage from '$lib/RolePage.svelte';
</script>

<RolePage role={page.params.id} />

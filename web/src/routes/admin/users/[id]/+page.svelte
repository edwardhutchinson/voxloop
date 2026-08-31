<script>
	// A user has one page under it: which roles they may assume.
	import { page } from '$app/state';

	import WhichRoles from '$lib/WhichRoles.svelte';
</script>

<WhichRoles user={page.params.id} />

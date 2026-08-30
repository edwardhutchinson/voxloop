<script>
	// The console reads one row at a time (ADR-0015), so it is a page per thing being
	// administered rather than one screen of everything: users, the roles they may assume,
	// and the loops voice is addressed to. Which role may hear or say what on which loop is
	// the grid, and it is its own page when it arrives.
	import Loops from './Loops.svelte';
	import Roles from './Roles.svelte';
	import Users from './Users.svelte';

	const pages = [
		{ name: 'Users', page: Users },
		{ name: 'Roles', page: Roles },
		{ name: 'Loops', page: Loops }
	];

	let showing = $state(pages[0]);
	const Showing = $derived(showing.page);
</script>

<nav>
	{#each pages as page (page.name)}
		<button class:showing={page === showing} onclick={() => (showing = page)}>
			{page.name}
		</button>
	{/each}
</nav>

<Showing />

<style>
	nav {
		display: flex;
		gap: 0.5rem;
		margin-bottom: 2rem;
	}

	.showing {
		border-color: var(--ink);
	}
</style>

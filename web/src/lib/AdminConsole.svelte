<script>
	// The console reads one row at a time (ADR-0015), so it is a page per thing being
	// administered rather than one screen of everything: users, the roles they may assume,
	// and the loops voice is addressed to.
	//
	// Which role may hear or say what on which loop is the grid, and it is administered from
	// a role's page or a loop's page — the row and the column — rather than from a wall of
	// cells. The wall is here as **Grid**, last and read-only, because a whole-configuration
	// read is a reviewing act rather than an administering one.
	import Loops from './Loops.svelte';
	import Matrix from './Matrix.svelte';
	import Roles from './Roles.svelte';
	import Users from './Users.svelte';

	const pages = [
		{ name: 'Users', page: Users },
		{ name: 'Roles', page: Roles },
		{ name: 'Loops', page: Loops },
		{ name: 'Grid', page: Matrix }
	];

	let showing = $state(pages[0]);
	const Showing = $derived(showing.page);
</script>

<nav>
	{#each pages as page (page.name)}
		<button
			class:showing={page === showing}
			aria-current={page === showing ? 'page' : undefined}
			onclick={() => (showing = page)}
		>
			{page.name}
		</button>
	{/each}
</nav>

<Showing />

<style>
	nav {
		display: flex;
		gap: var(--space-2);
		margin-bottom: var(--space-6);
	}

	/* The page being read is marked twice over: the border brightens and the name is
	   underlined. A brighter border on its own would be a state carried by colour, which is
	   the one thing the standard will not have. */
	.showing {
		border-color: var(--ink);
		text-decoration: underline;
	}
</style>

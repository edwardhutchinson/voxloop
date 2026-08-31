<script>
	// The admin console, and the frame for every page of it.
	//
	// It reads one row at a time (ADR-0015), so it is a page per thing being administered
	// rather than one screen of everything: users, the roles they may assume, and the loops
	// voice is addressed to. Which role may hear or say what on which loop is the grid, and it
	// is administered from a role's page or a loop's page — the row and the column — rather
	// than from a wall of cells. The wall is here as **Grid**, last and read-only, because a
	// whole-configuration read is a reviewing act rather than an administering one.
	//
	// **These pages have URLs and the lobby does not** (#76). An administration page claims
	// nothing live: it is a read of configuration, it cannot go stale the way a session can,
	// and it is the thing somebody wants to paste into a chat — *look at this loop's column*.
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	import Icon from '$lib/Icon.svelte';
	import { theFrame } from '$lib/frame.js';

	let { children } = $props();

	const frame = theFrame();

	const pages = [
		{ name: 'Users', at: '/admin/users' },
		{ name: 'Roles', at: '/admin/roles' },
		{ name: 'Loops', at: '/admin/loops' },
		{ name: 'Grid', at: '/admin/grid' }
	];

	// A role's reach is still **Roles**: the pages under a list belong to the list they were
	// reached from, and a nav that unmarked itself two clicks in would be saying the reader
	// had left the console.
	const reading = (at) => page.url.pathname === at || page.url.pathname.startsWith(`${at}/`);
</script>

{#if frame.who?.system_administration}
	<nav>
		{#each pages as held (held.at)}
			<a href={resolve(held.at)} aria-current={reading(held.at) ? 'page' : undefined}>
				{held.name}
			</a>
		{/each}
	</nav>

	{@render children()}
{:else}
	<!-- A page reached by URL is not a page reached by permission. Every read behind these
	     pages is refused to a caller without the flag, so nothing under here would answer;
	     what is owed is the reason, said plainly, rather than a page of empty tables. The way
	     out is here too, because the frame's own way in to the console is not shown to
	     somebody who does not hold it. -->
	<p class="refusal" role="alert">
		The admin console configures VoxLoop, and it opens on system administration — a flag held by the
		user rather than by any role. You do not hold it, so nothing on these pages would answer you.
	</p>
	<p class="back"><a href={resolve('/')}><Icon name="arrow-left" /> Lobby</a></p>
{/if}

<style>
	nav {
		display: flex;
		gap: var(--space-2);
		margin-bottom: var(--space-6);
	}

	/* The page being read is marked twice over: the border brightens and the name is
	   underlined. A brighter border on its own would be a state carried by colour, which is
	   the one thing the standard will not have. */
	nav a[aria-current='page'] {
		border-color: var(--ink);
		text-decoration: underline;
	}
</style>

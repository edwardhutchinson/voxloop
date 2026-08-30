<script>
	// The matrix, and **only** as a reference view (ADR-0015). Administering is done a row or
	// a column at a time, on a role page or a loop page; this is the one place a whole
	// configuration can be read at once, which is a reviewing act — checking the shape of a
	// grid, spotting a role with no reach or a loop nobody can hear.
	//
	// Nothing is edited here, deliberately. A prototype found the matrix is not how
	// administrators reason: a realistic pilot grid fills 167 of 300 cells, and past roughly
	// thirty loops the row header and the far column cannot share a screen.
	import { theGrid, whatWentWrong } from './server.js';

	let grid = $state(null);
	let refusal = $state(null);
	let reading = $state(true);

	$effect(() => {
		read();
	});

	async function read() {
		reading = true;
		refusal = null;
		try {
			grid = await theGrid();
		} catch (said) {
			refusal = whatWentWrong(said);
		}
		reading = false;
	}

	// The cells come back by their pair's ids, because the names are on both axes already.
	const held = $derived(
		new Map((grid?.cells ?? []).map((cell) => [`${cell.role}/${cell.loop}`, cell.permission]))
	);

	const permission = (role, loop) => held.get(`${role.id}/${loop.id}`) ?? 'none';
</script>

<section>
	<header>
		<h2>Grid</h2>
		<p>
			Every role against every loop, for reading rather than for working in. Set a
			permission from a role's page or a loop's page, where one cell is one line at full
			size.
		</p>
	</header>

	{#if refusal}
		<p class="refusal" role="alert">{refusal}</p>
	{:else if reading}
		<p class="quiet">Reading…</p>
	{:else if grid.roles.length === 0 || grid.loops.length === 0}
		<p class="quiet">
			There is nothing to lay out yet: a grid is roles against loops, and this deployment
			has {grid.roles.length} of one and {grid.loops.length} of the other.
		</p>
	{:else}
		<div class="scrolls">
			<table>
				<thead>
					<tr>
						<th>Role</th>
						{#each grid.loops as loop (loop.id)}
							<th class:unreviewed={loop.unreviewed}>{loop.name}</th>
						{/each}
					</tr>
				</thead>
				<tbody>
					{#each grid.roles as role (role.id)}
						<tr>
							<td>{role.name}</td>
							{#each grid.loops as loop (loop.id)}
								<td class:none={permission(role, loop) === 'none'}>
									{permission(role, loop)}
								</td>
							{/each}
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
		<p class="quiet">
			A loop nobody has ruled on is marked in its heading. Its cells are enforced as
			<strong>none</strong> whatever they say here, and it stays that way until somebody
			rules on it from its loop page.
		</p>
	{/if}
</section>

<style>
	/* The break point is real rather than hypothetical: past roughly 26 to 30 loops this
	   scrolls sideways, and a column read stops being a glance. That is why the pages
	   administrators work in are lists. */
	.scrolls {
		overflow-x: auto;
	}

	td,
	th {
		white-space: nowrap;
		padding-right: 1rem;
	}

	.none {
		color: var(--quiet);
	}

	.unreviewed::after {
		content: ' ·unreviewed';
		color: var(--quiet);
		font-weight: normal;
	}
</style>

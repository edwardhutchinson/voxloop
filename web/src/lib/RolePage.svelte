<script>
	// A **role page is the row** (ADR-0015): every loop, in the base order, with the one value
	// this role holds on it. It is read as a list at full size rather than as a line of a
	// matrix, because that is how administrators were found to reason about it — and because
	// past roughly thirty loops a row's header and its far end cannot share a screen.
	//
	// This page answers *what can this role reach*. Who may assume the role is eligibility,
	// and it is not here.
	import Icon from './Icon.svelte';
	import Rungs from './Rungs.svelte';
	import { roleRow, setCell, whatWentWrong } from './server.js';

	let { role, onback } = $props();

	let row = $state(null);
	let refusal = $state(null);
	let reading = $state(true);
	let setting = $state(null);

	$effect(() => {
		read(role.id);
	});

	async function read(id) {
		reading = true;
		await attempt(async () => {
			row = await roleRow(id);
		});
		reading = false;
	}

	async function attempt(what) {
		refusal = null;
		try {
			await what();
		} catch (said) {
			refusal = whatWentWrong(said);
		}
	}

	// One cell, one write, one audit entry. Nothing here is optimistic: the list is read again
	// afterwards, so what is on screen is what the deployment holds rather than what the
	// console asked for.
	async function set(held, permission) {
		setting = held.id;
		await attempt(() => setCell(role.id, held.id, permission));
		setting = null;
		await read(role.id);
	}
</script>

<section>
	<header>
		<h2>{row?.role.name ?? role.name}</h2>
		<p>
			What this role may hear, say and command, one loop at a time. Each rung carries the ones below
			it: <strong>control</strong> can emit, and
			<strong>emit</strong> can monitor. Granting one person one extra loop costs a role — there is no
			per-person exception anywhere in VoxLoop.
		</p>
	</header>

	<p class="back"><button onclick={onback}><Icon name="arrow-left" /> All roles</button></p>

	{#if refusal}
		<p class="refusal" role="alert">{refusal}</p>
	{/if}

	{#if reading}
		<p class="quiet">Reading…</p>
	{:else if row && row.cells.length === 0}
		<p class="quiet">No loops yet. A role reaches loops, so create one first.</p>
	{:else if row}
		<table>
			<thead>
				<tr>
					<th>Loop</th>
					<th>Permission</th>
				</tr>
			</thead>
			<tbody>
				{#each row.cells as cell (cell.loop.id)}
					<tr>
						<td>
							{cell.loop.name}
							<!-- An unreviewed loop is enforced as `none` on every rung whatever
							     its cells say, so a value set here confers nothing until that
							     loop is ruled on — which happens when every role's cell on it
							     has been set, or when somebody dismisses the mark from the
							     loop's own page. Either way it is per loop, never per cell. -->
							{#if cell.loop.unreviewed}
								<span class="note">
									unreviewed — enforced as none until every role's cell on this loop is set, or the
									mark is dismissed from its loop page
								</span>
							{/if}
						</td>
						<td>
							<Rungs
								held={cell.permission}
								of="{role.name} on {cell.loop.name}"
								busy={setting === cell.loop.id}
								onset={(permission) => set(cell.loop, permission)}
							/>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	{/if}
</section>

<style>
	.back {
		margin: 0 0 var(--space-5);
	}

	.note {
		display: block;
		color: var(--quiet);
		font-size: var(--type-1);
	}
</style>

<script>
	// A **loop page is the column** (ADR-0015): every role, by name, with the one value it
	// holds on this loop. It answers *who may hear this loop* — the question a permission
	// change is usually made to answer — and it is the other reading of the same cells the
	// role page shows.
	//
	// It is also where a loop is **ruled on**. The mark is cleared per loop and never per
	// cell, and dismissing it records a deliberate `none` against every role left alone, so
	// from that moment the column says what somebody decided rather than what nobody has
	// looked at.
	import Confirm from './Confirm.svelte';
	import Icon from './Icon.svelte';
	import Rungs from './Rungs.svelte';
	import { dismissUnreviewed, loopColumn, setCell, whatWentWrong } from './server.js';

	let { held, onback } = $props();

	let column = $state(null);
	let refusal = $state(null);
	let reading = $state(true);
	let setting = $state(null);
	let confirming = $state(null);

	$effect(() => {
		read(held.id);
	});

	async function read(id) {
		reading = true;
		await attempt(async () => {
			column = await loopColumn(id);
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

	async function set(role, permission) {
		setting = role.id;
		await attempt(() => setCell(role.id, held.id, permission));
		setting = null;
		await read(held.id);
	}

	async function rule() {
		confirming = null;
		await attempt(() => dismissUnreviewed(held.id));
		await read(held.id);
	}
</script>

<section>
	<header>
		<h2>{column?.loop.name ?? held.name}</h2>
		<p>
			Who may hear this loop, say anything on it, and hold operational authority over it. An absent
			permission and a <strong>none</strong> are the same thing to the server; the difference is only
			whether anybody has ruled on it.
		</p>
	</header>

	<p class="back"><button onclick={onback}><Icon name="arrow-left" /> All loops</button></p>

	{#if refusal}
		<p class="refusal" role="alert">{refusal}</p>
	{/if}

	{#if column?.loop.unreviewed}
		<p class="unreviewed" role="status">
			Nobody has ruled on this loop. Every cell below is enforced as <strong>none</strong>
			until somebody does, whatever it is set to — either by setting every role's permission here, or
			by dismissing the mark in one act.
			<button
				onclick={() =>
					(confirming = {
						consequence: `${held.name} is ruled on. Every role you have left at none is recorded as a deliberate none, and the permissions set here start applying.`
					})}
			>
				Rule on this loop
			</button>
		</p>
	{/if}

	{#if reading}
		<p class="quiet">Reading…</p>
	{:else if column}
		<table>
			<thead>
				<tr>
					<th>Role</th>
					<th>Permission</th>
				</tr>
			</thead>
			<tbody>
				{#each column.cells as cell (cell.role.id)}
					<tr>
						<td>{cell.role.name}</td>
						<td>
							<Rungs
								held={cell.permission}
								of="{cell.role.name} on {held.name}"
								busy={setting === cell.role.id}
								onset={(permission) => set(cell.role, permission)}
							/>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	{/if}

	{#if confirming}
		<Confirm
			consequence={confirming.consequence}
			oncommit={rule}
			oncancel={() => (confirming = null)}
		/>
	{/if}
</section>

<style>
	.unreviewed {
		display: flex;
		align-items: center;
		gap: var(--space-3);
		flex-wrap: wrap;
		margin: 0 0 var(--space-5);
		font-size: var(--type-2);
	}
</style>

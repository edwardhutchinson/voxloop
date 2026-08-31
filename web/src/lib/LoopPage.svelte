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
	//
	// It has a URL of its own (#76), so what arrives here is a loop's **id** and not a loop:
	// somebody who pasted the link into a chat sent an id, and the reader's console has been
	// told nothing else about it. Everything the page says about the loop is what the server
	// answered — including that there is no such loop, which is a sentence rather than a
	// blank page.
	import { resolve } from '$app/paths';

	import Confirm from './Confirm.svelte';
	import Icon from './Icon.svelte';
	import Rungs from './Rungs.svelte';
	import { dismissUnreviewed, loopColumn, setCell, whatWentWrong } from './server.js';

	let { held } = $props();

	let column = $state(null);
	let refusal = $state(null);
	let reading = $state(true);
	let setting = $state(null);
	let confirming = $state(null);

	$effect(() => {
		read(held);
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
		await attempt(() => setCell(role.id, held, permission));
		setting = null;
		await read(held);
	}

	async function rule() {
		confirming = null;
		await attempt(() => dismissUnreviewed(held));
		await read(held);
	}
</script>

<section>
	<!-- Above the heading, because the heading may never arrive: a link to a loop somebody
	     deleted has no name to show, and the way back out has to be on screen anyway. -->
	<p class="back">
		<a href={resolve('/admin/loops')}><Icon name="arrow-left" /> All loops</a>
	</p>

	{#if reading}
		<p class="quiet">Reading…</p>
	{:else if !column}
		<!-- There is no such loop, or the caller may not read it. Both are the server's own
		     sentence and the page is that sentence: an empty column would be the console
		     answering a question only the server can answer. -->
		<p class="refusal" role="alert">{refusal}</p>
	{:else}
		<header>
			<h2>{column.loop.name}</h2>
			<p>
				Who may hear this loop, say anything on it, and hold operational authority over it. An
				absent permission and a <strong>none</strong> are the same thing to the server; the difference
				is only whether anybody has ruled on it.
			</p>
		</header>

		{#if refusal}
			<p class="refusal" role="alert">{refusal}</p>
		{/if}

		{#if column.loop.unreviewed}
			<p class="unreviewed" role="status">
				Nobody has ruled on this loop. Every cell below is enforced as <strong>none</strong>
				until somebody does, whatever it is set to — either by setting every role's permission here, or
				by dismissing the mark in one act.
				<button
					onclick={() =>
						(confirming = {
							consequence: `${column.loop.name} is ruled on. Every role you have left at none is recorded as a deliberate none, and the permissions set here start applying.`
						})}
				>
					Rule on this loop
				</button>
			</p>
		{/if}

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
								of="{cell.role.name} on {column.loop.name}"
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

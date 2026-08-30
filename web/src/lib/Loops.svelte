<script>
	// A loop is an audio conference and the only thing voice can be addressed to (ADR-0001).
	// There is no kind, no type and no naming convention: a private room is an ordinary loop
	// an administrator configured, and VoxLoop neither knows nor cares (ADR-0055).
	//
	// The order this list is read in is the deployment's **base loop order**, which is
	// administered rather than derived (ADR-0053) — not alphabetical, and not creation order.
	// A new loop lands at the end, because appending is the only honest placement for
	// something VoxLoop has been told nothing about.
	import Confirm from './Confirm.svelte';
	import {
		createLoop,
		deleteLoop,
		editLoop,
		loops,
		setLoopOrder,
		whatWentWrong
	} from './server.js';

	let allLoops = $state([]);
	// The order as the server last answered it, so an arrangement in progress can be told
	// apart from the one that is actually saved. Nothing here renders optimistically: the
	// list says plainly that a rearrangement has not been committed yet.
	let saved = $state([]);
	let refusal = $state(null);
	let reading = $state(true);
	let confirming = $state(null);
	let creating = $state({ name: '' });
	let editing = $state(null);

	const arranged = $derived(allLoops.map((held) => held.id).join() !== saved.join());

	$effect(() => {
		read();
	});

	async function read() {
		reading = true;
		await attempt(async () => {
			allLoops = await loops();
			saved = allLoops.map((held) => held.id);
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

	async function create(event) {
		event.preventDefault();
		await attempt(async () => {
			await createLoop(creating.name);
			creating = { name: '' };
		});
		await read();
	}

	async function rename(event) {
		event.preventDefault();
		const { id, name } = editing;
		editing = null;
		await attempt(() => editLoop(id, { name }));
		await read();
	}

	// Moving a loop rearranges this list and nothing else. The order is one write, sent when
	// the administrator says so, rather than one write per press: an order arrived at by six
	// clicks is one decision, and the audit log should read as one.
	//
	// This is the one place the console shows something the server has not agreed to, and it
	// is not optimistic rendering: an arrangement in progress is marked as unsaved until it
	// is committed, so what is on screen is never asserted to be what the deployment holds.
	function move(at, by) {
		const to = at + by;
		if (to < 0 || to >= allLoops.length) {
			return;
		}

		const rearranged = [...allLoops];
		[rearranged[at], rearranged[to]] = [rearranged[to], rearranged[at]];
		allLoops = rearranged;
	}

	async function save() {
		await attempt(() => setLoopOrder(allLoops.map((held) => held.id)));
		await read();
	}

	async function commit() {
		const { act } = confirming;
		confirming = null;
		await attempt(act);
		await read();
	}
</script>

<section>
	<header>
		<h2>Loops</h2>
		<p>
			A loop is an audio conference, and the only thing voice can be addressed to. This
			order is the deployment's base order: it is administered here rather than derived,
			and every console starts from it. A new loop lands at the end.
		</p>
	</header>

	{#if refusal}
		<p class="refusal" role="alert">{refusal}</p>
	{/if}

	<form class="new" onsubmit={create}>
		<input bind:value={creating.name} placeholder="Loop" required />
		<button type="submit">Create</button>
	</form>

	{#if reading}
		<p class="quiet">Reading…</p>
	{:else if allLoops.length === 0}
		<p class="quiet">No loops yet. Create the first one above.</p>
	{:else}
		<table>
			<thead>
				<tr>
					<th>Order</th>
					<th>Loop</th>
					<th>Review</th>
					<th class="acts">Acts</th>
				</tr>
			</thead>
			<tbody>
				{#each allLoops as held, at (held.id)}
					<tr>
						<td class="place">
							<button
								aria-label="Move {held.name} up"
								disabled={at === 0}
								onclick={() => move(at, -1)}>↑</button
							>
							<button
								aria-label="Move {held.name} down"
								disabled={at === allLoops.length - 1}
								onclick={() => move(at, 1)}>↓</button
							>
						</td>
						<td>
							{#if editing?.id === held.id}
								<form onsubmit={rename}>
									<!-- svelte-ignore a11y_autofocus -->
									<input bind:value={editing.name} autofocus required />
									<button type="submit">Rename</button>
									<button type="button" onclick={() => (editing = null)}>Cancel</button>
								</form>
							{:else}
								<button
									class="name"
									onclick={() => (editing = { id: held.id, name: held.name })}
								>
									{held.name}
								</button>
							{/if}
						</td>
						<!-- Every loop is unreviewed until an administrator has set or dismissed
						     each role's cell, which is the grid's act: nothing on this page
						     clears the mark, and nothing here pretends to. -->
						<td class:quiet={!held.unreviewed}>
							{#if held.unreviewed}
								unreviewed
								<span class="note">nobody has ruled on this loop's permissions yet</span>
							{:else}
								ruled on
							{/if}
						</td>
						<td class="acts">
							<button
								class="destructive"
								onclick={() =>
									(confirming = {
										act: () => deleteLoop(held.id),
										consequence: `${held.name} is deleted. Nothing can be said on it or heard from it again, and the loops around it keep their order.`
									})}>Delete</button
							>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>

		{#if arranged}
			<p class="unsaved" role="status">
				This order has not been saved. Nothing has changed for anybody until it is.
				<button onclick={save}>Save order</button>
				<button onclick={read}>Discard</button>
			</p>
		{/if}
	{/if}

	{#if confirming}
		<Confirm
			consequence={confirming.consequence}
			oncommit={commit}
			oncancel={() => (confirming = null)}
		/>
	{/if}
</section>

<style>
	.place {
		white-space: nowrap;
	}

	.note {
		display: block;
		color: var(--quiet);
		font-size: 0.75rem;
	}

	.unsaved {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		margin-top: 1rem;
		font-size: 0.85rem;
	}
</style>

<script>
	// A role is a staffable position with a limit on how many may occupy it, not a group of
	// users (v1 §1). Single-occupant and multi-occupant roles are one concept under different
	// limits, so there is one list here and no kinds.
	//
	// Who may assume a role is eligibility, and what a role may hear or say is the grid.
	// Neither is here: this page administers the position itself.
	import Confirm from './Confirm.svelte';
	import { createRole, deleteRole, editRole, roles, whatWentWrong } from './server.js';

	let allRoles = $state([]);
	let refusal = $state(null);
	let reading = $state(true);
	let confirming = $state(null);
	let creating = $state({ name: '', maxOccupants: '' });
	let editing = $state(null);

	$effect(() => {
		read();
	});

	async function read() {
		reading = true;
		await attempt(async () => {
			allRoles = await roles();
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

	// An empty box is no limit rather than a limit of nothing: the field is how an
	// administrator says *anybody who is eligible*, which is what `Observer` ships as.
	function limit(typed) {
		return typed === '' ? null : Number(typed);
	}

	function occupancy(role) {
		return role.max_occupants === null ? 'no limit' : role.max_occupants;
	}

	async function create(event) {
		event.preventDefault();
		await attempt(async () => {
			await createRole(creating.name, limit(creating.maxOccupants));
			creating = { name: '', maxOccupants: '' };
		});
		await read();
	}

	async function edit(event) {
		event.preventDefault();
		const { id, name, maxOccupants } = editing;
		editing = null;
		// One act, one entry: a rename and a change of limit are sent together.
		await attempt(() => editRole(id, { name, max_occupants: limit(maxOccupants) }));
		await read();
	}

	function open(role) {
		editing = {
			id: role.id,
			name: role.name,
			maxOccupants: role.max_occupants === null ? '' : String(role.max_occupants)
		};
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
		<h2>Roles</h2>
		<p>
			A role is a position somebody assumes, with a limit on how many may hold it at once.
			Leave the limit empty for no limit. Install seeds <strong>Observer</strong>, which a
			site renames like any other role.
		</p>
	</header>

	{#if refusal}
		<p class="refusal" role="alert">{refusal}</p>
	{/if}

	<form class="new" onsubmit={create}>
		<input bind:value={creating.name} placeholder="Role" required />
		<input
			class="limit"
			type="number"
			min="1"
			bind:value={creating.maxOccupants}
			placeholder="Max occupants"
		/>
		<button type="submit">Create</button>
	</form>

	{#if reading}
		<p class="quiet">Reading…</p>
	{:else}
		<table>
			<thead>
				<tr>
					<th>Role</th>
					<th>Max occupants</th>
					<th class="acts">Acts</th>
				</tr>
			</thead>
			<tbody>
				{#each allRoles as role (role.id)}
					<tr>
						{#if editing?.id === role.id}
							<td colspan="3">
								<form class="new" onsubmit={edit}>
									<!-- svelte-ignore a11y_autofocus -->
									<input bind:value={editing.name} autofocus required />
									<input
										class="limit"
										type="number"
										min="1"
										bind:value={editing.maxOccupants}
										placeholder="No limit"
									/>
									<button type="submit">Save</button>
									<button type="button" onclick={() => (editing = null)}>Cancel</button>
								</form>
							</td>
						{:else}
							<td>
								<button class="name" onclick={() => open(role)}>{role.name}</button>
							</td>
							<td class:quiet={role.max_occupants === null}>{occupancy(role)}</td>
							<td class="acts">
								<button onclick={() => open(role)}>Edit</button>
								<button
									class="destructive"
									onclick={() =>
										(confirming = {
											act: () => deleteRole(role.id),
											consequence: `${role.name} is deleted. Nobody can assume it again, and its audit entries stay, attributed.`
										})}>Delete</button
								>
							</td>
						{/if}
					</tr>
				{/each}
			</tbody>
		</table>
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
	.limit {
		width: 9rem;
	}
</style>

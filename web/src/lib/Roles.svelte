<script>
	// A role is a staffable position with a limit on how many may occupy it, not a group of
	// users (v1 §1). Single-occupant and multi-occupant roles are one concept under different
	// limits, so there is one list here and no kinds.
	//
	// A role has two pages under it, one for each question asked about it: **Reach** is the
	// role's row on the grid — what it may hear and say — and **Eligible** is who may assume
	// it. The second is one of eligibility's two directions (ADR-0015); the other hangs off
	// the user, and there is no view of the whole.
	//
	// They are at `/admin/roles/{id}/reach` and `/admin/roles/{id}/eligibility`, reached by a
	// link rather than by a variable in here (#76). The list is still the ordinary way in to
	// them, and it is no longer the only one.
	import { resolve } from '$app/paths';

	import Confirm from './Confirm.svelte';
	import Icon from './Icon.svelte';
	import { createRole, deleteRole, editRole, roles, whatWentWrong } from './server.js';

	// The list as the server last answered it, and `null` until it has answered at all: a
	// refused read and a deployment with no roles would otherwise render alike, and only one
	// of them is a fact about the deployment.
	let allRoles = $state(null);
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
			A role is a position somebody assumes, with a limit on how many may hold it at once. Leave the
			limit empty for no limit. <strong>Reach</strong> is what the role may hear and say;
			<strong>Eligible</strong>
			is who may assume it. Install seeds
			<strong>Observer</strong>, which a site renames like any other role.
		</p>
	</header>

	{#if refusal}
		<p class="refusal" role="alert">{refusal}</p>
	{/if}

	{#if reading}
		<p class="quiet">Reading…</p>
	{:else if allRoles}
		<form class="new" onsubmit={create}>
			<input bind:value={creating.name} placeholder="Role" required />
			<input
				class="limit"
				type="number"
				min="1"
				bind:value={creating.maxOccupants}
				placeholder="Max occupants"
			/>
			<button type="submit"><Icon name="plus" /> Create</button>
		</form>

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
								<a href={resolve('/admin/roles/[id]/reach', { id: role.id })}>Reach</a>
								<a href={resolve('/admin/roles/[id]/eligibility', { id: role.id })}>Eligible</a>
								<button onclick={() => open(role)}><Icon name="pencil" /> Edit</button>
								<button
									class="destructive"
									onclick={() =>
										(confirming = {
											act: () => deleteRole(role.id),
											consequence: `${role.name} is deleted, and every grant of eligibility for it goes with it. Nobody can assume it again, and its audit entries stay, attributed.`
										})}><Icon name="trash-2" /> Delete</button
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
	/* Wide enough for the `Max occupants` placeholder, which is the widest thing the box ever
	   holds: what it takes is a small number, and a field the width of the name beside it
	   would suggest otherwise. */
	.limit {
		width: 9rem;
	}
</style>

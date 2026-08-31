<script>
	// Eligibility from the **role's** side: *who may assume this*. It is one of the two
	// directions eligibility is administered from (ADR-0015), and the other is the user page.
	// There is no third view, because rendered as a matrix — 190 users against 15 roles — it
	// was the least legible object the console prototype produced.
	//
	// The list is the eligible **and nobody else**. Showing every user on the deployment with
	// a mark against some of them is the same wall, one slice at a time; whoever is not here
	// is picked from the box below, which is a list to search rather than a grid to read.
	//
	// Nothing granted here confers anything. Eligibility permits somebody to take up the
	// position; what the position can hear or say is the grid, one page across.
	//
	// It has a URL of its own (#76), so what arrives here is a role's **id** and not a role.
	// The name is the server's to give, and where there is no such role that is the server's
	// sentence too.
	import { resolve } from '$app/paths';

	import Confirm from './Confirm.svelte';
	import Icon from './Icon.svelte';
	import {
		grantEligibility,
		revokeEligibility,
		users,
		whatWentWrong,
		whoMayAssume
	} from './server.js';

	let { role } = $props();

	let page = $state(null);
	let everybody = $state([]);
	let refusal = $state(null);
	let reading = $state(true);
	let granting = $state('');
	let confirming = $state(null);

	// Whoever is not eligible yet. The picker is the whole of how somebody is added, so it
	// is read from the user list rather than from a second eligibility read: there is no call
	// that answers who is *not* eligible, and there should not be one.
	const candidates = $derived(
		everybody.filter((account) => !(page?.users ?? []).some((user) => user.id === account.id))
	);

	$effect(() => {
		read(role);
	});

	async function read(id) {
		reading = true;
		await attempt(async () => {
			page = await whoMayAssume(id);
			everybody = await users();
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

	// Granting takes nothing away, so it lands. Revoking ends the occupancy of whoever is in
	// the seat, which is a consequence beyond the record and is confirmed first.
	async function grant(event) {
		event.preventDefault();
		const user = granting;
		granting = '';
		await attempt(() => grantEligibility(user, role));
		await read(role);
	}

	async function commit() {
		const { act } = confirming;
		confirming = null;
		await attempt(act);
		await read(role);
	}
</script>

<section>
	<p class="back">
		<a href={resolve('/admin/roles')}><Icon name="arrow-left" /> All roles</a>
	</p>

	{#if reading}
		<p class="quiet">Reading…</p>
	{:else if !page}
		<!-- There is no such role, or the caller may not read it. Both are the server's own
		     sentence and the page is that sentence. -->
		<p class="refusal" role="alert">{refusal}</p>
	{:else}
		<header>
			<h2>{page.role.name}</h2>
			<p>
				Who may assume this position. Eligibility is an unconditional grant and it carries no
				permissions of its own — what this role can hear, say and command is its
				<strong>reach</strong>, one page across. Revoking it from somebody occupying the role ends
				their occupancy immediately, and they are told why.
			</p>
		</header>

		{#if refusal}
			<p class="refusal" role="alert">{refusal}</p>
		{/if}

		<form class="new" onsubmit={grant}>
			<select bind:value={granting} required aria-label="Somebody to make eligible">
				<option value="" disabled>Somebody else…</option>
				{#each candidates as account (account.id)}
					<option value={account.id}>{account.username}</option>
				{/each}
			</select>
			<button type="submit" disabled={granting === ''}>
				<Icon name="plus" /> Make eligible
			</button>
		</form>

		{#if page.users.length === 0}
			<p class="quiet">Nobody may assume this role. It is a position with no candidates.</p>
		{:else}
			<table>
				<thead>
					<tr>
						<th>User</th>
						<th class="acts">Acts</th>
					</tr>
				</thead>
				<tbody>
					{#each page.users as user (user.id)}
						<tr>
							<td>{user.username}</td>
							<td class="acts">
								<button
									class="destructive"
									onclick={() =>
										(confirming = {
											act: () => revokeEligibility(user.id, role),
											consequence: `${user.username} can no longer assume ${page.role.name}. If they are occupying it, their occupancy ends immediately and they are told why.`
										})}><Icon name="trash-2" /> Revoke</button
								>
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
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

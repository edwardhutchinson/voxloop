<script>
	// Eligibility from the **user's** side: *which roles may this person assume*. The other
	// direction of the same grants (ADR-0015), read from the person rather than from the
	// position.
	//
	// It answers eligibility and nothing beyond it. **Reach is deliberately not here**: a
	// person's reach belongs to a (user, role) pair and is never composed across the roles
	// they may assume, because a session is bound to one role and a union would display
	// authority nobody can hold. Answering *what can this person do* is this page and then
	// that role's reach — one extra hop, taken knowingly.
	//
	// It has a URL of its own (#76), so what arrives here is a user's **id** and not a user:
	// somebody who pasted the link into a chat sent an id, and the reader's console has been
	// told nothing else about it. The name is the server's to give, and where there is no such
	// user that is the server's sentence too.
	import { resolve } from '$app/paths';

	import Confirm from './Confirm.svelte';
	import Icon from './Icon.svelte';
	import {
		grantEligibility,
		revokeEligibility,
		roles,
		whatWentWrong,
		whichRoles
	} from './server.js';

	let { user } = $props();

	let page = $state(null);
	let allRoles = $state([]);
	let refusal = $state(null);
	let reading = $state(true);
	let granting = $state('');
	let confirming = $state(null);

	// The positions this person may not assume yet, picked from rather than rendered as a
	// row of yes and no against every role.
	const candidates = $derived(
		allRoles.filter((role) => !(page?.roles ?? []).some((open) => open.id === role.id))
	);

	$effect(() => {
		read(user);
	});

	async function read(id) {
		reading = true;
		await attempt(async () => {
			page = await whichRoles(id);
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

	async function grant(event) {
		event.preventDefault();
		const role = granting;
		granting = '';
		await attempt(() => grantEligibility(user, role));
		await read(user);
	}

	async function commit() {
		const { act } = confirming;
		confirming = null;
		await attempt(act);
		await read(user);
	}
</script>

<section>
	<!-- Above the heading, because the heading may never arrive: a link to a user somebody
	     deleted has no name to show, and the way back out has to be on screen anyway. -->
	<p class="back">
		<a href={resolve('/admin/users')}><Icon name="arrow-left" /> All users</a>
	</p>

	{#if reading}
		<p class="quiet">Reading…</p>
	{:else if !page}
		<!-- There is no such user, or the caller may not read them. Both are the server's own
		     sentence and the page is that sentence: an empty list would be the console
		     answering a question only the server can answer. -->
		<p class="refusal" role="alert">{refusal}</p>
	{:else}
		<header>
			<h2>{page.user.username}</h2>
			<p>
				Which positions this person may assume. A user carries eligibility and nothing else — no
				permissions of their own, and no exception anywhere. Every account starts eligible for <strong
					>Observer</strong
				>, and giving somebody one extra loop costs a role rather than a per-person grant.
			</p>
		</header>

		{#if refusal}
			<p class="refusal" role="alert">{refusal}</p>
		{/if}

		<form class="new" onsubmit={grant}>
			<select bind:value={granting} required aria-label="A role to make them eligible for">
				<option value="" disabled>Another role…</option>
				{#each candidates as role (role.id)}
					<option value={role.id}>{role.name}</option>
				{/each}
			</select>
			<button type="submit" disabled={granting === ''}>
				<Icon name="plus" /> Make eligible
			</button>
		</form>

		{#if page.roles.length === 0}
			<p class="quiet">
				This person may assume nothing. They can sign in, and the lobby has no seat to offer them.
			</p>
		{:else}
			<table>
				<thead>
					<tr>
						<th>Role</th>
						<th class="acts">Acts</th>
					</tr>
				</thead>
				<tbody>
					{#each page.roles as role (role.id)}
						<tr>
							<td>{role.name}</td>
							<td class="acts">
								<button
									class="destructive"
									onclick={() =>
										(confirming = {
											act: () => revokeEligibility(user, role.id),
											consequence: `${page.user.username} can no longer assume ${role.name}. If they are occupying it, their occupancy ends immediately and they are told why.`
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

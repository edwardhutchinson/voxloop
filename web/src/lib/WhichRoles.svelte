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
	import Confirm from './Confirm.svelte';
	import Icon from './Icon.svelte';
	import {
		grantEligibility,
		revokeEligibility,
		roles,
		whatWentWrong,
		whichRoles
	} from './server.js';

	let { account, onback } = $props();

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
		read(account.id);
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
		await attempt(() => grantEligibility(account.id, role));
		await read(account.id);
	}

	async function commit() {
		const { act } = confirming;
		confirming = null;
		await attempt(act);
		await read(account.id);
	}
</script>

<section>
	<header>
		<h2>{page?.user.username ?? account.username}</h2>
		<p>
			Which positions this person may assume. A user carries eligibility and nothing else — no
			permissions of their own, and no exception anywhere. Every account starts eligible for <strong
				>Observer</strong
			>, and giving somebody one extra loop costs a role rather than a per-person grant.
		</p>
	</header>

	<p class="back"><button onclick={onback}><Icon name="arrow-left" /> All users</button></p>

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
		<button type="submit" disabled={granting === ''}>Make eligible</button>
	</form>

	{#if reading}
		<p class="quiet">Reading…</p>
	{:else if page && page.roles.length === 0}
		<p class="quiet">
			This person may assume nothing. They can sign in, and the lobby has no seat to offer them.
		</p>
	{:else if page}
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
										act: () => revokeEligibility(account.id, role.id),
										consequence: `${page.user.username} can no longer assume ${role.name}. If they are occupying it, their occupancy ends immediately and they are told why.`
									})}>Revoke</button
							>
						</td>
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
	.back {
		margin: 0 0 var(--space-5);
	}
</style>

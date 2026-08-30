<script>
	import {
		Refused,
		createUser,
		deleteUser,
		editUser,
		forcePasswordReset,
		lockAccount,
		unlockAccount,
		users
	} from './server.js';

	let accounts = $state([]);
	let refusal = $state(null);
	let reading = $state(true);

	// Every act that touches a record is confirmed against what it will do, in words, before
	// it is committed (ADR-0015). The consequences a live deployment adds — who is cut
	// mid-word, whose subscriptions drop — arrive with the blast radius the server computes
	// once there are sessions for one of these to end.
	let confirming = $state(null);
	let creating = $state({ username: '', systemAdministration: false });
	let editing = $state(null);

	$effect(() => {
		read();
	});

	async function read() {
		reading = true;
		await attempt(async () => {
			accounts = await users();
		});
		reading = false;
	}

	async function attempt(what) {
		refusal = null;
		try {
			await what();
		} catch (said) {
			refusal = said instanceof Refused ? said.message : 'VoxLoop could not answer that.';
		}
	}

	function ask(account, act, consequence) {
		confirming = { account, act, consequence };
	}

	async function commit() {
		const { account, act } = confirming;
		confirming = null;
		await attempt(() => act(account.id));
		await read();
	}

	async function create(event) {
		event.preventDefault();
		await attempt(async () => {
			await createUser(creating.username, creating.systemAdministration);
			creating = { username: '', systemAdministration: false };
		});
		await read();
	}

	async function rename(event) {
		event.preventDefault();
		const { id, username } = editing;
		editing = null;
		await attempt(() => editUser(id, { username }));
		await read();
	}

	async function setFlag(account, held) {
		await attempt(() => editUser(account.id, { system_administration: held }));
		await read();
	}
</script>

<section>
	<header>
		<h2>Users</h2>
		<p>
			A user is created here and sets their own password from an enrolment code, because
			VoxLoop has no mail path.
		</p>
	</header>

	{#if refusal}
		<p class="refusal" role="alert">{refusal}</p>
	{/if}

	<form class="new" onsubmit={create}>
		<input bind:value={creating.username} placeholder="Username" required />
		<label>
			<input type="checkbox" bind:checked={creating.systemAdministration} />
			System administration
		</label>
		<button type="submit">Create</button>
	</form>

	{#if reading}
		<p class="quiet">Reading…</p>
	{:else}
		<table>
			<thead>
				<tr>
					<th>Username</th>
					<th>System administration</th>
					<th>Account</th>
					<th class="acts">Acts</th>
				</tr>
			</thead>
			<tbody>
				{#each accounts as account (account.id)}
					<tr>
						<td>
							{#if editing?.id === account.id}
								<form onsubmit={rename}>
									<!-- svelte-ignore a11y_autofocus -->
									<input bind:value={editing.username} autofocus required />
									<button type="submit">Rename</button>
									<button type="button" onclick={() => (editing = null)}>Cancel</button>
								</form>
							{:else}
								<button
									class="name"
									onclick={() => (editing = { id: account.id, username: account.username })}
								>
									{account.username}
								</button>
							{/if}
						</td>
						<td>
							<label>
								<input
									type="checkbox"
									checked={account.system_administration}
									onchange={(event) => setFlag(account, event.currentTarget.checked)}
								/>
								{account.system_administration ? 'held' : 'not held'}
							</label>
						</td>
						<td class:locked={account.locked}>{account.locked ? 'locked' : 'unlocked'}</td>
						<td class="acts">
							{#if account.locked}
								<button
									onclick={() =>
										ask(
											account,
											unlockAccount,
											`${account.username} will be able to sign in again.`
										)}>Unlock</button
								>
							{:else}
								<button
									onclick={() =>
										ask(
											account,
											lockAccount,
											`${account.username}'s sign-in and session end immediately, and they cannot sign in until the account is unlocked.`
										)}>Lock</button
								>
							{/if}
							<button
								onclick={() =>
									ask(
										account,
										forcePasswordReset,
										`${account.username}'s password is taken away and their sign-in and session end immediately. They cannot sign in until an enrolment code sets a new one.`
									)}>Force password reset</button
							>
							<button
								class="destructive"
								onclick={() =>
									ask(
										account,
										deleteUser,
										`${account.username} is deleted and signed out everywhere. Their audit entries stay, attributed.`
									)}>Delete</button
							>
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	{/if}

	{#if confirming}
		<div class="confirming" role="alertdialog">
			<p>{confirming.consequence}</p>
			<p class="quiet">Nothing live is affected: no session exists on this deployment yet.</p>
			<button onclick={commit}>Commit</button>
			<button onclick={() => (confirming = null)}>Cancel</button>
		</div>
	{/if}
</section>

<style>
	header p,
	.quiet {
		margin: 0.25rem 0 0;
		color: var(--quiet);
		font-size: 0.85rem;
	}

	h2 {
		margin: 0;
		font-size: 1.1rem;
	}

	.refusal {
		color: var(--refusal);
	}

	.new {
		display: flex;
		gap: 0.75rem;
		align-items: center;
		margin: 1.5rem 0;
		flex-wrap: wrap;
	}

	table {
		width: 100%;
		border-collapse: collapse;
	}

	th,
	td {
		text-align: left;
		padding: 0.5rem 0.75rem 0.5rem 0;
		border-bottom: 1px solid var(--rule);
		vertical-align: top;
	}

	th {
		font-size: 0.75rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--quiet);
	}

	.acts {
		text-align: right;
		white-space: nowrap;
	}

	.locked {
		color: var(--refusal);
	}

	.name {
		background: none;
		border: 0;
		padding: 0;
		color: inherit;
		font: inherit;
		text-decoration: underline dotted;
		cursor: pointer;
	}

	.destructive {
		color: var(--refusal);
	}

	.confirming {
		position: fixed;
		inset: auto 1.5rem 1.5rem auto;
		max-width: 28rem;
		padding: 1rem;
		background: var(--raised);
		border: 1px solid var(--rule);
		border-radius: 0.25rem;
	}

	.confirming p {
		margin: 0 0 0.75rem;
	}
</style>

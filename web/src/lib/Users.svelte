<script>
	// A user has one page under it: which roles they may assume. That is eligibility read from
	// the person's side, and it is one of the two directions it is administered from
	// (ADR-0015) — the other hangs off the role. A user carries eligibility and nothing else:
	// no permissions of their own, and no per-person exception anywhere.
	import Confirm from './Confirm.svelte';
	import WhichRoles from './WhichRoles.svelte';
	import {
		createUser,
		deleteUser,
		editUser,
		forcePasswordReset,
		issueEnrolmentCode,
		lockAccount,
		unlockAccount,
		users,
		whatWentWrong
	} from './server.js';

	let accounts = $state([]);
	// The account whose eligibility is being administered, where one is. *Admitting* rather
	// than *assuming*: assuming is the act by which a signed-in user takes up a role, and
	// nobody does that here.
	let admitting = $state(null);
	let refusal = $state(null);
	let reading = $state(true);

	// Every act with a consequence beyond the record itself is confirmed against what it will
	// do, in words, before it is committed (ADR-0015): locking, forcing a reset, deleting,
	// and taking the system-administration flag away. Creating a user and renaming one are
	// not confirmed, because neither ends anybody's sign-in or closes anybody's console.
	//
	// The consequences a live deployment adds — who is cut mid-word, whose subscriptions drop
	// — arrive as the blast radius the server computes once there are sessions for one of
	// these acts to end, and this panel is where they will be shown.
	let confirming = $state(null);
	let creating = $state({ username: '', systemAdministration: false });
	let editing = $state(null);

	// The one code the server ever hands back, held only until this page is left. Reading one
	// twice is what a single-use credential must not allow, so there is nowhere to read it
	// from afterwards — not the account list, not the audit log, not here.
	let issued = $state(null);

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
			refusal = whatWentWrong(said);
		}
	}

	function ask(account, act, consequence) {
		confirming = { account, act, consequence };
	}

	async function commit() {
		const { act } = confirming;
		confirming = null;
		await attempt(act);
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

	async function issue(account) {
		issued = null;
		confirming = null;
		await attempt(async () => {
			const code = await issueEnrolmentCode(account.id);
			issued = { username: account.username, ...code };
		});
		await read();
	}

	// When a code stops being good, in this browser's own idea of the time.
	function until(expiresAt) {
		return new Date(expiresAt).toLocaleString();
	}

	function setFlag(account, held) {
		const set = () => editUser(account.id, { system_administration: held });

		// Giving the flag takes nothing away, so it lands. Taking it away closes the console
		// on whoever held it, which is a consequence beyond the record.
		if (held) {
			attempt(set).then(read);
			return;
		}

		ask(
			account,
			set,
			`${account.username} loses the admin console. Nobody but a system administrator can give it back, and the last one cannot be taken away at all.`
		);
	}
</script>

{#if admitting}
	<WhichRoles account={admitting} onback={() => (admitting = null)} />
{:else}
	<section>
		<header>
			<h2>Users</h2>
			<p>
				A user is created here and sets their own password from an enrolment code, because
				VoxLoop has no mail path. A code is single-use, expiring, and handed over out of
				band; a reset is the same act again. <strong>Roles</strong> is which positions they
				may assume, which is the only authority a user carries.
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
						<th>Password</th>
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
								<!-- A button rather than a checkbox: taking the flag away is confirmed
								     first, and a checkbox left flipped while the record has not changed
								     would be the console asserting a state the server never agreed to. -->
								{account.system_administration ? 'held' : 'not held'}
								<button onclick={() => setFlag(account, !account.system_administration)}>
									{account.system_administration ? 'Take away' : 'Give'}
								</button>
							</td>
							<td class:locked={account.locked}>{account.locked ? 'locked' : 'unlocked'}</td>
							<td class:quiet={!account.enrolled}>
								{account.enrolled ? 'set' : 'awaiting enrolment'}
								{#if account.enrolment_expires_at !== null}
									<span class="outstanding">
										code outstanding until {until(account.enrolment_expires_at)}
									</span>
								{/if}
							</td>
							<td class="acts">
								<button
									onclick={() =>
										(admitting = { id: account.id, username: account.username })}>Roles</button
								>
								{#if account.locked}
									<button
										onclick={() =>
											ask(
												account,
												() => unlockAccount(account.id),
												`${account.username} will be able to sign in again.`
											)}>Unlock</button
									>
								{:else}
									<button
										onclick={() =>
											ask(
												account,
												() => lockAccount(account.id),
												`${account.username}'s sign-in and session end immediately, and they cannot sign in until the account is unlocked.`
											)}>Lock</button
									>
								{/if}
								<button onclick={() => issue(account)}>Issue enrolment code</button>
								<button
									onclick={() =>
										ask(
											account,
											() => forcePasswordReset(account.id),
											`${account.username}'s password is taken away and their sign-in and session end immediately. They cannot sign in until an enrolment code sets a new one.`
										)}>Force password reset</button
								>
								<button
									class="destructive"
									onclick={() =>
										ask(
											account,
											() => deleteUser(account.id),
											`${account.username} is deleted and signed out everywhere, and every role they were eligible for goes with them. Their audit entries stay, attributed.`
										)}>Delete</button
								>
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		{/if}

		{#if issued}
			<div class="issued" role="status">
				<p>
					An enrolment code for <strong>{issued.username}</strong>, good once, until
					{until(issued.expires_at)}.
				</p>
				<code>{issued.code}</code>
				<p class="quiet">
					Hand it over out of band — in person, or over the comms you already have. VoxLoop
					will not show it again, and issuing another invalidates this one.
				</p>
				<button onclick={() => (issued = null)}>Done</button>
			</div>
		{/if}

		{#if confirming}
			<Confirm
				consequence={confirming.consequence}
				oncommit={commit}
				oncancel={() => (confirming = null)}
			/>
		{/if}
	</section>
{/if}

<style>
	.locked {
		color: var(--refusal);
	}

	.outstanding {
		display: block;
		color: var(--quiet);
		font-size: 0.75rem;
	}

	.issued {
		position: fixed;
		inset: auto 1.5rem 1.5rem auto;
		max-width: 28rem;
		padding: 1rem;
		background: var(--raised);
		border: 1px solid var(--rule);
		border-radius: 0.25rem;
	}

	.issued p {
		margin: 0 0 0.75rem;
	}

	.issued code {
		display: block;
		margin-bottom: 0.75rem;
		padding: 0.5rem;
		background: var(--ground);
		border: 1px solid var(--rule);
		border-radius: 0.2rem;
		font-size: 0.95rem;
		word-break: break-all;
		user-select: all;
	}
</style>

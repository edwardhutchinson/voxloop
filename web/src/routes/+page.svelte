<script>
	// The console frame. There is no lobby yet (#36), so a signed-in user lands here and the
	// admin console is what they find — or a plain sentence saying it is not theirs.
	//
	// It opens on the system-administration flag **alone and never on a role** (v1 §9), which
	// is the whole reason this page asks the server who it is talking to rather than reading
	// anything out of the cookie: the cookie carries no claims, so the flag is whatever the
	// store says it is on this request.
	import AdminConsole from '$lib/AdminConsole.svelte';
	import SignIn from '$lib/SignIn.svelte';
	import { principal, signOut } from '$lib/server.js';

	let who = $state(null);
	let asked = $state(false);

	$effect(() => {
		ask();
	});

	async function ask() {
		try {
			who = await principal();
		} catch {
			who = null;
		}
		asked = true;
	}

	async function leave() {
		try {
			await signOut();
		} finally {
			who = null;
		}
	}
</script>

{#if !asked}
	<p class="quiet">Asking VoxLoop…</p>
{:else if !who}
	<SignIn onSignedIn={ask} />
{:else}
	<main>
		<header>
			<h1>VoxLoop</h1>
			<p>
				Signed in as <strong>{who.username}</strong>
				<button onclick={leave}>Sign out</button>
			</p>
		</header>

		{#if who.system_administration}
			<AdminConsole />
		{:else}
			<p class="refusal">
				You may not. The admin console is for a system administrator, and it is the flag
				rather than any role that opens it.
			</p>
		{/if}
	</main>
{/if}

<style>
	:global(:root) {
		--ground: #14161a;
		--raised: #1d2026;
		--ink: #e8eaed;
		--quiet: #9aa1ad;
		--rule: #2f343d;
		--refusal: #e8a0a0;

		color-scheme: dark;
		background: var(--ground);
		color: var(--ink);
		font: 15px/1.5 system-ui, sans-serif;
	}

	:global(body) {
		margin: 0;
	}

	:global(button) {
		font: inherit;
		color: var(--ink);
		background: var(--raised);
		border: 1px solid var(--rule);
		border-radius: 0.2rem;
		padding: 0.3rem 0.7rem;
		cursor: pointer;
	}

	:global(input) {
		font: inherit;
		color: var(--ink);
		background: var(--ground);
		border: 1px solid var(--rule);
		border-radius: 0.2rem;
		padding: 0.35rem 0.5rem;
	}

	main {
		max-width: 60rem;
		margin: 0 auto;
		padding: 2rem 1.5rem 6rem;
	}

	header {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 1rem;
		padding-bottom: 1rem;
		margin-bottom: 2rem;
		border-bottom: 1px solid var(--rule);
	}

	h1 {
		margin: 0;
		font-size: 1.25rem;
		letter-spacing: 0.02em;
	}

	header p {
		margin: 0;
		color: var(--quiet);
		font-size: 0.85rem;
		display: flex;
		align-items: center;
		gap: 0.75rem;
	}

	.quiet {
		color: var(--quiet);
		padding: 2rem;
	}

	.refusal {
		color: var(--refusal);
	}
</style>

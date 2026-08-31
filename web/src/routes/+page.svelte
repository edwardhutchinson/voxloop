<script>
	// The console frame. There is no lobby yet (#36), so a signed-in user lands here and the
	// admin console is what they find — or a plain sentence saying it is not theirs.
	//
	// It opens on the system-administration flag **alone and never on a role** (v1 §9), which
	// is the whole reason this page asks the server who it is talking to rather than reading
	// anything out of the cookie: the cookie carries no claims, so the flag is whatever the
	// store says it is on this request.
	import AdminConsole from '$lib/AdminConsole.svelte';
	import ChangePassword from '$lib/ChangePassword.svelte';
	import Enrol from '$lib/Enrol.svelte';
	import SignIn from '$lib/SignIn.svelte';
	import { principal, signOut } from '$lib/server.js';

	let who = $state(null);
	let asked = $state(false);
	// Redeeming a code and signing in are two public acts on the same page. Somebody holding
	// a code has no password yet and so cannot sign in at all, which is why this is a way in
	// rather than something reached from inside.
	let enrolling = $state(false);
	let enrolled = $state(false);

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
	{#if enrolling}
		<Enrol
			onEnrolled={() => {
				enrolling = false;
				enrolled = true;
			}}
			onBack={() => (enrolling = false)}
		/>
	{:else}
		<SignIn onSignedIn={ask} note={enrolled ? 'Password set. Sign in with it.' : null} />
		<p class="aside">
			<button class="plain" onclick={() => ((enrolling = true), (enrolled = false))}>
				I have an enrolment code
			</button>
		</p>
	{/if}
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
				You may not. The admin console is for a system administrator, and it is the flag rather than
				any role that opens it.
			</p>
		{/if}

		<ChangePassword />
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
		font:
			15px/1.5 system-ui,
			sans-serif;
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

	/* The furniture every admin console page shares. It is here rather than repeated in each
	   of them because they are three readings of one console, and a table that looked
	   different per page would say the pages were different kinds of thing. */
	:global(h2) {
		margin: 0;
		font-size: 1.1rem;
	}

	:global(header p),
	:global(.quiet) {
		margin: 0.25rem 0 0;
		color: var(--quiet);
		font-size: 0.85rem;
	}

	:global(.refusal) {
		color: var(--refusal);
	}

	:global(.new) {
		display: flex;
		gap: 0.75rem;
		align-items: center;
		margin: 1.5rem 0;
		flex-wrap: wrap;
	}

	:global(table) {
		width: 100%;
		border-collapse: collapse;
	}

	:global(th),
	:global(td) {
		text-align: left;
		padding: 0.5rem 0.75rem 0.5rem 0;
		border-bottom: 1px solid var(--rule);
		vertical-align: top;
	}

	:global(th) {
		font-size: 0.75rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--quiet);
	}

	:global(.acts) {
		text-align: right;
		white-space: nowrap;
	}

	/* A name is edited by clicking it, so it is a button that reads as text. */
	:global(.name) {
		background: none;
		border: 0;
		padding: 0;
		color: inherit;
		font: inherit;
		text-decoration: underline dotted;
		cursor: pointer;
	}

	:global(.destructive) {
		color: var(--refusal);
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

	.aside {
		max-width: 22rem;
		margin: -2.5rem auto 0;
		font-size: 0.85rem;
	}

	.plain {
		background: none;
		border: 0;
		padding: 0;
		color: var(--quiet);
		text-decoration: underline dotted;
	}
</style>

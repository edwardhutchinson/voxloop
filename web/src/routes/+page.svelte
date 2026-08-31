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
	main {
		max-width: 60rem;
		margin: 0 auto;
		padding: var(--space-6) var(--space-5) var(--space-page-bottom);
	}

	header {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: var(--space-4);
		padding-bottom: var(--space-4);
		margin-bottom: var(--space-6);
		border-bottom: 1px solid var(--rule);
	}

	h1 {
		margin: 0;
		font-size: var(--type-5);
		letter-spacing: 0.02em;
	}

	header p {
		margin: 0;
		color: var(--quiet);
		font-size: var(--type-2);
		display: flex;
		align-items: center;
		gap: var(--space-3);
	}

	.quiet {
		padding: var(--space-6);
	}

	/* The way in for somebody holding an enrolment code, under the sign-in form rather than
	   beside it: it is the rarer of the two acts and reads as the afterthought it is. */
	.aside {
		max-width: 22rem;
		margin: var(--space-3) auto 0;
		font-size: var(--type-2);
	}

	.plain {
		background: none;
		border: 0;
		padding: 0;
		color: var(--quiet);
		text-decoration: underline dotted;
	}
</style>

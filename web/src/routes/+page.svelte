<script>
	// The console frame. A signed-in user lands in the **lobby** — signed in, no role, no
	// audio, no authority (ADR-0023) — and that is what this page shows first.
	//
	// The admin console is reachable **from the lobby and from within a session**, gated on
	// the user's system-administration flag and never on a role (v1 §9): an operator who is
	// also a sysadmin must not have to drop off the air to add a loop. That is why this page
	// asks the server who it is talking to rather than reading anything out of the cookie —
	// the cookie carries no claims, so the flag is whatever the store says on this request.
	import AdminConsole from '$lib/AdminConsole.svelte';
	import ChangePassword from '$lib/ChangePassword.svelte';
	import Enrol from '$lib/Enrol.svelte';
	import Lobby from '$lib/Lobby.svelte';
	import SignIn from '$lib/SignIn.svelte';
	import { principal, signOut } from '$lib/server.js';
	import { openSignalling } from '$lib/session.js';

	let who = $state(null);
	let asked = $state(false);
	// Redeeming a code and signing in are two public acts on the same page. Somebody holding
	// a code has no password yet and so cannot sign in at all, which is why this is a way in
	// rather than something reached from inside.
	let enrolling = $state(false);
	let enrolled = $state(false);
	// The lobby, or the admin console. Two surfaces one person may hold at once, never a
	// role and never a mode: an administrator is in the lobby the whole time they are in
	// here.
	//
	// **It belongs to a sign-in and never outlives one.** Every sign-in starts in the lobby,
	// whoever the last one was: the admin console is where an administrator went, not where
	// the browser is, and a tab that came back to it for the next person would be showing a
	// surface that person may have no business on.
	let administering = $state(false);
	// What the server said on its way out, where it said anything: a sign-in that ended
	// somewhere else is a different fact from a tab that was closed, and the person in front
	// of this one is owed it.
	let ended = $state(null);
	// The lobby, as the signalling channel last had it.
	let lobby = $state(null);
	// The channel went away without saying why. What was last shown stays on screen and is
	// marked, rather than blanked: an empty page reads as *nothing is happening*, when in
	// fact anything may be happening and the console simply cannot see it (ADR-0018).
	let lost = $state(false);
	// The last thing the socket would not do, and why. It is not the end of anything, so it
	// is shown where it happened rather than taking the page away.
	let refused = $state(null);
	// Whether there is a sign-in to hold a socket open on — and nothing finer. Asking the
	// server again hands back a new answer object each time, and an effect watching *that*
	// would tear the socket down and open another one for a sign-in that never changed.
	const signedIn = $derived(who !== null);
	// Which surface is on screen, decided by the flag **as the server last reported it** and
	// not by the choice on its own. The console opens on the system-administration flag and
	// never on a role (v1 §9), and the flag is read from the store per request rather than
	// carried in the cookie — so the frame reads the same way, and a flag that has gone takes
	// the surface with it rather than leaving it up until something is clicked.
	const onTheAdminConsole = $derived(administering && who?.system_administration === true);

	$effect(() => {
		ask();
	});

	// **One socket per tab, opened at sign-in** (ADR-0054), and it belongs to the frame for
	// exactly that reason: it is open for as long as this person is signed in, whichever
	// surface they are reading. An administrator in the admin console has not left the lobby.
	$effect(() => {
		if (!signedIn) return;

		return openSignalling({
			onLobby: (said) => {
				lobby = said;
				lost = false;
				refused = null;
			},
			onRefused: (reason) => (refused = reason),
			onEnded: itEnded,
			onLost: () => (lost = true)
		});
	});

	async function ask() {
		try {
			who = await principal();
		} catch {
			who = null;
		}
		asked = true;
	}

	// The signalling channel said the sign-in is over. Asking again is what settles whether
	// it truly is, because the store is the only thing entitled to answer that.
	async function itEnded(reason) {
		ended = reason;
		administering = false;
		lobby = null;
		refused = null;
		await ask();
	}

	async function leave() {
		try {
			await signOut();
		} finally {
			who = null;
			administering = false;
			lobby = null;
			refused = null;
			ended = null;
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
		<SignIn onSignedIn={ask} note={ended ?? (enrolled ? 'Password set. Sign in with it.' : null)} />
		<p class="otherway">
			<button class="lesser" onclick={() => ((enrolling = true), (enrolled = false))}>
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
				{#if who.system_administration}
					<button onclick={() => (administering = !administering)}>
						{onTheAdminConsole ? 'Lobby' : 'Admin console'}
					</button>
				{/if}
				<button onclick={leave}>Sign out</button>
			</p>
		</header>

		{#if onTheAdminConsole}
			<AdminConsole />
		{:else}
			<Lobby {lobby} {lost} {refused} />
		{/if}

		<ChangePassword />
	</main>
{/if}

<style>
	main {
		/* A measure for the tables rather than for prose: the widest of them is the user list,
		   and past this the acts column ends up an eye's travel from the name it acts on. */
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
	   beside it: it is the rarer of the two acts and reads as the afterthought it is. The
	   measure is the form's own, so the two line up. */
	.otherway {
		max-width: 22rem;
		margin: var(--space-3) auto 0;
		font-size: var(--type-2);
	}
</style>

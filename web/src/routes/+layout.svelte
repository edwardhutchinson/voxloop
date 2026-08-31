<script>
	// The console frame, and every page of the console is rendered inside it.
	//
	// A signed-in user lands in the **lobby** — signed in, no role, no audio, no authority
	// (ADR-0023) — and the admin console is reachable from there and from within a session,
	// gated on the user's system-administration flag and never on a role (v1 §9): an operator
	// who is also a sysadmin must not have to drop off the air to add a loop. That is why this
	// asks the server who it is talking to rather than reading anything out of the cookie —
	// the cookie carries no claims, so the flag is whatever the store says on this request.
	//
	// **The frame is the layout rather than a page** because of the socket. One socket per
	// tab, opened at sign-in (ADR-0054), and the admin console's pages have URLs (#76): if the
	// socket belonged to a page, moving between those URLs would tear it down and open another
	// one, and once a resume is a thing that happens it would be a resume per navigation.
	// Everything that outlives a navigation lives here, and what is under it is the surface.
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';

	// The one place the token file is pulled in (ADR-0069). Every route is inside this, so a
	// component never imports a stylesheet and cannot forget to.
	import '../app.css';

	import ChangePassword from '$lib/ChangePassword.svelte';
	import Enrol from '$lib/Enrol.svelte';
	import SignIn from '$lib/SignIn.svelte';
	import { holdFrame } from '$lib/frame.js';
	import { principal, signOut } from '$lib/server.js';
	import { openSignalling } from '$lib/session.js';

	let { children } = $props();

	let asked = $state(false);
	// Redeeming a code and signing in are two public acts on the same page. Somebody holding
	// a code has no password yet and so cannot sign in at all, which is why this is a way in
	// rather than something reached from inside.
	let enrolling = $state(false);
	let enrolled = $state(false);
	// What the server said on its way out, where it said anything: a sign-in that ended
	// somewhere else is a different fact from a tab that was closed, and the person in front
	// of this one is owed it.
	let ended = $state(null);

	// Everything a surface under here reads. One object because it is one thing — what the
	// frame currently knows — and reactive because every field of it is an answer that can
	// arrive, change or be withdrawn while somebody is looking at it.
	const frame = $state({
		// Who the server says is signed in, and whether the admin console exists for them.
		who: null,
		// The lobby, as the signalling channel last had it.
		lobby: null,
		// The presence document, where this tab holds a session. **The server says which of
		// the two a person is looking at**, by sending one document or the other: whether
		// somebody holds a role is live state, never something the console decides for
		// itself (ADR-0016).
		presence: null,
		// Why the session ended, said once and shown in the lobby it lands back in. Audio
		// genuinely stopped, so a console that merely reappeared in the lobby would be
		// leaving the operator to work out what happened (v1 §2).
		relinquished: null,
		// The channel went away without saying why. What was last shown stays on screen and
		// is marked, rather than blanked: an empty page reads as *nothing is happening*, when
		// in fact anything may be happening and the console simply cannot see it (ADR-0018).
		lost: false,
		// The last thing the socket would not do, and why. It is not the end of anything, so
		// it is shown where it happened rather than taking the page away.
		refused: null,
		// The two acts a tab performs on its own session. They are here because the socket is
		// here: a page under the frame asks the frame, and never opens a second channel.
		assume: () => {},
		relinquish: () => {}
	});

	holdFrame(frame);

	// Whether there is a sign-in to hold a socket open on — and nothing finer. Asking the
	// server again hands back a new answer object each time, and an effect watching *that*
	// would tear the socket down and open another one for a sign-in that never changed.
	const signedIn = $derived(frame.who !== null);
	// The admin console is a place, so which surface is on screen is now the URL's answer
	// rather than a variable's. Whether the person may be there is still the server's, and it
	// is asked one level down, on the admin console's own layout.
	const administering = $derived(page.url.pathname.startsWith('/admin'));

	$effect(() => {
		ask();
	});

	// **One socket per tab, opened at sign-in** (ADR-0054), and it belongs to the frame for
	// exactly that reason: it is open for as long as this person is signed in, whichever
	// surface they are reading and whichever URL they are at. An administrator in the admin
	// console has not left the lobby.
	$effect(() => {
		if (!signedIn) return;

		const channel = openSignalling({
			onLobby: (said) => {
				frame.lobby = said;
				// The lobby is where a session ends up, so arriving at it clears the session
				// rather than leaving two documents on screen describing two different states.
				frame.presence = null;
				frame.lost = false;
				frame.refused = null;
			},
			onPresence: (said) => {
				frame.presence = said;
				// A role taken up is the answer to whatever the lobby was refusing, and it is
				// the end of whatever ended before it.
				frame.relinquished = null;
				frame.lost = false;
				frame.refused = null;
			},
			onSessionEnded: (reason) => {
				frame.presence = null;
				frame.relinquished = reason;
			},
			onRefused: (reason) => (frame.refused = reason),
			onEnded: itEnded,
			onLost: () => (frame.lost = true)
		});

		frame.assume = (role) => {
			frame.refused = null;
			frame.relinquished = null;
			channel.assume(role);
		};
		frame.relinquish = channel.relinquish;

		return () => {
			frame.assume = () => {};
			frame.relinquish = () => {};
			channel.close();
		};
	});

	async function ask() {
		try {
			frame.who = await principal();
		} catch {
			frame.who = null;
		}
		asked = true;
	}

	// **A sign-in never inherits the last one's surface.** The admin console is where an
	// administrator went, not where the browser is, so a tab whose sign-in has ended goes back
	// to the frame's own page rather than leaving an admin URL standing for whoever signs in
	// next. Arriving at one of those URLs cold is a different act and lands where it was
	// pointed, once the server has said who is asking.
	function toTheTop() {
		if (page.url.pathname !== '/') goto(resolve('/'));
	}

	// The signalling channel said the sign-in is over. Asking again is what settles whether
	// it truly is, because the store is the only thing entitled to answer that.
	async function itEnded(reason) {
		ended = reason;
		frame.lobby = null;
		frame.presence = null;
		frame.relinquished = null;
		frame.refused = null;
		toTheTop();
		await ask();
	}

	async function leave() {
		try {
			await signOut();
		} finally {
			frame.who = null;
			frame.lobby = null;
			frame.presence = null;
			frame.relinquished = null;
			frame.refused = null;
			ended = null;
			toTheTop();
		}
	}
</script>

{#if !asked}
	<p class="quiet">Asking VoxLoop…</p>
{:else if !frame.who}
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
				Signed in as <strong>{frame.who.username}</strong>
				{#if frame.who.system_administration}
					<!-- A link rather than a toggle, because one of these two is somewhere with a
					     URL and the other is not: the admin console is a place, and the lobby is
					     whatever the server says about this session. -->
					<a href={administering ? resolve('/') : resolve('/admin')}>
						{administering ? 'Lobby' : 'Admin console'}
					</a>
				{/if}
				<button onclick={leave}>Sign out</button>
			</p>
		</header>

		{@render children()}

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

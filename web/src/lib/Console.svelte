<script>
	// The operating console: what somebody who has assumed a role is looking at.
	//
	// It renders the **presence document** and nothing else. The document is the API
	// (ADR-0019): whatever is on this page came out of it, and anything in it is something
	// the server has committed to keeping true — so this page never computes a state, never
	// merges one document into another, and never renders optimistically.
	//
	// **Two views of one loop list** (ADR-0032), both complete, both driven by that one
	// document, so they cannot disagree about anything except layout. The **board** is the
	// glanceable view and the **ledger** is the reading view; which of them is on screen is
	// the only thing this page remembers, because it is a fact about the reader rather than
	// about the world. Every other fact on it is the server's, and arrives again on the next
	// tick.
	//
	// **Changing role is a relinquish followed by an assume** (v1 §2), so there is no role
	// picker here and no *switch*. Relinquishing lands in the lobby, and the lobby is where a
	// role is taken up. Audio genuinely stops in between, and offering a control that hid
	// that would be the class of lie this product exists to avoid.
	//
	// **Input is a seam and everything here goes through its interface** (ADR-0021, ADR-0061).
	// This page reads one answer — *is this session keying* — and never asks a source anything;
	// which key was pressed, and whether holding it or pressing it once is what talks, are
	// `modes.js`'s and the seam's. The only other thing that reaches Input at all is the list
	// of keys below, which asks it what a binding is called and what may be bound. That is
	// what lets the Tauri wrapper add a native hotkey and change nothing here (ADR-0020).
	//
	// **Nothing here lights the transmitting lamp.** Intent goes down: the local track is
	// keyed and the server is told, in that order, because that order is what buys
	// key-to-first-audio under 100 ms (ADR-0008). The lamp comes back up in the presence
	// document, like every other state on this page.
	import Bindings from './Bindings.svelte';
	import Board from './Board.svelte';
	import Ledger from './Ledger.svelte';
	import { keyingModes, LATCH, modes, MOMENTARY } from './modes.js';

	let {
		presence,
		lost,
		refused,
		onRelinquish,
		onSubscribe,
		onUnsubscribe,
		onArm,
		onDisarm,
		onKeying
	} = $props();

	// **Whether the key is latched open, and the source that went while it was held.** Both are
	// facts about the input on this desk rather than state the server has committed to
	// (ADR-0016) — knowable here, and true the moment they are said. Nothing about the *world*
	// is held on this page: that all arrives in the presence document, and the bar renders
	// these two as the local assertions they are rather than beside the lamp.
	let latched = $state(false);
	let dropped = $state(null);

	// One reading of the modes for the life of this console. **The console ORs nothing and
	// times nothing** — the OR is the seam's and the modes are `modes.js`'s, and doing either
	// here is how a latch ends up derived from a press (ADR-0022).
	const keys = keyingModes({
		onKeying: (wants) => {
			// A key going down is the answer to whatever the last one dropped, so the notice
			// goes when the operator keys again rather than sitting under a live transmission.
			if (wants) dropped = null;
			onKeying(wants);
		},
		onLatched: (is) => (latched = is),
		onDropped: (source) => (dropped = source)
	});

	// The listeners go when this page does. A role given up is not a role anybody can key, and
	// a keyboard binding that outlived the console would be exactly the source ADR-0022 says
	// must be inert outside an assumed role.
	$effect(() => () => keys.stop());

	// The keys as they stand, mirrored so that changing one redraws the list. #55 is what makes
	// a change outlive the console; until then this is the whole of where one lives.
	let bound = $state(keys.bound());

	// The board is what a control room reads at a glance, so it is what a console opens on.
	// Which view somebody lands in becomes theirs — personalisation per (user, role), from a
	// role default — with #55.
	let showing = $state('board');

	// **One order, and both views are handed it.** Reordering it reorders both, because there
	// is only one of it: two independent orders would put the same loop third in one view and
	// eleventh in the other, which is the quiet kind of disagreement that teaches an operator
	// to distrust the console (ADR-0032). It is the administered base order the document
	// arrives in (ADR-0053), and this line is the one #55 changes to make it personal.
	const inOrder = $derived(presence.loops);

	// **The armed set in words, worked out once and handed to both views** (ADR-0034). Two
	// views computing it separately is exactly how a board and a ledger come to disagree about
	// where somebody's voice is going, which is the one thing the transmit bar may not do.
	const armedOn = $derived(
		inOrder.filter((reachable) => reachable.armed).map((reachable) => reachable.name)
	);

	// **Whether emission stands at all**, decided once here rather than in the bar, because
	// two things read it: the bar, which draws the key control, and Input, which is told
	// whether that control is on screen. `impaired` is a transient fault that routinely clears
	// itself and emission stands through it; `lost` is where emission is withdrawn
	// (ADR-0042). Anything the console has no reading of is read as `lost`, which is the safe
	// direction — a console that cannot tell what the audio path is doing has no business
	// offering a key control over it. The rest of the emission predicate is #43's.
	const mayKey = $derived(
		presence.media_path === 'connected' || presence.media_path === 'impaired'
	);

	// **Every source dies together, because they die of the same thing** (ADR-0021). A source
	// that dies while keyed forces an unkey: a key control that vanished under a held pointer
	// delivers no release, and a keyboard binding that went inert under a held key delivers no
	// release either, so without this the level would stay high, the microphone would stay
	// open, and the server would go on telling everybody a session with no audio path was
	// transmitting. It also drops a latch, because key state never returns across a withdrawal
	// (v1 §7).
	$effect(() => {
		keys.available(mayKey);
	});

	// **Clicking a loop toggles monitoring**, and the toggle is decided here rather than in
	// either view. It is two acts on the wire — subscribe and unsubscribe — and which one a
	// click is comes from the document, which is the only thing that knows: the views are
	// handed a click and say which loop it was on (v1 §8, ADR-0032).
	//
	// **There is no confirmation.** Optimistic rendering is banned (ADR-0016), so the card
	// visibly lags the click, and a misclick on a loop the operator staffs announces itself
	// by dropping it to `away` for everyone — which is the safety argument for making it one
	// click rather than two.
	function toggle(reachable) {
		if (reachable.subscribed) onUnsubscribe(reachable.id);
		else onSubscribe(reachable.id);
	}

	// **Arming is the same two-acts-not-a-toggle shape**, decided here for the same reason:
	// the document is the only thing that knows which of the two a press is, and the views
	// say which loop was pressed. It is a separate act from monitoring in both directions
	// (ADR-0013) and shares nothing with it but this shape.
	function arming(reachable) {
		if (reachable.armed) onDisarm(reachable.id);
		else onArm(reachable.id);
	}
</script>

<section>
	<header>
		<h2>{presence.role.name}</h2>
		<p>
			You have assumed this role and hold its authority. To take up another, relinquish this one
			first — audio stops, and your subscriptions and arms go with it.
		</p>
		<p class="quiet">
			<!-- Said once, above both views, because it is a fact about the loop list rather
			     than about either rendering of it. It is here for the lag rather than for the
			     gesture: nothing renders optimistically (ADR-0016), so a loop changes a round
			     trip after the click, and an operator who has not been told reads that as a
			     console that missed one. -->
			A loop changes when VoxLoop confirms it, not when you click it.
		</p>
	</header>

	{#if lost}
		<p class="lost" role="alert">
			The connection to VoxLoop was lost. This is what it last said, and it is not being kept up to
			date.
		</p>
	{/if}

	{#if refused}
		<p class="refusal" role="alert">{refused}</p>
	{/if}

	{#if inOrder.length === 0}
		<!-- A fact about reach rather than about either view, so it is said here and once. The
		     view still renders, because an empty reach is a console with no loops on it rather
		     than a console that is not there. -->
		<p class="quiet">
			This role reaches no loops. Reach is one cell on the grid per loop, set by a system
			administrator, and a role may be assumed with an empty row.
		</p>
	{/if}

	<div class="views" role="group" aria-label="How the loops are shown">
		<button aria-pressed={showing === 'board'} onclick={() => (showing = 'board')}>Board</button>
		<button aria-pressed={showing === 'ledger'} onclick={() => (showing = 'ledger')}>Ledger</button>
	</div>

	{#if showing === 'board'}
		<Board
			loops={inOrder}
			mediaPath={presence.media_path}
			{armedOn}
			{mayKey}
			{latched}
			{dropped}
			keyed={presence.keyed}
			onToggle={toggle}
			onArm={arming}
			onKeyDown={keys.controls[MOMENTARY].down}
			onKeyUp={keys.controls[MOMENTARY].up}
			onLatchDown={keys.controls[LATCH].down}
			onLatchUp={keys.controls[LATCH].up}
		/>
	{:else}
		<Ledger
			loops={inOrder}
			mediaPath={presence.media_path}
			{armedOn}
			{mayKey}
			{latched}
			{dropped}
			keyed={presence.keyed}
			onToggle={toggle}
			onArm={arming}
			onKeyDown={keys.controls[MOMENTARY].down}
			onKeyUp={keys.controls[MOMENTARY].up}
			onLatchDown={keys.controls[LATCH].down}
			onLatchUp={keys.controls[LATCH].up}
		/>
	{/if}

	<!-- Under the loops rather than among them: the keys are a setting, and a setting beside
	     the thing it is about would be one more thing on a page an operator reads at a glance.
	     What is on it is above; this is where somebody goes to change how they get there. -->
	<div class="keys">
		<Bindings
			{modes}
			{bound}
			onRebind={(named, binding) => {
				const no = keys.rebind(named, binding);
				if (!no) bound = keys.bound();
				return no;
			}}
		/>
	</div>

	<p class="relinquish">
		<button class="destructive" onclick={onRelinquish}>Relinquish {presence.role.name}</button>
	</p>
</section>

<style>
	/* Directly above the list it governs, rather than in the header: it is a control over what
	   is under it and not a fact about the role. */
	.views {
		display: flex;
		gap: var(--space-1);
		margin: var(--space-5) 0 var(--space-4);
	}

	/* Which view is showing is carried by the view: a field of cards and a table are not
	   mistakable for each other, so the mark on the button is a reminder rather than the state
	   itself, and `aria-pressed` is what says it to a screen reader. */
	.views button[aria-pressed='true'] {
		border-color: var(--ink);
	}

	/* Below the loops rather than beside the heading. Relinquishing is a full stop and the
	   one act on this page, and putting it in the header would place the way off the air
	   next to the name of the role somebody just took. */
	.keys {
		margin: var(--space-6) 0 0;
		padding-top: var(--space-4);
		border-top: 1px solid var(--rule);
	}

	.relinquish {
		margin: var(--space-5) 0 0;
	}
</style>

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
	import LoopVolume from './LoopVolume.svelte';
	import { CONFIRMED, DISCONNECTED, UNCONFIRMED, worse } from './session.js';
	import { keyingModes, LATCHED, modes, MOMENTARY, PRIORITY } from './modes.js';

	// `connection` is where this tab stands with the signalling channel, measured here rather
	// than pushed (ADR-0018) — the one state on this page the server did not say, because the
	// one thing a server cannot do to a console it has lost is tell it that it has been lost.
	let {
		presence,
		connection,
		refused,
		onRelinquish,
		onSubscribe,
		onUnsubscribe,
		onArm,
		onDisarm,
		onMute,
		onUnmute,
		onSetVolume,
		onKeying,
		onPriority
	} = $props();

	// **Whether the key is latched open, and the source that went while it was held.** Both are
	// facts about the input on this desk rather than state the server has committed to
	// (ADR-0016) — knowable here, and true the moment they are said. Nothing about the *world*
	// is held on this page: that all arrives in the presence document, and the bar renders
	// these two as the local assertions they are rather than beside the lamp.
	let latched = $state(false);
	let dropped = $state(null);
	// **The one user-facing message in the product that does not originate at the server**
	// (ADR-0018). A latch taken down by the network is the single case where VoxLoop cuts
	// audio it cannot announce, so the announcement that *can* still be made is made: this
	// console tells its own operator. An operator who believes they are still transmitting is
	// the failure the rule was written to remove, arriving through the other door.
	let latchDropped = $state(false);

	// One reading of the modes for the life of this console. **The console ORs nothing and
	// times nothing** — the OR is the seam's and the modes are `modes.js`'s, and doing either
	// here is how a latch ends up derived from a press (ADR-0022).
	const keys = keyingModes({
		onKeying: (wants) => {
			// A key going down is the answer to whatever the last one dropped, so the notice
			// goes when the operator keys again rather than sitting under a live transmission.
			if (wants) {
				dropped = null;
				latchDropped = false;
			}
			onKeying(wants);
		},
		// **Priority goes down as its own level** (ADR-0046), after the key on the way up and
		// before it on the way down. The server marks the loops and audits the press; nothing
		// here draws it, because whether a transmission is at priority is the document's.
		onPriority,
		onLatched: (is) => (latched = is),
		onDropped: (source) => (dropped = source),
		onLatchDropped: () => (latchDropped = true)
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

	// **Which loop's volume the operator has opened, by id** — a fact about the reader, like
	// which view is showing, and nothing the server has to say. The loop itself is read out of
	// the document every time rather than kept, so the modal shows the level VoxLoop last
	// confirmed, and a loop that leaves reach while its modal is open takes the modal with it.
	let volumeOpenFor = $state(null);
	const volumeOf = $derived(presence.loops.find((reachable) => reachable.id === volumeOpenFor));

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

	// **Everything the transmit bar renders, as one value.** It is one component so that it is
	// one wording (ADR-0034), and it is one prop for the same reason a rung further down: the
	// views carry the bar and place it, and neither of them has a name for anything inside it,
	// so neither can forward half of it or word a word of it. It is also what keeps the next
	// state the bar gains off both views entirely.
	const bar = $derived({
		mediaPath: presence.media_path,
		connection: standing,
		armedOn,
		keyed: presence.keyed,
		priority: presence.priority,
		mayKey,
		latched,
		dropped,
		latchDropped,
		onDown: keys.onScreen[MOMENTARY].down,
		onUp: keys.onScreen[MOMENTARY].up,
		onLatchDown: keys.onScreen[LATCHED].down,
		onLatchUp: keys.onScreen[LATCHED].up,
		onPriorityDown: keys.onScreen[PRIORITY].down,
		onPriorityUp: keys.onScreen[PRIORITY].up
	});

	// **Where the channel stands, from both ends, merged pessimistically** — green needs both,
	// red needs one (ADR-0018, and the rule ADR-0042 already applies to the media path). This
	// tab measures the heartbeats it is not getting; the server measures the answers it is not
	// getting and says so in the document. They are two different silences and they can
	// honestly disagree, and the disagreement that matters is a console whose answers are
	// being lost while the server's heartbeats still arrive: the server has closed its fan-out
	// and nothing this tab could measure would say so.
	const standing = $derived(worse(connection.state, presence.connection));

	// **Whose reading it is**, because the two failures want different sentences: *VoxLoop
	// cannot be reached* and *VoxLoop is not hearing this console* send an operator to look at
	// different things, and only the first of them is a console that has gone blind.
	const heardFromVoxLoop = $derived(connection.state === CONFIRMED);

	// How long ago this tab last heard VoxLoop, in whole seconds. **The running age is what
	// stops a frozen console being mistaken for a live one** (ADR-0018): last-known state is
	// not blanked, because an empty page reads as *nothing is happening* when everything may
	// be — so what makes it honest is the number beside it moving. It is this tab's own clock,
	// so it is said only where this tab's own clock is what is reporting.
	const staleFor = $derived(Math.floor(connection.since / 1000));

	// **Whether emission stands at all**, decided once here rather than in the bar, because
	// two things read it: the bar, which draws the key control, and Input, which is told
	// whether that control is on screen.
	//
	// **Two independent withdrawal conditions, and both of them are here** (ADR-0018,
	// ADR-0042). The audio path answers *can anybody hear me*: `impaired` is a transient fault
	// that routinely clears itself and emission stands through it, and `lost` is where it is
	// withdrawn. The state channel answers *can anybody be told what I am doing*: at
	// `disconnected` no listener's console would show this session talking, no loop would
	// attribute it and no authority holder could cut it — the audio would arrive and the
	// accountability would not.
	//
	// They are kept as two answers rather than folded into one because the bar has to say
	// **which** applies: they are different problems with different fixes, and one wording for
	// both sends an operator to look at the wrong thing.
	//
	// Anything the console has no reading of is read as withdrawn, which is the safe direction
	// — a console that cannot tell what its own paths are doing has no business offering a key
	// control over them.
	const anAudioPath = $derived(
		presence.media_path === 'connected' || presence.media_path === 'impaired'
	);
	const aStateChannel = $derived(standing !== DISCONNECTED);
	const mayKey = $derived(anAudioPath && aStateChannel);

	// **Every source dies together, because they die of the same thing** (ADR-0021). A source
	// that dies while keyed forces an unkey: a key control that vanished under a held pointer
	// delivers no release, and a keyboard binding that went inert under a held key delivers no
	// release either, so without this the level would stay high, the microphone would stay
	// open, and the server would go on telling everybody a session with no audio path was
	// transmitting. It also drops a latch, because key state never returns across a withdrawal
	// (v1 §7).
	//
	// **The microphone's liveness is not this** and the two are never folded into one
	// (ADR-0021). A microphone that is unplugged is Audio's to notice, and it arrives here as
	// the media path; whether a key is being held is Input's, and it arrives as `dropped`. The
	// bar says both, in that order, because *there is no audio path* and *your hand is still
	// down* are two facts with two fixes — and a headset with an inline button produces both
	// at once, which is the case one signal could not report.
	$effect(() => {
		keys.available(mayKey);
	});

	// **A latched emission is dropped after a couple of seconds of `unconfirmed`, while a
	// momentary key survives** (ADR-0018). The asymmetry is the whole rule: a held button is a
	// human continuously asserting intent, and a latch is an assertion made once, possibly
	// minutes ago, whose entire safety story is that this console will show it to you. That
	// story is void the moment this console cannot be trusted, so the latch dies well before
	// emission itself is withdrawn — and the operator is told, locally, because there is
	// nobody left to tell them.
	$effect(() => {
		if (!connection.aLatchStands) keys.theLatchCannotBeShown();
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

	// **Muting is the same shape again**, and it is not an unsubscribe: the loop stays
	// monitored, so its talking indicator keeps arriving, and nobody else on it is touched
	// (v1 §5). The document is what says which of the two a press is.
	function muting(reachable) {
		if (reachable.muted) onUnmute(reachable.id);
		else onMute(reachable.id);
	}

	// The cog opens the volume for one loop. **It is the only way to a volume** (v1 §8): per-
	// loop volume is personalisation rather than a live operational control, and nothing on
	// the main surface may nudge it.
	function openTheVolume(reachable) {
		volumeOpenFor = reachable.id;
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

	<!-- **The console's own state, marked stale rather than blanked** (ADR-0018). Blanking
	     was rejected as its own lie: an empty console implies *nothing is happening*, when in
	     fact everything may be happening and this tab simply cannot see it. Blocking the page
	     was rejected too, because it disarms an operator at the exact moment things are going
	     wrong. So the loops stay where they are, under a sentence saying how old they are.

	     **The freeze needs no code and has none.** Documents and heartbeats travel on one
	     socket and heartbeats are the more frequent of the two, so a document arriving means a
	     heartbeat arrived more recently still — this console cannot be reading a rung above
	     `confirmed` and taking in fresh state at the same time. What was needed was the mark,
	     and that is what this is.

	     What each rung costs the *transmit bar* is said there, beside the key control, because
	     that is the half an operator acts on and it is the strip both views carry. -->
	{#if standing === DISCONNECTED && heardFromVoxLoop}
		<!-- The half of the failure this console cannot see for itself: VoxLoop is still
		     reaching it, so what is on screen is current — and VoxLoop is not hearing its
		     answers, so it has closed the fan-out and nothing this tab could measure would
		     have said so. It is a different problem from the one below and gets a different
		     sentence, because *your console is blind* and *your console is unheard* send an
		     operator to look at different things. -->
		<p class="lost" role="alert">
			VoxLoop is not hearing this console, so it will not emit. What is on screen is current; what
			this console sends is not arriving.
		</p>
	{:else if standing === DISCONNECTED}
		<p class="lost" role="alert">
			The connection to VoxLoop was lost {staleFor} s ago. This is what it last said, and it is not being
			kept up to date.
		</p>
	{:else if standing === UNCONFIRMED}
		<p class="stale" role="status">
			VoxLoop was last confirmed {staleFor} s ago. This is what it last said, and it is not being kept
			up to date.
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
			{bar}
			onToggle={toggle}
			onArm={arming}
			onMute={muting}
			onCog={openTheVolume}
		/>
	{:else}
		<Ledger
			loops={inOrder}
			{bar}
			onToggle={toggle}
			onArm={arming}
			onMute={muting}
			onCog={openTheVolume}
		/>
	{/if}

	<!-- One modal, above both views rather than inside either, so a cog pressed on the board and
	     the same cog pressed in the ledger open the same thing. -->
	{#if volumeOf}
		<LoopVolume
			loop={volumeOf}
			onSet={(volume) => onSetVolume(volumeOf.id, volume)}
			onClose={() => (volumeOpenFor = null)}
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

	/* Set off from the loops by a rule and the largest gap on the page, because it is the one
	   thing here that is not the shift: everything above it changes minute to minute, and this
	   is a setting somebody visits once. The gap is what stops it being read as another state
	   of the console. */
	.keys {
		margin: var(--space-6) 0 0;
		padding-top: var(--space-4);
		border-top: 1px solid var(--rule);
	}

	/* Below the loops rather than beside the heading. Relinquishing is a full stop and the
	   one act on this page, and putting it in the header would place the way off the air
	   next to the name of the role somebody just took. */
	.relinquish {
		margin: var(--space-5) 0 0;
	}
</style>

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
	import Board from './Board.svelte';
	import Ledger from './Ledger.svelte';

	let { presence, lost, refused, onRelinquish, onSubscribe, onUnsubscribe } = $props();

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
			     than about either rendering of it. The lag is the design (ADR-0016) and an
			     operator who has not been told about it reads it as a console that missed a
			     click. -->
			Clicking a loop starts or stops monitoring it. There is no confirmation, and the loop changes when
			VoxLoop says it has rather than when you click.
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
		<Board loops={inOrder} mediaPath={presence.media_path} onToggle={toggle} />
	{:else}
		<Ledger loops={inOrder} mediaPath={presence.media_path} onToggle={toggle} />
	{/if}

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
	.relinquish {
		margin: var(--space-5) 0 0;
	}
</style>

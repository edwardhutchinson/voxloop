<script>
	// The board: a card per loop in reach, and the glanceable view (ADR-0032). It is what a
	// control room reads at a glance, and it is the one the operator asked for.
	//
	// **A card cannot hold a sentence.** Anything the model requires has to fit here as a
	// word, and may be a sentence only in the ledger — which is the whole division of labour
	// that justifies keeping both views, and the standing constraint on every state added
	// from here (v1 §8). The one state a card carries today is the rung this role holds, as
	// the word the grid uses for it; `Ledger.svelte` is where it is spelled out.
	//
	// **It renders the loops it is handed, in the order it is handed them.** The order is one
	// thing held above both views, so there is nothing to sort here and nowhere for a second
	// order to live.
	//
	// Clicking the body to toggle monitoring is #39, the staffing marks are #48, and the mute
	// and cog controls that must not propagate that click are #44. A card holds almost
	// nothing at this point, and both views ship at that point on purpose: every later ticket
	// then pays the two-view cost as it goes.
	import TransmitBar from './TransmitBar.svelte';

	let { loops, mediaPath } = $props();
</script>

<ul class="board">
	{#each loops as reachable (reachable.id)}
		<li>
			<h3>{reachable.name}</h3>
			<p class="quiet">{reachable.permission}</p>
		</li>
	{/each}
</ul>

<!-- Never scrolled away (ADR-0034). On the board it closes the field along the bottom edge:
     the cards are scanned rather than read, and the bar is the one thing on the page that is
     not a loop. The page's own `--space-page-bottom` is what keeps the last row clear of it. -->
<div class="transmit">
	<TransmitBar {mediaPath} />
</div>

<style>
	.board {
		display: grid;
		/* Cards keep one width and the row holds as many as fit, rather than stretching to
		   fill: an operator learns where a loop is by where it sits, and a list that reflows
		   into two columns when one loop enters reach has moved every card in it. The width is
		   a loop name on one line — `Air-to-ground` and `Flight Director` are the long ones a
		   pilot deployment writes — because a name wrapping is what makes a card unreadable at
		   a glance, and the card is the glanceable view. */
		grid-template-columns: repeat(auto-fill, minmax(11rem, 1fr));
		gap: var(--space-3);
		margin: 0;
		padding: 0;
		list-style: none;
	}

	li {
		background: var(--raised);
		border: 1px solid var(--rule);
		border-radius: var(--radius);
		padding: var(--space-3);
	}

	h3 {
		margin: 0;
		font-size: var(--type-3);
	}

	/* The ground is set on whichever element does the pinning, here and in the ledger, because
	   that is the element the loops pass under: a bar drawing its own background would leave
	   the padding around it transparent and the cards would show through the gap. */
	.transmit {
		position: fixed;
		inset: auto 0 0;
		background: var(--ground);
		border-top: 1px solid var(--rule);
		padding: var(--space-3) var(--space-5);
	}
</style>

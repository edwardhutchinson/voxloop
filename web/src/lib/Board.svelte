<script>
	// The board: a card per loop in reach, and the glanceable view (ADR-0032). It is what a
	// control room reads at a glance, and it is the one the operator asked for.
	//
	// **A card cannot hold a sentence.** Anything the model requires has to fit here as a
	// word, and may be a sentence only in the ledger — which is the whole division of labour
	// that justifies keeping both views, and the standing constraint on every state added
	// from here (v1 §8). The two states a card carries today are the rung this role holds and
	// whether the loop is being monitored, both as words; `Ledger.svelte` spells them out.
	//
	// **It renders the loops it is handed, in the order it is handed them.** The order is one
	// thing held above both views, so there is nothing to sort here and nowhere for a second
	// order to live.
	//
	// **The card body is a button and the card is not.** Clicking the body toggles monitoring
	// (v1 §8), and the arm, mute and cog controls that arrive with #41 and #44 must not
	// propagate that click — so the clickable region is one element inside the card rather
	// than the card itself, and it is a `<button>`, which cannot contain another one. A
	// control added to this card is a sibling of the body by construction, and there is no
	// `stopPropagation` for anybody to forget.
	//
	// **The loop name is not a heading**, and that changed when the body became a control: a
	// heading announces a section of content, and a card is a control. The `<ul>` is what
	// carries the structure — a list of twenty toggle buttons, each named for its loop and
	// its state — and a heading inside a button is not valid markup anyway.
	//
	// The staffing marks are #48. Nothing here decides which act a click is: it says which
	// loop was clicked, and `Console.svelte` reads the document to know the rest.
	import TransmitBar from './TransmitBar.svelte';

	let { loops, mediaPath, onToggle } = $props();
</script>

<ul class="board">
	{#each loops as reachable (reachable.id)}
		<li>
			<button class="body" aria-pressed={reachable.subscribed} onclick={() => onToggle(reachable)}>
				<span class="loop">{reachable.name}</span>
				<!-- The state in a word, never in the border alone: colour is never the only
				     thing carrying a state, and a card is read at a glance by somebody who may
				     not be looking at another card to compare it with. -->
				<span class="monitoring">{reachable.subscribed ? 'Monitoring' : 'Not monitoring'}</span>
				<span class="rung">{reachable.permission}</span>
			</button>
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
	}

	/* The chrome is the card's and the padding is the body's, so the whole of a card that
	   holds nothing else is clickable. A control landing beside it later takes its own room
	   out of this one rather than out of the card. */
	.body {
		display: block;
		width: 100%;
		text-align: left;
		background: none;
		border: 0;
		border-radius: var(--radius);
		padding: var(--space-3);
	}

	/* A monitored loop is a heavier card. It says so in words as well — the border is the
	   thing that makes a field of twenty cards readable in one look, not the thing carrying
	   the state. */
	.body[aria-pressed='true'] {
		box-shadow: inset 0 0 0 1px var(--ink);
	}

	.loop {
		display: block;
		font-size: var(--type-3);
	}

	.monitoring {
		display: block;
		margin-top: var(--space-1);
		font-size: var(--type-2);
	}

	.rung {
		display: block;
		margin-top: var(--space-1);
		color: var(--quiet);
		font-size: var(--type-2);
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

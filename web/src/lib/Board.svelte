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
	// (v1 §8), and the arm, the mute and the cog must not propagate that click. So the
	// clickable region is one element inside the card rather than the card itself, and it is a
	// `<button>`, which cannot contain another one: the three controls sit beside the body by
	// construction, and there is no `stopPropagation` for anybody to forget.
	//
	// **The loop name is not a heading**, and that changed when the body became a control: a
	// heading announces a section of content, and a card is a control. The `<ul>` is what
	// carries the structure — a list of twenty toggle buttons, each named for its loop and
	// its state — and a heading inside a button is not valid markup anyway.
	//
	// **A blind arm says so in words** (v1 §4, §8). Arming is independent of subscription
	// (ADR-0013), so a loop can be a destination for somebody who is not hearing it — which is
	// legal and is the case the console has to compensate for. The words are the compensation;
	// the talking indicator beside them is the rest of it.
	//
	// **A mute is a word on the card, and so is a loop turned down** (#44). The mute says
	// *Muted* beside *Monitoring* rather than instead of it, because the loop is still
	// monitored — which is why its talking indicator is still here. The volume says its
	// percentage only below unity: per-loop volume is the one attenuation nothing warns anybody
	// about (v1 §4), and the card is where the operator who turned it down is reminded, but a
	// word true of nearly every card on the board is one nobody reads.
	//
	// **Loop health is a word on the card only when it is not the ordinary case** (#46). A loop
	// whose beacon is not arriving says `Not receiving`, because a quiet loop and an
	// unreachable one sound identical and must never look identical (v1 §6); one just taken up
	// says `Checking`. A loop that is being received says nothing, for the reason a loop at
	// unity does — the ledger says it on every row.
	//
	// **Volume is behind the cog and nowhere else on the card** (v1 §8, ADR-0034). It is not a
	// live operational control, so nothing here can nudge it: the cog opens a modal scoped to
	// the loop, which `Console.svelte` holds above both views.
	//
	// **Staffing state is a word here and a sentence in the ledger** (v1 §8, ADR-0065). A card
	// cannot hold `away — 1 muted, 2 not subscribed`, and it does not have to: the word says
	// whether a human is behind the loop, which is what a glance is for, and the reading view
	// says why. A loop with no staffing roles says nothing where the word goes — blank, never
	// a fourth word (ADR-0056).
	//
	// **The loops this role staffs are marked, in two states**, and the mark is on the card
	// whether or not anything is wrong: *you staff this*, and *you would staff this and are
	// not monitoring it*. Showing only the second would make it an alarm rather than a fact
	// about the operator's own console, appearing for the first time at the moment something
	// is wrong. Clicking the body is the whole fix, and it is already the card's act.
	//
	// Nothing here decides which act a click is: it says which loop was clicked, and
	// `Console.svelte` reads the document to know the rest.
	import Icon from './Icon.svelte';
	import Talking from './Talking.svelte';
	import TransmitBar from './TransmitBar.svelte';
	import { carries } from './rungs.js';
	import { theWord } from './staffing.js';

	// `bar` is everything the transmit bar renders, handed over whole and never read here.
	// **Placing it is this view's business and wording it is the bar's** (ADR-0034), so this
	// view has no name for any of what is in it: a state that arrives as one value cannot be
	// half-forwarded, and adding one to the bar is not an edit to either view.
	let { loops, bar, onToggle, onArm, onMute, onCog } = $props();

	// Which loops carry an arm control at all. **Reach is the grid and only the grid**: a role
	// that may hear a loop and not speak on it gets no control, rather than one that is
	// refused when pressed (ADR-0016). The rung is read through `rungs.js`, which both views
	// share, because it is the grid's rule rather than either view's.
	const mayEmit = (reachable) => carries(reachable.permission, 'emit');
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
				{#if reachable.subscribed && reachable.muted}
					<span class="muted">Muted</span>
				{/if}
				{#if reachable.volume < 100}
					<span class="volume">{reachable.volume}%</span>
				{/if}
				<!-- Whether a human is behind the loop: a fact about the loop rather than about
				     this console, and blank where nothing staffs it (ADR-0056). -->
				{#if theWord(reachable.staffing)}
					<span class="staffing" class:nobody={reachable.staffing.state !== 'staffed'}>
						{theWord(reachable.staffing)}
					</span>
				{/if}
				<!-- The mark, in words rather than in colour alone, and in both its states. The
				     second is the actionable one: this card's body is what fixes it. -->
				{#if reachable.staffs}
					<span class="staffs">
						{reachable.subscribed ? 'You staff this' : 'You staff this, not monitoring it'}
					</span>
				{/if}
				{#if reachable.subscribed && reachable.health === 'not-receiving'}
					<span class="health unreceived">Not receiving</span>
				{:else if reachable.subscribed && reachable.health === 'checking'}
					<span class="health">Checking</span>
				{/if}
				<!-- The priority mark is this indicator's one variant, and it is drawn on every
				     loop the document marks — muted, unheard or at full volume (ADR-0059). -->
				{#if reachable.talking}
					<span class="spoken"><Talking priority={reachable.priority} /></span>
				{/if}
				<span class="rung">{reachable.permission}</span>
			</button>
			{#if mayEmit(reachable)}
				<p class="arming">
					<!-- The button names the act and never the state, the way the card body does:
					     what is true now is the word beside it, so neither has to be read as the
					     other and the state is never carried by `aria-pressed` alone. -->
					<button aria-pressed={reachable.armed} onclick={() => onArm(reachable)}>Arm</button>
					<span class="armed">
						{#if reachable.armed && !reachable.subscribed}
							<!-- The blind arm, in words rather than in a border (v1 §4). -->
							Armed, not hearing it
						{:else if reachable.armed}
							Armed
						{:else}
							Not armed
						{/if}
					</span>
				</p>
			{/if}
			<p class="hearing">
				<!-- Only on a loop being monitored: a mute presupposes a subscription (ADR-0049),
				     and a control that did nothing would misrepresent what a press does. Like the
				     arm, it names the act, and the word in the body names what is true now. -->
				{#if reachable.subscribed}
					<button aria-pressed={reachable.muted} onclick={() => onMute(reachable)}>
						{#if reachable.muted}
							<Icon name="volume-2" /> Unmute
						{:else}
							<Icon name="volume-x" /> Mute
						{/if}
					</button>
				{/if}
				<button
					class="cog"
					aria-label="Volume for {reachable.name}"
					onclick={() => onCog(reachable)}
				>
					<Icon name="settings" />
				</button>
			</p>
		</li>
	{/each}
</ul>

<!-- Never scrolled away (ADR-0034). On the board it closes the field along the bottom edge:
     the cards are scanned rather than read, and the bar is the one thing on the page that is
     not a loop. The page's own `--space-page-bottom` is what keeps the last row clear of it. -->
<div class="transmit">
	<TransmitBar {...bar} />
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

	.spoken {
		display: block;
		margin-top: var(--space-1);
	}

	/* Outside the body and inside the card: the chrome is the card's, so the arm takes its
	   room out of the body's padding rather than out of the card. */
	.arming {
		display: flex;
		align-items: center;
		gap: var(--space-2);
		margin: 0;
		padding: 0 var(--space-3) var(--space-3);
	}

	.armed {
		font-size: var(--type-2);
	}

	.muted,
	.volume,
	.health,
	.staffing,
	.staffs {
		display: block;
		margin-top: var(--space-1);
		font-size: var(--type-2);
	}

	/* The same room as the arm, under it. The cog goes to the far end of the row: it is a
	   setting rather than an act, and the mute beside it is the one of the two a hand reaches
	   for mid-shift. */
	.hearing {
		display: flex;
		align-items: center;
		gap: var(--space-2);
		margin: 0;
		padding: 0 var(--space-3) var(--space-3);
	}

	.cog {
		margin-left: auto;
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

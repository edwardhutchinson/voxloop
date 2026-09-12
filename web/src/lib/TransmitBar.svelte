<script>
	// The transmit bar: the console's answer to *who am I about to talk to*, and — because it
	// stays live while the key is held — to *who am I talking to* (ADR-0034, ADR-0058).
	//
	// **It is one component so that it is one wording.** Both views carry it and each puts it
	// somewhere different, and the thing that must not vary between them is what it says: the
	// bar is the whole of VoxLoop's compensation for emitting to several places at once, and a
	// board and a ledger disagreeing about the audience would be worse than neither saying
	// anything. Placing it is the view's business; wording it is this file's, and there is
	// nowhere else to write a word of it.
	//
	// **Emission has two independent withdrawal conditions, and the bar says which**
	// (ADR-0018, ADR-0042, v1 §6). A lost signalling channel and a lost audio path are
	// different problems with different fixes — one is *nobody can be told what you are
	// doing* and the other is *nobody can hear you* — and one wording for both would send an
	// operator to look at the wrong thing.
	//
	// **Where both apply, the signalling channel is named**, and the other is not stacked
	// underneath it. It is the one that also blinds this console, so it is the one to fix
	// first — and the media path drawn beside it is the last thing this console was told,
	// which is exactly the reading it has no business presenting as current.
	//
	// **The middle rungs are said too, and neither of them withdraws anything.** `impaired`
	// and `unconfirmed` exist for the same reason: a binary reading would cut audio for a
	// reroute or a blip that heals itself in a second. So they read as what they are — this
	// is true, and emission still stands.
	//
	// **The transmitting lamp is the server's answer and nothing else** (ADR-0008). It is
	// `keyed` out of the presence document, which is the only thing this component reads it
	// from — the button going down lights nothing, and there is no local state here that
	// could. That round trip is the cost of the honesty rule and it is paid deliberately:
	// audio is already flowing by the time the lamp lights, so it is a display latency rather
	// than an audio one.
	//
	// **Two modes and no third** (v1 §4), and they are two controls here because they are two
	// bindings (ADR-0022). Latch is never reached by how the key control was pressed — no
	// short tap, no double tap, no held duration — so a single-button device is momentary
	// only, and the second button is what the console offers instead of a gesture.
	//
	// **The latch is this console's own state and the lamp is the server's**, and the two are
	// rendered as the different things they are (ADR-0016). *You have latched the key open* is
	// a fact about the input on this desk, knowable here and true the moment it is said; *you
	// are on the air* is the server's answer and arrives in the presence document. So the
	// latch never touches the lamp, and pressing latch lights nothing.
	//
	// **Priority is a third control, held like the key control and never latched** (ADR-0046).
	// It is a second level beside the ordinary one rather than a mode, so it sits between the
	// latch and the lamp and publishes a press and a release like the key control does: from
	// cold it keys and elevates, and over a latch it elevates and leaves the latch standing.
	// Whether the transmission *is* at priority is the server's answer like the lamp is, so the
	// lamp says it — an elevated latch shows as elevated with no new surface.
	//
	// **The audience is two counts and no names** (ADR-0034). Sixteen names is more than
	// anyone reads in the second before keying, and the two numbers that remain are the two
	// that change a decision: the first is reassurance and the second is a warning. The third
	// bucket — people in reach who did not take the loop up — is computed on the server and
	// never sent here, which is where that promise is kept: the document is the API, so a
	// count the console must not render is a count the console is not handed.
	//
	// **`0 hearing` blocks nothing.** It renders in the warning colour and the key control is
	// untouched: emitting to a loop nobody is hearing is legal, and blocking it — or
	// interposing a dialog — would be the console overruling an operator about their own
	// operation.
	//
	// **There is no third count.** The bar could say *3 of 6 have this turned down* and does
	// not: it would sit beside `present, not hearing` and be read as another flavour of it,
	// and a volume is persisted precisely because it is safe to be stale, so it can be wrong.
	//
	// The presets that sit beside the key control are #56's.

	import Icon from './Icon.svelte';
	import { CONFIRMED, DISCONNECTED, UNCONFIRMED } from './session.js';

	// The media path as the presence document has it: `connected`, `impaired` or `lost`
	// (ADR-0042). Anything else is read as `lost`, which is the safe direction and the honest
	// one — a console that cannot tell what the audio path is doing has no business offering
	// a key control over it.
	//
	// `armedOn` is the loops this session has armed, in the document's order and by name;
	// `keyed` is the server's answer about this session; `mayKey` is whether emission stands
	// at all, decided once above both views because Input's liveness is decided from the same
	// answer.
	//
	// `onDown`/`onUp` and `onLatchDown`/`onLatchUp` are what the two controls publish to
	// Input. They are four callbacks rather than two and a mode, because **a source never
	// knows which emission mode it serves** (ADR-0021): each of these is a button reporting
	// what a pointer is doing to it, and which of the two modes that serves is settled in
	// `modes.js`, above the seam.
	//
	// `connection` is where this tab stands with the signalling channel, in the ladder's own
	// words: `confirmed`, `unconfirmed` or `disconnected` (ADR-0018). It is measured by this
	// console rather than pushed by the server, because the one thing a server cannot do to a
	// console it has lost is tell it that it has been lost.
	//
	// `latched` is whether the key is latched open, `dropped` is the source that went while it
	// was being held, if one has, and `latchDropped` is whether a latch was taken down by
	// something other than the operator.
	//
	// `priority` is the server's answer about whether this session's transmission is at
	// priority, read out of the document beside `keyed`; `onPriorityDown`/`onPriorityUp` are
	// what the priority control publishes to Input, as a button and nothing more.
	//
	// `audience` is who would actually hear this arm set, as the document counts them: the
	// two buckets it carries, and no third. `armsMovedElsewhere` is the document's answer to
	// *did anything but this session last move that set* — the mark, and the server is the
	// only thing that can say it, because it is the thing that applies both kinds of change.
	let {
		mediaPath,
		connection = CONFIRMED,
		armedOn = [],
		keyed = false,
		mayKey = false,
		latched = false,
		dropped = null,
		latchDropped = false,
		onDown,
		onUp,
		onLatchDown,
		onLatchUp,
		priority = false,
		onPriorityDown,
		onPriorityUp,
		audience = { hearing: 0, presentNotHearing: 0 },
		armsMovedElsewhere = false
	} = $props();

	// **The armed set in words** (ADR-0034), and the same words in both views. It is a list
	// rather than a count because this is the half of the bar an operator acts on: the second
	// before keying is spent reading where their voice is about to go, and *three loops* does
	// not answer that.
	const destinations = $derived(
		armedOn.length === 0 ? 'nothing' : new Intl.ListFormat('en').format(armedOn)
	);

	// The counts as they are rendered, read straight out of the document and computed nowhere
	// else: what the operator is told is what the server has committed to keeping true.
	const hearing = $derived(audience.hearing);
	const presentNotHearing = $derived(audience.presentNotHearing);

	// **The console must not place a focusable control where an operator's hands rest**
	// (v1 §4). A key pressed with focus on a control is refused, so a key control that took
	// focus when it was clicked would leave the operator's *keyboard* binding dead until they
	// clicked somewhere else — on the one control where that matters most, and with nothing on
	// screen to explain it. Preventing the default on the way down is what stops focus moving,
	// and it is the only thing prevented: the press itself is published from the same handler.
	const holding = (event) => {
		event.preventDefault();
		onDown();
	};

	const pressing = (event) => {
		event.preventDefault();
		onLatchDown();
	};

	const holdingPriority = (event) => {
		event.preventDefault();
		onPriorityDown();
	};

	// The lamp, in the server's words. `priority` without `keyed` is not a state the server
	// sends, and it is read as what the lamp is for: whether this session is on the air.
	const lamp = $derived(keyed ? (priority ? 'Keyed at priority' : 'Keyed') : 'Not keyed');
</script>

<section aria-label="Transmit bar">
	<!-- The rungs, loudest first, and one of them at a time. Which one is on screen is what
	     tells an operator where to look; two at once would make them choose. -->
	{#if connection === DISCONNECTED}
		<!-- **A session with no signalling channel has no emission path** (ADR-0018). Every
		     talking indicator anyone sees is a server broadcast, so keying here would put
		     voice into a system where nobody's console shows it, no loop attributes it and no
		     authority holder can cut it. This console disables the key control and the server
		     closes the fan-out, independently, because the situation that makes the rule
		     necessary is the one where this console may itself be wedged. -->
		<p class="withdrawn" role="status">
			VoxLoop and this console are not in touch, so it will not emit: nobody's console would show
			you talking, and nobody could cut you. This is the connection to VoxLoop rather than the
			audio, which is a different problem with a different fix.
		</p>
	{:else if !mayKey}
		<p class="withdrawn" role="status">
			There is no audio path, so VoxLoop will not emit. This is the audio rather than the connection
			to VoxLoop, which is a different problem with a different fix.
		</p>
	{:else if connection === UNCONFIRMED}
		<!-- *We cannot confirm your transmission right now* is a materially different statement
		     from *we know you are disconnected*, and it is the honest one here. Emission stands
		     — cutting somebody off mid-word for a half-second blip is the failure this rung
		     exists to prevent — but a latch does not, so the sentence says both. -->
		<p class="unconfirmed" role="status">
			VoxLoop cannot confirm what you are doing, so a latched key will not be held open. You can
			still talk by holding the key.
		</p>
	{:else if mediaPath === 'impaired'}
		<!-- A transient fault, of the kind that routinely clears itself in a second or two.
		     Emission stands: a binary reading would cut audio for a reroute that heals, which
		     is exactly what the middle rung exists to prevent (ADR-0042). -->
		<p class="impaired" role="status">
			The audio path is faulty. This usually clears itself, and emission still stands.
		</p>
	{/if}

	<!-- **The armed set stands whatever the audio path is doing** (ADR-0034). The bar answers
	     *who am I about to talk to*, and an operator whose path has dropped is owed that answer
	     more than anybody: it is what they are coming back to. Only the key control goes. -->
	<p class="armed">Armed on {destinations}.</p>

	{#if armsMovedElsewhere}
		<!-- **Only a change this session did not ask for is marked** (ADR-0058). A preset is a
		     mid-key change by design and the most routine one in the system, and a deliberate
		     arm is the operator's own hand; marking either would fire the signal constantly
		     and train an operator straight past it. What is left is the change nobody here
		     made, and the operator may be mid-sentence when it lands — so it is said in words
		     beside the set it is about, and it stands until they do something deliberate. -->
		<p class="moved" role="status">
			A permission change moved this set. Your own hand did not, so read it again before you key.
		</p>
	{/if}

	<!-- **The audience, before keying and while keyed, in the same words** (ADR-0034,
	     ADR-0058). Two counts and no names: the first is what says the voice is going
	     somewhere and the second is what says somebody who means to be covering this will not
	     hear it. It is the whole of VoxLoop's compensation for emitting to several places at
	     once, because the receiving side is told nothing (ADR-0057). -->
	<p class="audience">
		<span class:unheard={hearing === 0}>{hearing} hearing</span>
		<span class:warned={presentNotHearing > 0}>{presentNotHearing} present, not hearing</span>
	</p>

	{#if dropped}
		<!-- **A source that dies while keyed forces an unkey and says so locally** (ADR-0021).
		     Outside the block below on purpose: the case that produces this is the audio path
		     going under a held key, which takes the key control away with it, so a notice
		     drawn beside that control would be a notice nobody ever reads.
		     It names what they are holding rather than what went, because that is the part
		     they can act on and the part that is true in every case that produces this: the
		     hand is still down, and nothing it does from there talks until it comes up. -->
		<p class="dropped" role="status">
			You were still holding {dropped} when VoxLoop stopped emitting. Let go and press again to talk.
		</p>
	{/if}

	{#if latchDropped}
		<!-- **The one user-facing message in the product that does not originate at the
		     server** (ADR-0018). A latch is an assertion made once, possibly minutes ago, and
		     its entire safety story is that this console will show it to you; the moment this
		     console cannot be trusted, the latch is a hot mic nobody can be told about. So it
		     is dropped, and this is the announcement that can still be made.

		     It says what it cost and not what caused it: the sentence above is what names the
		     rung, and this one is true whichever of them took the latch down — the channel
		     going quiet, or the audio path going. An operator who believes they are still
		     transmitting is the failure the rule exists to remove, arriving through the other
		     door, so it is the one alert in this strip and it stands until they key again. -->
		<p class="dropped" role="alert">
			The latched key was dropped, so you are not transmitting. Press Latch again once this clears.
		</p>
	{/if}

	{#if mayKey}
		<p class="keying">
			<!-- **The key control renders differently at zero armed** (v1 §8) rather than being
			     disabled: a revocation can empty the arm set under somebody who is mid-sentence,
			     and taking the control out of their hand is a bigger lie than showing them that
			     it reaches nobody. It still keys. -->
			<button
				class="key"
				aria-pressed={keyed}
				onpointerdown={holding}
				onpointerup={onUp}
				onpointercancel={onUp}
				onpointerleave={onUp}
			>
				<Icon name={armedOn.length === 0 ? 'mic-off' : 'mic'} />
				{armedOn.length === 0 ? 'Key — reaching nobody' : 'Key'}
			</button>

			<!-- **Press to open, press to close** (v1 §4), which is why the act is on the way
			     down and there is nothing on the way up that could undo it. It names the act
			     rather than the state, the way every control on the console does; what is true
			     now is the sentence under it. -->
			<button
				aria-pressed={latched}
				onpointerdown={pressing}
				onpointerup={onLatchUp}
				onpointercancel={onLatchUp}
				onpointerleave={onLatchUp}
			>
				{latched ? 'Unlatch' : 'Latch'}
			</button>

			<!-- **Held, and never latched** (v1 §4), so it is drawn and wired like the key
			     control: a press and a release, and nothing on the way down that could hold it.
			     It names the act; what is true now is the lamp, lit by the document. -->
			<button
				aria-pressed={priority}
				onpointerdown={holdingPriority}
				onpointerup={onPriorityUp}
				onpointercancel={onPriorityUp}
				onpointerleave={onPriorityUp}
			>
				Priority
			</button>

			<!-- The lamp, in words, and lit by the document alone. It is a separate thing from
			     the control that asks for it, because *I pressed this* and *VoxLoop says you are
			     on the air* are two facts and only the second one is worth showing. -->
			<span class="lamp" class:priority={keyed && priority} role="status">
				{lamp}
			</span>
		</p>

		{#if latched}
			<!-- Said in words rather than carried by the pressed control alone, and said as the
			     local thing it is: this is the console holding the key open, not VoxLoop
			     reporting that it is. The lamp beside it is the half that came back. -->
			<p class="latched">
				You have latched the key open. It stays open until you unlatch it or the audio path goes.
			</p>
		{/if}
	{/if}
</section>

<style>
	/* Five names for one rendering, deliberately, the way `.refusal` and `.destructive` are:
	   a fault that clears itself, a channel that cannot confirm, a fault that has withdrawn
	   emission, a latch that was taken away and an arm set somebody else moved read alike and
	   are not the same thing, so a rule that later tells them apart has somewhere to go.
	   `.impaired` is the media path's rung and `.unconfirmed` is the signalling channel's, and
	   they are two classes rather than one because the two ladders are two ladders
	   (`CONTEXT.md`).

	   The colour is v1 §8's — *this is true and you should look at it* — and it is never what
	   carries the state: the sentence says which of the two withdrawal conditions applies and
	   would still say it in monochrome. */
	.impaired,
	.unconfirmed,
	.withdrawn,
	.dropped,
	.moved {
		margin: 0;
		color: var(--warning);
	}

	.armed {
		margin: 0;
		font-size: var(--type-2);
	}

	/* The two counts on one line, in the order they are read: the reassurance, then the
	   warning. They are spans inside one paragraph rather than two paragraphs, because they
	   are one answer — *who will hear me* — and a reader who takes in only the first half of
	   it has been told something that is not true. */
	.audience {
		display: flex;
		gap: var(--space-3);
		margin: 0;
		font-size: var(--type-2);
	}

	/* Two names for one rendering, deliberately. `0 hearing` is *this reaches nobody* and it
	   blocks nothing; a count of people who took the loop up and will not hear it is *these
	   people believe they are covering this*, which is the console's only warning about mute
	   (ADR-0034). They are different facts with different fixes, so a rule that later tells
	   them apart has somewhere to go, and the words say both without the colour. */
	.unheard,
	.warned {
		color: var(--warning);
	}

	.keying {
		display: flex;
		align-items: center;
		gap: var(--space-3);
		margin: var(--space-2) 0 0;
	}

	/* The one control on the console an operator's hand rests on, so it is the one that is
	   worth being larger than the furniture. */
	.key {
		font-size: var(--type-3);
	}

	/* The lamp is drawn from the document and is never pre-lit, so there is no pressed state
	   here to style — what changes is the word. The heavier weight is what makes it findable
	   at the edge of vision without motion, which is spent elsewhere. */
	.lamp {
		font-size: var(--type-2);
		font-weight: 600;
	}

	/* A general sibling rather than an adjacent one: the latch and priority controls sit between
	   the key and the lamp, and an adjacent combinator would not reach past them. */
	.key[aria-pressed='false'] ~ .lamp {
		color: var(--quiet);
		font-weight: inherit;
	}

	/* The same colour the priority mark takes on a card, for the same reason and never alone:
	   the lamp's words say it. */
	.lamp.priority {
		color: var(--warning);
	}

	/* The latch control stays at the furniture's size, beside a key control that does not: the
	   key is the one an operator's hand rests on, and two controls both claiming that would
	   make neither of them findable. What is true now runs underneath, in words. */
	.latched {
		margin: var(--space-2) 0 0;
		font-size: var(--type-2);
	}
</style>

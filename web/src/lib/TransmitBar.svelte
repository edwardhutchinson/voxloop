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
	// **Emission has two independent withdrawal conditions**, and the bar has to say *which*
	// (ADR-0042, v1 §6). A lost signalling channel and a lost audio path are different
	// problems with different fixes — one is *nobody can be told what you are doing* and the
	// other is *nobody can hear you* — and one wording for both would send an operator to
	// look at the wrong thing. What is here today is the audio path. The state channel's
	// ladder is the console's `lost` banner until ADR-0018's rungs are built, and when they
	// are, they are said **here**, beside this, in these words.
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
	// The two audience counts are #49's, and the presets that sit beside the key control are
	// #56's.

	import Icon from './Icon.svelte';

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
	// `latched` is whether the key is latched open, and `dropped` is the source that went
	// while it was being held, if one has.
	let {
		mediaPath,
		armedOn = [],
		keyed = false,
		mayKey = false,
		latched = false,
		dropped = null,
		onDown,
		onUp,
		onLatchDown,
		onLatchUp
	} = $props();

	// **The armed set in words** (ADR-0034), and the same words in both views. It is a list
	// rather than a count because this is the half of the bar an operator acts on: the second
	// before keying is spent reading where their voice is about to go, and *three loops* does
	// not answer that.
	const destinations = $derived(
		armedOn.length === 0 ? 'nothing' : new Intl.ListFormat('en').format(armedOn)
	);

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
</script>

<section aria-label="Transmit bar">
	{#if mediaPath === 'impaired'}
		<!-- A transient fault, of the kind that routinely clears itself in a second or two.
		     Emission stands: a binary reading would cut audio for a reroute that heals, which
		     is exactly what the middle rung exists to prevent (ADR-0042). -->
		<p class="impaired" role="status">
			The audio path is faulty. This usually clears itself, and emission still stands.
		</p>
	{:else if !mayKey}
		<p class="withdrawn" role="status">
			There is no audio path, so VoxLoop will not emit. This is the audio rather than the connection
			to VoxLoop, which is a different problem with a different fix.
		</p>
	{/if}

	<!-- **The armed set stands whatever the audio path is doing** (ADR-0034). The bar answers
	     *who am I about to talk to*, and an operator whose path has dropped is owed that answer
	     more than anybody: it is what they are coming back to. Only the key control goes. -->
	<p class="armed">Armed on {destinations}.</p>

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

			<!-- The lamp, in words, and lit by the document alone. It is a separate thing from
			     the control that asks for it, because *I pressed this* and *VoxLoop says you are
			     on the air* are two facts and only the second one is worth showing. -->
			<span class="lamp" role="status">
				{keyed ? 'Keyed' : 'Not keyed'}
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
	/* Two names for one rendering, deliberately, the way `.refusal` and `.destructive` are:
	   a fault that clears itself and a fault that has withdrawn emission read alike and are
	   not the same thing, so a rule that later tells them apart has somewhere to go.

	   The colour is v1 §8's — *this is true and you should look at it* — and it is never what
	   carries the state: the sentence says which of the two withdrawal conditions applies and
	   would still say it in monochrome. */
	.impaired,
	.withdrawn,
	.dropped {
		margin: 0;
		color: var(--warning);
	}

	.armed {
		margin: 0;
		font-size: var(--type-2);
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

	/* A general sibling rather than an adjacent one: the latch control sits between the key and
	   the lamp, and an adjacent combinator would quietly stop matching the day a third binding
	   lands beside them (ADR-0046). */
	.key[aria-pressed='false'] ~ .lamp {
		color: var(--quiet);
		font-weight: inherit;
	}

	/* The latch control stays at the furniture's size, beside a key control that does not: the
	   key is the one an operator's hand rests on, and two controls both claiming that would
	   make neither of them findable. What is true now runs underneath, in words. */
	.latched {
		margin: var(--space-2) 0 0;
		font-size: var(--type-2);
	}
</style>

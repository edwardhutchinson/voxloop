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
	// The two audience counts are #49's, and the presets that sit beside the key control are
	// #56's.

	import Icon from './Icon.svelte';

	// The media path as the presence document has it: `connected`, `impaired` or `lost`
	// (ADR-0042). Anything else is read as `lost`, which is the safe direction and the honest
	// one — a console that cannot tell what the audio path is doing has no business offering
	// a key control over it.
	//
	// `armed` is the loops this session has armed, in the document's order and by name;
	// `keyed` is the server's answer about this session; `onDown` and `onUp` are what the key
	// control publishes to Input, which is where the mode logic will live (#42).
	let { mediaPath, armed = [], keyed = false, onDown, onUp } = $props();

	// **The armed set in words** (ADR-0034), and the same words in both views. It is a list
	// rather than a count because this is the half of the bar an operator acts on: the second
	// before keying is spent reading where their voice is about to go, and *three loops* does
	// not answer that.
	const destinations = $derived(
		armed.length === 0 ? 'nothing' : new Intl.ListFormat('en').format(armed)
	);

	// Emission is withdrawn on a lost path (ADR-0042), so there is no key control over one.
	// `impaired` keeps it: a transient fault clears itself in a second or two, and a binary
	// reading would cut audio for a reroute that heals.
	const mayKey = $derived(mediaPath === 'connected' || mediaPath === 'impaired');
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

	{#if mayKey}
		<p class="armed">Armed on {destinations}.</p>

		<p class="keying">
			<!-- **The key control renders differently at zero armed** (v1 §8) rather than being
			     disabled: a revocation can empty the arm set under somebody who is mid-sentence,
			     and taking the control out of their hand is a bigger lie than showing them that
			     it reaches nobody. It still keys. -->
			<button
				class="key"
				aria-pressed={keyed}
				onpointerdown={onDown}
				onpointerup={onUp}
				onpointercancel={onUp}
				onpointerleave={onUp}
			>
				<Icon name={armed.length === 0 ? 'mic-off' : 'mic'} />
				{armed.length === 0 ? 'Key — reaching nobody' : 'Key'}
			</button>

			<!-- The lamp, in words, and lit by the document alone. It is a separate thing from
			     the control that asks for it, because *I pressed this* and *VoxLoop says you are
			     on the air* are two facts and only the second one is worth showing. -->
			<span class="lamp" role="status">
				{keyed ? 'Transmitting' : 'Not transmitting'}
			</span>
		</p>
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
	.withdrawn {
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

	.key[aria-pressed='false'] + .lamp {
		color: var(--quiet);
		font-weight: inherit;
	}
</style>

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
	// The armed set in words and the key state arrive with #41, the two audience counts with
	// #49.
	//
	// The act it is about is **emission** rather than *transmit*, which `CONTEXT.md` avoids;
	// the strip itself is the *transmit bar*, which is the glossary's own name for it.

	// The media path as the presence document has it: `connected`, `impaired` or `lost`
	// (ADR-0042). Anything else is read as `lost`, which is the safe direction and the honest
	// one — a console that cannot tell what the audio path is doing has no business offering
	// a key control over it.
	let { mediaPath } = $props();
</script>

<section aria-label="Transmit bar">
	{#if mediaPath === 'connected'}
		<p class="quiet">
			VoxLoop cannot emit yet. The armed set, the audience and the key control belong here.
		</p>
	{:else if mediaPath === 'impaired'}
		<!-- A transient fault, of the kind that routinely clears itself in a second or two.
		     Emission stands: a binary reading would cut audio for a reroute that heals, which
		     is exactly what the middle rung exists to prevent (ADR-0042). -->
		<p class="impaired" role="status">
			The audio path is faulty. This usually clears itself, and emission still stands.
		</p>
	{:else}
		<p class="withdrawn" role="status">
			There is no audio path, so VoxLoop will not emit. This is the audio rather than the connection
			to VoxLoop, which is a different problem with a different fix.
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
</style>

<script>
	// The check tone: the operator confirming that VoxLoop reaches their ears (ADR-0017).
	//
	// **The beacon proves audio reached the browser, and says nothing about the last metre.**
	// A headset unplugged, or the operating system repointing its default output, is invisible
	// to the server and to the beacon alike — and it is the likeliest real *"I can't hear
	// Flight"*. So at assume the console asks the operator to play a tone and say whether they
	// heard it, and it asks again, loudly, whenever the output moves under them.
	//
	// **It is a component because it holds a decision** (`styling.md`): whether the operator
	// has said they heard the tone is theirs to say and nothing else's, so it is kept here and
	// nowhere else. It is not the server's either — nothing on this page is sent up — because
	// what reaches somebody's ears is a fact about the desk in front of them.
	//
	// `moved` is what the console's watch on the outputs last saw change, or nothing. It is
	// read here rather than kept, and `onConfirmed` is how the console learns the operator has
	// heard the tone on the output as it now stands.
	import { playTheCheckTone } from './output.js';

	let { moved = null, onPlay = playTheCheckTone, onConfirmed = () => {} } = $props();

	// `unchecked` until the tone has been played at least once, `played` while the operator is
	// being asked, and then `heard` or `unheard` on their answer.
	let step = $state('unchecked');

	async function play() {
		try {
			await onPlay();
		} catch (why) {
			// A tone that could not be played is one nobody heard, and the question is still
			// asked: the operator is the one who knows whether anything came out.
			console.error('VoxLoop could not play the check tone', why);
		}
		step = 'played';
	}

	function answered(heard) {
		step = heard ? 'heard' : 'unheard';
		if (heard) onConfirmed();
	}
</script>

{#if step === 'played'}
	<div class="check" role="group" aria-label="Check tone">
		<p>Did you hear the check tone?</p>
		<p class="answers">
			<button onclick={() => answered(true)}>Yes, I heard it</button>
			<button onclick={() => answered(false)}>No</button>
		</p>
	</div>
{:else if step === 'unheard'}
	<div class="check moved" role="alert">
		<p>
			VoxLoop is not reaching your ears. Check that your headset is plugged in and chosen as your
			audio output, then play the tone again.
		</p>
		<p class="answers"><button onclick={play}>Play the check tone</button></p>
	</div>
{:else if moved}
	<!-- Loud because it is the one change nothing else on this page would show: every loop can
	     read as received while the sound goes to a speaker across the room. -->
	<div class="check moved" role="alert">
		<p>
			{#if moved.removed}
				An audio output was unplugged.
			{/if}
			{#if moved.swapped}
				Your audio output changed: sound now goes to {moved.swapped}.
			{/if}
			Play the tone to check that you can still hear VoxLoop.
		</p>
		<p class="answers"><button onclick={play}>Play the check tone</button></p>
	</div>
{:else if step === 'unchecked'}
	<div class="check" role="group" aria-label="Check tone">
		<p>Check that you can hear VoxLoop before you rely on it.</p>
		<p class="answers"><button onclick={play}>Play the check tone</button></p>
	</div>
{/if}

<style>
	.check {
		margin: 0 0 var(--space-4);
		padding: var(--space-3) var(--space-4);
		background: var(--raised);
		border: 1px solid var(--rule);
		border-radius: var(--radius);
	}

	.check p {
		margin: 0;
	}

	/* Only the sentence takes the warning colour below. The controls stay controls. */
	.answers {
		display: flex;
		gap: var(--space-2);
		margin-top: var(--space-2);
		color: var(--ink);
	}

	/* The output moved, or the operator did not hear the tone: the words say which, and the
	   warning colour makes it hard to walk past — the same reading `--warning` has everywhere,
	   *this is true and you should look at it*. */
	.moved {
		border-color: var(--warning);
		color: var(--warning);
	}
</style>

<script>
	// The keys, and changing them.
	//
	// **Bindings are the user's** (ADR-0021): a keybinding is not reach, so it is
	// personalisation rather than administration, and it is changed here rather than by a
	// system administrator. What it is *not* is a second way to reach a mode — the two modes
	// are ADR-0022's and there are two of them here because there are two of them, and the
	// only thing this page can do is say which key each one is on.
	//
	// **What may be bound is the seam's answer, never this component's.** Space activates
	// focused controls and `CapsLock` does not reliably report being released, and both are
	// refused in `input/bindings.js` where the reasoning is — a console carrying its own copy
	// of that list is a console that can get it wrong. This asks and shows the sentence it is
	// given.
	//
	// **A change lasts as long as this console does.** Persisting it is #55's, along with
	// every other thing a user has set about their own console; what is here is the act, and
	// the sentence below says plainly which of the two it is so that nobody discovers it by
	// reloading.
	import { describe, fromEvent, stillReaching } from '$lib/input';

	// `modes` is what each mode is called and what it does, from above the seam — this
	// component is as unable to name a mode as everything below it, and for the same reason.
	// `bound` is the key each of them is on, and `onRebind` answers with a refusal or `null`.
	let { modes, bound, onRebind } = $props();

	// Which mode is waiting for a key, and what was said about the last attempt. Both are
	// facts about this form rather than about the world, which is the whole of what a console
	// may hold locally (ADR-0016).
	let capturing = $state(null);
	let refused = $state(null);

	function stop() {
		capturing = null;
		refused = null;
	}

	// **One button rather than two.** Swapping the control for a different one when capture
	// starts would take the focus with it — the element that had it is gone — and a form
	// waiting for a key press with nothing focused is a form that never hears one.
	function pressed(named, event) {
		if (capturing === named) return captured(event);
	}

	function captured(event) {
		// Every press while this is listening is the binding rather than whatever the key
		// usually does, so nothing here submits, scrolls or types.
		event.preventDefault();

		if (event.code === 'Escape') return stop();
		// Somebody reaching for `Shift + \`` presses shift first. That is not a binding and it
		// is not a mistake either, so it is neither taken nor refused.
		if (stillReaching(event)) return;

		const no = onRebind(capturing, fromEvent(event));
		if (no) {
			refused = no;
			return;
		}

		stop();
	}
</script>

<section>
	<h3>Keys</h3>
	<p class="quiet">
		These keys work while you hold this role, and not while you are typing or while a control has
		focus. They last as long as this console: VoxLoop does not yet remember them for you.
	</p>

	<ul>
		{#each modes as mode (mode.named)}
			<li>
				<span>{mode.called}</span>
				<!-- The key as somebody would say it out loud, and never as the code the browser
				     uses: a binding nobody can read is a binding nobody can check. -->
				<span class="binding">{describe(bound[mode.named])}</span>
				<!-- The listener is on the button because the button is what has focus, and a
				     press with focus on a control is one the keyboard source refuses anyway
				     (ADR-0022) — which is exactly what makes this the safe place to capture
				     one. Looking away is giving up: a form left waiting for a key would be one
				     that took the next thing typed anywhere on the page. -->
				<button
					aria-pressed={capturing === mode.named}
					onclick={() => {
						capturing = mode.named;
						refused = null;
					}}
					onkeydown={(event) => pressed(mode.named, event)}
					onblur={() => capturing === mode.named && stop()}
				>
					{capturing === mode.named ? 'Press a key, or Esc' : 'Change'}
				</button>
				<span class="meaning">{mode.means}</span>
				{#if capturing === mode.named && refused}
					<span class="refusal" role="alert">{refused}</span>
				{/if}
			</li>
		{/each}
	</ul>
</section>

<style>
	h3 {
		margin: 0;
		font-size: var(--type-3);
	}

	ul {
		margin: var(--space-3) 0 0;
		padding: 0;
		list-style: none;
	}

	/* A row per mode: what it is, the key it is on, and the way to change it. The key sits
	   between the two so that reading down the column answers *which keys am I on* without
	   reading either sentence. */
	li {
		display: grid;
		grid-template-columns: 1fr auto auto;
		align-items: baseline;
		gap: var(--space-2) var(--space-3);
		padding: var(--space-2) 0;
		border-top: 1px solid var(--rule);
	}

	.binding {
		font-family: ui-monospace, monospace;
	}

	/* The sentence and the refusal run under the row rather than in a column of their own:
	   they are about the whole row, and a fourth column would set the grid by its longest
	   sentence. */
	.meaning,
	.refusal {
		grid-column: 1 / -1;
		font-size: var(--type-1);
	}
</style>

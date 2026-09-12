<script>
	// The one asserted state in VoxLoop, and the rules that keep it from being mistaken for an
	// observed one (ADR-0016, v1 §6).
	//
	// **Everything else on this console is something the server saw.** This is something an
	// operator said, and it is only ever as true as the moment they said it — so it is drawn
	// differently, it is always shown with how long ago its claimant last did anything
	// deliberate, and neither of those is optional here: the document carries the claim and the
	// age as one value, so there is no way to render the first without the second.
	//
	// **A stale assertion is still shown, with its age.** Nothing here expires one, hides one
	// or rounds one away. An operator who said they were stepping out an hour ago and has not
	// been seen since is exactly what this says, and the judgement about what that means is the
	// reader's. That is a deliberate refusal to be helpful.
	//
	// **It is a component because it holds a decision**, and because it is the one place the
	// asserted rendering lives: a second surface drawing a claim like a fact is a diff a
	// reviewer would have to notice, and there is nowhere here for one to be written.
	//
	// **Nothing here watches for activity.** There is no timer, no `mousemove`, no `scroll` and
	// no focus handler, because VoxLoop never guesses whether a human is in the chair — an
	// operator watching telemetry is idle at the keyboard and very much on console. What clears
	// the claim is a deliberate act, and every one of those is a message the server already
	// receives, so this file has nothing to contribute to the clearing but a button for
	// somebody who has come back and has nothing else to do yet.

	// `asserted` is the document's `off_console`: `null` where nobody has claimed anything —
	// *on console* is the absence of a claim rather than a second one — and otherwise the claim
	// with the age of its evidence.
	let { asserted, onOffConsole, onBackOnConsole } = $props();

	// **The age as the server measured it**, said in the largest unit that does not round the
	// number away. It is not this tab's clock: the acts it is measured from arrive at the
	// server, and a console counting its own would be a second answer to a question that
	// already has one (ADR-0016).
	//
	// It rolls up to minutes and hours where the connection's age above it does not, and that
	// is the difference between the two facts rather than an inconsistency to tidy away: a
	// channel is measured in seconds and is acted on within seconds, and this is measured in
	// the length of a tea break. `300 s ago` is the right reading of one and the wrong reading
	// of the other.
	function howLongAgo(seconds) {
		if (seconds < 60) return `${seconds} s ago`;
		if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;

		const hours = Math.floor(seconds / 3600);
		const minutes = Math.floor((seconds % 3600) / 60);

		return minutes === 0 ? `${hours} h ago` : `${hours} h ${minutes} min ago`;
	}
</script>

{#if asserted}
	<!-- `role="status"` rather than `alert`: it is not a fault and not a refusal. It is this
	     operator's own claim, said back to them. -->
	<div class="asserted" role="status">
		<p>
			<!-- **The words carry the provenance**, because colour and a border never carry a state
			     on their own here. *You said* is the whole difference between this and every other
			     line on the page, and it is said first. -->
			You said you are off console. Last active {howLongAgo(asserted.last_active_seconds)}.
		</p>
		<!-- **What it says is what is true today.** Declaring this drops the staffing state of
		     the loops your role staffs to `away` — and staffing state does not exist yet (#48),
		     so the sentence that says so lands with it. A console explaining a consequence
		     nothing has yet is the class of lie this product exists to avoid, and it is not
		     made safe by being about the future. -->
		<p class="quiet">
			VoxLoop cannot see whether you are at your desk — this is what you told it, not something it
			has seen. Your loops are still open and audio is still playing.
		</p>
		<button onclick={onBackOnConsole}>I am back on console</button>
	</div>
{:else}
	<p class="acts">
		<button onclick={onOffConsole}>Off console</button>
	</p>
{/if}

<style>
	/* **Drawn unlike anything observed, in the shape as well as in the words** (ADR-0016). The
	   stale and lost marks above it are sentences in the warning colour, and every state on a
	   card is a word inside a solid border; this is a dashed box, which is the one outline on
	   the console that says *nobody checked this* — and it is deliberately not the warning
	   colour, because a claim an operator made about themselves is not a fault to look at. */
	.asserted {
		margin: 0 0 var(--space-4);
		padding: var(--space-3) var(--space-4);
		/* Dashed rather than solid: the border is doing the same work as the wording, and a
		   solid one is what every observed surface on this console already wears. */
		border: 1px dashed var(--quiet);
		border-radius: var(--radius);
	}

	.asserted p {
		margin: 0 0 var(--space-2);
	}

	/* The way out sits under the claim rather than beside it, because it is the answer to the
	   sentence above it and not a control over the page. */
	.asserted button {
		margin-top: var(--space-1);
	}
</style>

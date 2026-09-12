<script>
	// The staffing flag, as one control: does this role count toward this loop's staffing
	// state (v1 §1)?
	//
	// It sits beside `Rungs.svelte` and is deliberately not part of it. **It is not a fifth
	// rung**: it confers nothing — no subscription, no reach, no console change (ADR-0065) —
	// and a ladder ending in it would make *counts as cover* something an administrator
	// grants by raising somebody's permission.
	//
	// **A role that cannot answer cannot staff**, so where the cell holds less than `emit`
	// there is no control at all and the page says why. That is the same rule the console
	// follows for an arm control: reach is the grid, and an act that would be refused when
	// pressed is not offered (ADR-0016).
	import Icon from './Icon.svelte';

	let { staffs, mayEmit, of, onset, busy = false } = $props();
</script>

{#if mayEmit}
	<button
		class="setting"
		class:held={staffs}
		aria-pressed={staffs}
		aria-label="Staffs {of}"
		disabled={busy}
		onclick={() => onset(!staffs)}
	>
		<!-- The mark, and not only the brighter ink: whether a role staffs a loop is a state,
		     and a state is never carried by colour alone. -->
		<span class="mark"
			>{#if staffs}<Icon name="check" />{/if}</span
		>
		staffs it
	</button>
{:else}
	<span class="meaning">a role that may not emit on a loop cannot staff it</span>
{/if}

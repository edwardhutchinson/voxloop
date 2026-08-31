<script>
	// The operating console: what somebody who has assumed a role is looking at.
	//
	// It renders the **presence document** and nothing else. The document is the API
	// (ADR-0019): whatever is on this page came out of it, and anything in it is something
	// the server has committed to keeping true — so this page never computes a state, never
	// merges one document into another, and never renders optimistically.
	//
	// **What is here is #37's half of the console and no more**: the role, the session it is
	// bound to, and the loops in reach. The board and ledger over that loop list is #38, the
	// subscriptions are #39 and the transmit bar is #41. The list is deliberately plain
	// rather than dressed as a console that does not exist yet.
	//
	// **Changing role is a relinquish followed by an assume** (v1 §2), so there is no role
	// picker here and no *switch*. Relinquishing lands in the lobby, and the lobby is where a
	// role is taken up. Audio genuinely stops in between, and offering a control that hid
	// that would be the class of lie this product exists to avoid.
	let { presence, lost, refused, onRelinquish } = $props();
</script>

<section>
	<header>
		<h2>{presence.role.name}</h2>
		<p>
			You have assumed this role and hold its authority. To take up another, relinquish this one
			first — audio stops, and your subscriptions and arms go with it.
		</p>
	</header>

	{#if lost}
		<p class="lost" role="alert">
			The connection to VoxLoop was lost. This is what it last said, and it is not being kept up to
			date.
		</p>
	{/if}

	{#if refused}
		<p class="refusal" role="alert">{refused}</p>
	{/if}

	{#if presence.loops.length === 0}
		<p class="quiet">
			This role reaches no loops. Reach is one cell on the grid per loop, set by a system
			administrator, and a role may be assumed with an empty row.
		</p>
	{:else}
		<table>
			<thead>
				<tr>
					<th>Loop</th>
					<th>This role holds</th>
				</tr>
			</thead>
			<tbody>
				{#each presence.loops as reachable (reachable.id)}
					<tr>
						<td>{reachable.name}</td>
						<td>{reachable.permission}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	{/if}

	<p class="relinquish">
		<button class="destructive" onclick={onRelinquish}>Relinquish {presence.role.name}</button>
	</p>
</section>

<style>
	/* Below the loops rather than beside the heading. Relinquishing is a full stop and the
	   one act on this page, and putting it in the header would place the way off the air
	   next to the name of the role somebody just took. */
	.relinquish {
		margin: var(--space-5) 0 0;
	}
</style>

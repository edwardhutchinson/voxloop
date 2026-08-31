<script>
	// The lobby: what a signed-in user with no role assumed is looking at.
	//
	// It answers one question — *should I assume a role, and which?* — and deliberately
	// nothing else (ADR-0023). No audio, no authority, no talking indicators, no
	// configuration. The roles this person may assume and who is in each seat is the whole
	// of it, and the staffing state of the loops those roles staff joins it later, carried
	// in full rather than as a word: the lobby is read once and deliberately, by somebody
	// about to be in a position to fix what it says.
	//
	// Assuming a role is the act this page is for and it is not built yet, so this page does
	// not pretend otherwise: there is no button here that does nothing.
	//
	// The document arrives from the frame rather than from a socket of this page's own,
	// because the socket belongs to the tab and is opened at sign-in — an administrator
	// reading the admin console has not left the lobby, and their socket has not closed.
	let { lobby, lost } = $props();

	// A seat nobody is in is the answer this page exists to give, so it is said in words
	// rather than left as an empty cell.
	const occupancy = (role) => (role.occupants.length ? role.occupants.join(', ') : 'Nobody');

	const limit = (role) => (role.max_occupants === null ? 'No limit' : role.max_occupants);
</script>

<section>
	<header>
		<h2>Lobby</h2>
		<p>
			You are signed in and have assumed no role, so you have no audio and no authority. These are
			the roles you may assume, and who is in each.
		</p>
	</header>

	{#if lost}
		<p class="lost" role="alert">
			The connection to VoxLoop was lost. This is what it last said, and it is not being kept up to
			date.
		</p>
	{/if}

	{#if !lobby}
		<p class="quiet">Asking VoxLoop…</p>
	{:else if lobby.roles.length === 0}
		<p class="quiet">
			You are eligible for no roles. Eligibility is granted by a system administrator, and it is
			what lets you take up a position.
		</p>
	{:else}
		<table>
			<thead>
				<tr>
					<th>Role</th>
					<th>Occupied by</th>
					<th>Max occupants</th>
				</tr>
			</thead>
			<tbody>
				{#each lobby.roles as role (role.id)}
					<tr>
						<td>{role.name}</td>
						<td class:quiet={role.occupants.length === 0}>{occupancy(role)}</td>
						<td class:quiet={role.max_occupants === null}>{limit(role)}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	{/if}
</section>

<style>
	/* Losing the signalling channel is not a refusal and not a destructive act, so it is not
	   written like one: what is on screen is frozen and marked rather than blanked, because
	   an empty page reads as *nothing is happening* when everything may be (ADR-0018). The
	   sentence carries the state; the colour only makes it hard to walk past. The three-state
	   ladder this is the first step of arrives with the console's connection state. */
	.lost {
		margin: 0 0 var(--space-4);
		color: var(--warning);
	}
</style>

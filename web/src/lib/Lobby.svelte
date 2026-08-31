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
	// **Assuming is the act this page is for**, and it is the only one on it. An occupied
	// single-occupant seat is offered as unavailable and says so in words rather than being
	// hidden: a role missing from this list would be indistinguishable from one nobody made
	// this person eligible for, and those are different facts. Asking its incumbent for it is
	// a takeover request and arrives with #50.
	//
	// The document arrives from the frame rather than from a socket of this page's own,
	// because the socket belongs to the tab and is opened at sign-in — an administrator
	// reading the admin console has not left the lobby, and their socket has not closed.
	let { lobby, lost, refused, relinquished, onAssume } = $props();

	// A seat nobody is in is the answer this page exists to give, so it is said in words
	// rather than left as an empty cell.
	const occupancy = (role) => (role.occupants.length ? role.occupants.join(', ') : 'Nobody');

	const limit = (role) => (role.max_occupants === null ? 'No limit' : role.max_occupants);

	// A seat is full when as many people are in it as it holds. A role with no limit is the
	// limit left unset rather than a role that is never full (ADR-0068), so the same
	// comparison answers both.
	const isFull = (role) =>
		role.max_occupants !== null && role.occupants.length >= role.max_occupants;
</script>

<section>
	<header>
		<h2>Lobby</h2>
		<p>
			You are signed in and have assumed no role, so you have no audio and no authority. These are
			the roles you may assume, and who is in each.
		</p>
	</header>

	{#if relinquished}
		<!-- Said before the lobby was rendered, and shown here: a console that merely
		     reappeared in the lobby would leave the operator to work out that their audio
		     stopped and why (v1 §2). -->
		<p class="ended" role="status">{relinquished}</p>
	{/if}

	{#if lost}
		<p class="lost" role="alert">
			The connection to VoxLoop was lost. This is what it last said, and it is not being kept up to
			date.
		</p>
	{/if}

	{#if refused}
		<p class="refusal" role="alert">{refused}</p>
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
					<th class="acts">Assume</th>
				</tr>
			</thead>
			<tbody>
				{#each lobby.roles as role (role.id)}
					<tr>
						<td>{role.name}</td>
						<td class:quiet={role.occupants.length === 0}>{occupancy(role)}</td>
						<td class:quiet={role.max_occupants === null}>{limit(role)}</td>
						<td class="acts">
							{#if isFull(role)}
								<span class="quiet">Occupied</span>
							{:else}
								<button onclick={() => onAssume(role.id)}>Assume</button>
							{/if}
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	{/if}
</section>

<style>
	/* A session that ended is not a refusal and not a warning: it is the ordinary end of a
	   shift, or a role taken up on another machine, and it is told once. It reads as the
	   quiet statement of fact it is, and the words carry it — nothing here is colour alone. */
	.ended {
		margin: 0 0 var(--space-4);
		color: var(--quiet);
	}

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

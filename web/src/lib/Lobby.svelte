<script>
	// The lobby: what a signed-in user with no role assumed is looking at.
	//
	// It answers one question — *should I assume a role, and which?* — and deliberately
	// nothing else (ADR-0023). No audio, no authority, no talking indicators, no
	// configuration. The roles this person may assume, who is in each seat, and **the
	// staffing state of the loops those roles staff, with the reason in full** — ledger-style
	// rather than board-style, because the lobby is read once and deliberately, by somebody
	// about to be in a position to fix what it says. `away — 2 not subscribed` tells them the
	// seat's loops need setting up before they take it.
	//
	// It is not reach: a role reaches loops it does not staff, and those are not this page's
	// business. The sentence is `staffing.js`'s, the same one the ledger says, because a
	// second implementation is how two surfaces come to disagree about whether somebody is
	// behind a loop.
	//
	// **Assuming is the act this page is for**, and it is the only one on it. An occupied
	// single-occupant seat is offered as unavailable and says so in words rather than being
	// hidden: a role missing from this list would be indistinguishable from one nobody made
	// this person eligible for, and those are different facts. Asking its occupant for it is
	// a takeover request and arrives with #50.
	//
	// The document arrives from the frame rather than from a socket of this page's own,
	// because the socket belongs to the tab and is opened at sign-in — an administrator
	// reading the admin console has not left the lobby, and their socket has not closed.
	import { theSentence } from './staffing.js';

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
					<th>Loops it staffs</th>
					<th class="acts">Assume</th>
				</tr>
			</thead>
			<tbody>
				{#each lobby.roles as role (role.id)}
					<tr>
						<td>{role.name}</td>
						<td class:quiet={role.occupants.length === 0}>{occupancy(role)}</td>
						<td class:quiet={role.max_occupants === null}>{limit(role)}</td>
						<td>
							<!-- A role that staffs nothing says so, rather than leaving a cell empty:
							     *this seat answers for no loop* is an answer, and a blank is not. -->
							{#if role.staffs.length === 0}
								<span class="quiet">None</span>
							{:else}
								<ul>
									{#each role.staffs as held_on (held_on.id)}
										<li>
											<span class="name">{held_on.name}</span>
											<span class="meaning" class:nobody={held_on.staffing.state !== 'staffed'}>
												{theSentence(held_on.staffing)}
											</span>
										</li>
									{/each}
								</ul>
							{/if}
						</td>
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
	/* The loops one seat answers for, listed down its own cell. There is no bullet and no
	   indent: it is a list because it is one, and a row of a table is not the place for a
	   second level of furniture. */
	ul {
		margin: 0;
		padding: 0;
		list-style: none;
	}

	li + li {
		margin-top: var(--space-2);
	}
</style>

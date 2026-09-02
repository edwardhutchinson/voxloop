<script>
	// The ledger: a compact table row per loop in reach, and the reading view (ADR-0032). It
	// holds the same loops as the board, in the same order, and it is where state too long
	// for a card lives — the staffing reason above all, when #48 brings it.
	//
	// What it spells out today is the rung: the board says `emit`, and here the row says what
	// `emit` lets this role do with the loop. Each rung carries the ones below it (ADR-0011),
	// so the sentence grows rather than changes.
	//
	// **It renders the loops it is handed, in the order it is handed them**, for the reason
	// `Board.svelte` gives: two independent orders would put the same loop third in one view
	// and eleventh in the other.
	import TransmitBar from './TransmitBar.svelte';

	let { loops, mediaPath } = $props();

	// A rung is a word on the board and a sentence here. A rung this does not know is shown
	// as the word the document used: the grid is the only thing entitled to say what a role
	// holds, and rendering nothing would be the console dropping a fact it was given.
	const confers = {
		monitor: 'Hear it',
		emit: 'Hear it, and speak on it',
		control: 'Hear it, speak on it, and hold authority on it'
	};
</script>

<!-- Never scrolled away (ADR-0034). Here it rides above the rows rather than under them: the
     ledger is read top-down from its header, and a bar under a table of unknown length reads
     as that table's footer rather than as a fixture of the console. -->
<div class="transmit">
	<TransmitBar {mediaPath} />
</div>

<table>
	<thead>
		<tr>
			<th>Loop</th>
			<th>This role may</th>
		</tr>
	</thead>
	<tbody>
		{#each loops as reachable (reachable.id)}
			<tr>
				<td>{reachable.name}</td>
				<td>{confers[reachable.permission] ?? reachable.permission}</td>
			</tr>
		{/each}
	</tbody>
</table>

<style>
	.transmit {
		position: sticky;
		top: 0;
		background: var(--ground);
		border-bottom: 1px solid var(--rule);
		padding: var(--space-3) 0;
		margin-bottom: var(--space-3);
	}
</style>

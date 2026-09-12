<script>
	// The ledger: a compact table row per loop in reach, and the reading view (ADR-0032). It
	// holds the same loops as the board, in the same order, and it is where state too long
	// for a card lives — **the staffing reason above all** (#48).
	//
	// What it spells out today is the rung and the subscription. The board says `emit` and
	// `Monitoring`; here the row says what `emit` lets this role do with the loop, and what
	// monitoring it means in the operator's own ears. Each rung carries the ones below it
	// (ADR-0011), so the sentence grows rather than changes.
	//
	// **It renders the loops it is handed, in the order it is handed them**, for the reason
	// `Board.svelte` gives: two independent orders would put the same loop third in one view
	// and eleventh in the other.
	//
	// **The act is a control in the row rather than the row itself.** The board's card is the
	// click target because v1 §8 makes it one; a table row is not a control, and a row that
	// swallowed clicks would take the cog and the mute down with it. It is the same act either
	// way, and `Console.svelte` decides which of the two messages a click is — so the two views
	// cannot come to disagree about what a click means.
	//
	// **Mute and volume are sentences here** (#44). The mute says what the card's one word
	// cannot: the loop is still monitored and nobody else is affected. The volume is said on
	// every row, at unity too, because this is the reading view and a column that went blank
	// for most rows would read as a column with nothing in it. Setting it is behind the cog, as
	// on the card (ADR-0034), and the modal it opens is `Console.svelte`'s.
	//
	// **Loop health is a sentence on every monitored row** (#46), the ordinary case included:
	// this is the reading view, and it is where the gap v1 §16 records can be said — the
	// sentence for a loop being received says the loop reaches you, and no more, because a
	// beacon arriving does not prove any given talker would be heard.
	//
	// **Every state the board carries is carried here too**, which from this ticket on means
	// the arm, the blind arm and the talking indicator. The indicator is the one thing that is
	// literally the same object in both views, because it is one component (ADR-0033) — what
	// differs is that the board says `Not hearing it` beside a blind arm and this says what
	// that means in a sentence.
	import Icon from './Icon.svelte';
	import Talking from './Talking.svelte';
	import TransmitBar from './TransmitBar.svelte';
	import { carries } from './rungs.js';
	import { theSentence } from './staffing.js';

	// `bar` is everything the transmit bar renders, handed over whole and never read here.
	// **Placing it is this view's business and wording it is the bar's** (ADR-0034), so this
	// view has no name for any of what is in it: a state that arrives as one value cannot be
	// half-forwarded, and adding one to the bar is not an edit to either view.
	let { loops, bar, onToggle, onArm, onMute, onCog } = $props();

	// Which loops carry an arm control at all. **Reach is the grid and only the grid**: a role
	// that may hear a loop and not speak on it gets no control, rather than one that is
	// refused when pressed (ADR-0016). The rung is read through `rungs.js`, which both views
	// share, because it is the grid's rule rather than either view's.
	const mayEmit = (reachable) => carries(reachable.permission, 'emit');

	// A rung is a word on the board and a sentence here. A rung this does not know is shown
	// as the word the document used: the grid is the only thing entitled to say what a role
	// holds, and rendering nothing would be the console dropping a fact it was given.
	const confers = {
		monitor: 'Hear it',
		emit: 'Hear it, and speak on it',
		control: 'Hear it, speak on it, and hold authority on it'
	};

	// **Staffing state is a sentence here and a word on the board** (v1 §8). The reason is
	// counted over occupants where they differ — `away — 1 muted, 2 not subscribed` — and it
	// collapses to the plain sentence where they agree. Neither the wording nor the
	// collapsing is this view's: `staffing.js` holds both, because the lobby says the same
	// thing and a second implementation is how two surfaces come to disagree.
	//
	// The mark is a sentence too, and it is here in both its states for the reason it is on
	// the card in both: it is a fact about this operator's own console rather than an alarm.

	// Loop health in a sentence. A reading this does not know is said as nothing rather than
	// guessed at: the server is the only thing entitled to judge a loop received (ADR-0017).
	const reaches = {
		receiving: 'Its beacon is arriving, so this loop reaches you.',
		checking: 'Checking that this loop reaches you.',
		'not-receiving':
			'Its beacon is not arriving, so you may not hear this loop even when somebody talks on it.'
	};
</script>

<!-- Never scrolled away (ADR-0034). Here it rides above the rows rather than under them: the
     ledger is read top-down from its header, and a bar under a table of unknown length reads
     as that table's footer rather than as a fixture of the console. -->
<div class="transmit">
	<TransmitBar {...bar} />
</div>

<table>
	<thead>
		<tr>
			<th>Loop</th>
			<th>This role may</th>
			<th>Monitoring</th>
			<th>Reaching you</th>
			<th>Volume</th>
			<th>Staffing</th>
			<th>Emitting to</th>
		</tr>
	</thead>
	<tbody>
		{#each loops as reachable (reachable.id)}
			<tr>
				<td>{reachable.name}</td>
				<td>{confers[reachable.permission] ?? reachable.permission}</td>
				<td>
					<button onclick={() => onToggle(reachable)}>
						{reachable.subscribed ? 'Stop monitoring' : 'Monitor'}
					</button>
					<!-- The state as a sentence, under the control that changes it. The button
					     names the act and this names what is true now, so neither has to be read
					     as the other — and the state is never carried by the button's wording
					     alone. -->
					<span class="meaning">
						{#if reachable.subscribed && reachable.muted}
							You have muted this loop. It is still monitored, and nobody else is affected.
						{:else if reachable.subscribed}
							You are hearing this loop.
						{:else}
							You are not hearing this loop.
						{/if}
					</span>
					{#if reachable.subscribed}
						<!-- Only on a loop being monitored: a mute presupposes a subscription
						     (ADR-0049). -->
						<button aria-pressed={reachable.muted} onclick={() => onMute(reachable)}>
							{#if reachable.muted}
								<Icon name="volume-2" /> Unmute
							{:else}
								<Icon name="volume-x" /> Mute
							{/if}
						</button>
					{/if}
					{#if reachable.talking}
						<Talking priority={reachable.priority} />
					{/if}
				</td>
				<td>
					<!-- Only on a loop being monitored: there is no beacon counted on any other.
					     And nothing while the document has no reading, which is a channel not
					     confirmed — the console already says that, above both views (v1 §6). -->
					{#if reachable.subscribed && reaches[reachable.health]}
						<span class="meaning" class:unreceived={reachable.health === 'not-receiving'}>
							{reaches[reachable.health]}
						</span>
					{/if}
				</td>
				<td>
					<span class="meaning">
						{reachable.volume < 100
							? `Plays at ${reachable.volume}% of full volume.`
							: 'Plays at full volume.'}
					</span>
					<button aria-label="Volume for {reachable.name}" onclick={() => onCog(reachable)}>
						<Icon name="settings" />
					</button>
				</td>
				<td>
					<!-- Nothing at all where the loop has no staffing roles: the absence of a
					     staffing state is not a fourth state and does not read as one
					     (ADR-0056). -->
					{#if theSentence(reachable.staffing)}
						<span class="meaning" class:nobody={reachable.staffing.state !== 'staffed'}>
							{theSentence(reachable.staffing)}
						</span>
					{/if}
					{#if reachable.staffs}
						<!-- The sentence the card has no room for. The second state is the
						     actionable one, and what fixes it is the control in the Monitoring
						     column of this same row. -->
						<span class="meaning">
							{#if reachable.subscribed}
								You staff this loop, and you are hearing it.
							{:else}
								You staff this loop and it is not on your console. Monitor it to answer for it.
							{/if}
						</span>
					{/if}
				</td>
				<td>
					{#if mayEmit(reachable)}
						<button aria-pressed={reachable.armed} onclick={() => onArm(reachable)}>
							{reachable.armed ? 'Disarm' : 'Arm'}
						</button>
						<!-- The sentence the card has no room for. **Arming is independent of
						     subscription** (ADR-0013), so an armed loop somebody is not hearing
						     is a legal state rather than a mistake — and the sentence says what
						     it costs rather than warning about it. -->
						<span class="meaning">
							{#if reachable.armed && !reachable.subscribed}
								Your voice goes here and you are not hearing it.
							{:else if reachable.armed}
								Your voice goes here when you key.
							{:else}
								Your voice does not go here.
							{/if}
						</span>
					{:else}
						<span class="meaning">This role may not speak on this loop.</span>
					{/if}
				</td>
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

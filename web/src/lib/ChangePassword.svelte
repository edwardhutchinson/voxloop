<script>
	// Changing one's own password, by re-presenting the current one.
	//
	// It does not end the session, and the panel says so: an operator on the air is entitled
	// to know that this will not take their audio away before they click it. Every other act
	// on a password in VoxLoop ends every sign-in, so the exception is worth stating.
	import { changePassword, whatWentWrong } from './server.js';

	let current = $state('');
	let next = $state('');
	let again = $state('');
	let refusal = $state(null);
	let done = $state(false);
	let attempting = $state(false);

	async function attempt(event) {
		event.preventDefault();
		done = false;

		if (next !== again) {
			refusal = 'Those two passwords are not the same.';
			return;
		}

		attempting = true;
		refusal = null;
		try {
			await changePassword(current, next);
			current = '';
			next = '';
			again = '';
			done = true;
		} catch (said) {
			refusal = whatWentWrong(said);
		} finally {
			attempting = false;
		}
	}
</script>

<section>
	<h2>Your password</h2>
	<p class="quiet">
		Re-present the one you have now. You stay signed in and, if you have assumed a role, on the air.
	</p>

	<form onsubmit={attempt}>
		<label class="field">
			Current
			<input type="password" bind:value={current} autocomplete="current-password" required />
		</label>
		<label class="field">
			New
			<input type="password" bind:value={next} autocomplete="new-password" required />
		</label>
		<label class="field">
			New again
			<input type="password" bind:value={again} autocomplete="new-password" required />
		</label>
		<button type="submit" disabled={attempting}>{attempting ? 'Changing…' : 'Change'}</button>
	</form>

	{#if refusal}
		<p class="refusal" role="alert">{refusal}</p>
	{:else if done}
		<p class="quiet" role="status">Changed. You are still signed in.</p>
	{/if}
</section>

<style>
	section {
		margin-top: var(--space-6);
		padding-top: var(--space-6);
		border-top: 1px solid var(--rule);
	}

	form {
		display: flex;
		gap: var(--space-3);
		/* Each field is a label stacked over its input, so the row is aligned on its bottom
		   edge: the button then sits on the inputs' line rather than halfway up the labels. */
		align-items: end;
		margin-top: var(--space-5);
		flex-wrap: wrap;
	}

	.refusal {
		margin: var(--space-3) 0 0;
	}
</style>

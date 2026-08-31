<script>
	import { signIn, whatWentWrong } from './server.js';

	// A line the page above wants shown here — after an enrolment code has just been
	// redeemed, there is nothing to refuse and something to say.
	let { onSignedIn, note = null } = $props();

	let username = $state('');
	let password = $state('');
	let refusal = $state(null);
	let attempting = $state(false);

	async function attempt(event) {
		event.preventDefault();
		attempting = true;
		refusal = null;
		try {
			await signIn(username, password);
			password = '';
			await onSignedIn();
		} catch (said) {
			refusal = whatWentWrong(said);
		} finally {
			attempting = false;
		}
	}
</script>

<form onsubmit={attempt}>
	<h1>VoxLoop</h1>

	<label>
		Username
		<input bind:value={username} autocomplete="username" required />
	</label>

	<label>
		Password
		<input type="password" bind:value={password} autocomplete="current-password" required />
	</label>

	<button type="submit" disabled={attempting}>{attempting ? 'Signing in…' : 'Sign in'}</button>

	{#if refusal}
		<p class="refusal" role="alert">{refusal}</p>
	{:else if note}
		<p class="quiet" role="status">{note}</p>
	{/if}
</form>

<style>
	form {
		display: grid;
		gap: var(--space-4);
		max-width: 22rem;
		margin: var(--space-6) auto 0;
	}

	h1 {
		margin: 0;
		font-size: var(--type-5);
		letter-spacing: 0.02em;
	}

	label {
		display: grid;
		gap: var(--space-1);
		font-size: var(--type-2);
		color: var(--quiet);
	}

	.refusal {
		margin: 0;
	}

	.quiet {
		margin: 0;
	}
</style>

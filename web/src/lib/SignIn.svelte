<script>
	import { Refused, signIn } from './server.js';

	let { onSignedIn } = $props();

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
			refusal = said instanceof Refused ? said.message : 'VoxLoop could not answer that.';
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
	{/if}
</form>

<style>
	form {
		display: grid;
		gap: 1rem;
		max-width: 22rem;
		margin: 4rem auto;
	}

	h1 {
		margin: 0;
		font-size: 1.5rem;
		letter-spacing: 0.02em;
	}

	label {
		display: grid;
		gap: 0.35rem;
		font-size: 0.85rem;
		color: var(--quiet);
	}

	.refusal {
		margin: 0;
		color: var(--refusal);
	}
</style>

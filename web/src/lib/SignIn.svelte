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

<form class="wayin" onsubmit={attempt}>
	<h1>VoxLoop</h1>

	<label class="field">
		Username
		<input bind:value={username} autocomplete="username" required />
	</label>

	<label class="field">
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

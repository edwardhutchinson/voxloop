<script>
	// Redeeming an enrolment code: the only way a password is ever set.
	//
	// There is no username field and there is not going to be one. The code identifies the
	// user, which is what makes this safe to leave open to anybody: there is nothing here to
	// aim at somebody else's account and nothing to enumerate.
	import { redeemEnrolment, whatWentWrong } from './server.js';

	let { onEnrolled, onBack } = $props();

	let code = $state('');
	let password = $state('');
	let again = $state('');
	let refusal = $state(null);
	let attempting = $state(false);

	async function attempt(event) {
		event.preventDefault();

		if (password !== again) {
			refusal = 'Those two passwords are not the same.';
			return;
		}

		attempting = true;
		refusal = null;
		try {
			await redeemEnrolment(code.trim(), password);
			code = '';
			password = '';
			again = '';
			onEnrolled();
		} catch (said) {
			refusal = whatWentWrong(said);
		} finally {
			attempting = false;
		}
	}
</script>

<form onsubmit={attempt}>
	<h1>Set your password</h1>
	<p class="quiet">
		An administrator issues an enrolment code and hands it over in person. It is good once. VoxLoop
		has no mail path, so this is how a password is set and how one is reset.
	</p>

	<label>
		Enrolment code
		<input bind:value={code} autocomplete="off" spellcheck="false" required />
	</label>

	<label>
		New password
		<input type="password" bind:value={password} autocomplete="new-password" required />
	</label>

	<label>
		New password again
		<input type="password" bind:value={again} autocomplete="new-password" required />
	</label>

	<p class="quiet">At least twelve characters. There are no other rules.</p>

	<button type="submit" disabled={attempting}>{attempting ? 'Setting…' : 'Set password'}</button>
	<button type="button" class="plain" onclick={onBack}>Back to signing in</button>

	{#if refusal}
		<p class="refusal" role="alert">{refusal}</p>
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

	.quiet {
		margin: 0;
	}

	.plain {
		background: none;
		border: 0;
		color: var(--quiet);
		text-decoration: underline dotted;
	}

	.refusal {
		margin: 0;
	}
</style>

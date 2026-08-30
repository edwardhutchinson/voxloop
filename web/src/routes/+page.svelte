<script>
	let reached = $state('asking…');

	$effect(() => {
		fetch('/api/liveness')
			.then((answer) => (answer.ok ? answer.text() : Promise.reject(answer.status)))
			.then((answer) => (reached = answer))
			.catch((refusal) => (reached = `no answer (${refusal})`));
	});
</script>

<main>
	<h1>VoxLoop</h1>
	<p>The server says: {reached}</p>
</main>

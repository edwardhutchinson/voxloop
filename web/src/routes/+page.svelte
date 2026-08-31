<script>
	// The frame's own page, and **the one surface with no URL of its own** — which is the
	// point of the split (#76). Whether somebody is looking at the lobby or at an operating
	// console is not a place they navigated to: it is whether they hold a role, and that is
	// live state the server resolves from the state authority. A URL asserting it would be a
	// claim about a session, and when the server disagreed the console would have to bounce
	// them somewhere else — a URL asserting a state nobody observed, which is the class of lie
	// ADR-0016 exists to stop. Reloading asks the server where you are and lands you there.
	//
	// Assuming a role is not built yet, so today the answer is always the lobby.
	import Lobby from '$lib/Lobby.svelte';
	import { theFrame } from '$lib/frame.js';

	const frame = theFrame();
</script>

<Lobby lobby={frame.lobby} lost={frame.lost} refused={frame.refused} />

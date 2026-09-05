<script>
	// The frame's own page, and **the one surface with no URL of its own** — which is the
	// point of the split (#76). Whether somebody is looking at the lobby or at an operating
	// console is not a place they navigated to: it is whether they hold a role, and that is
	// live state the server resolves from the state authority. A URL asserting it would be a
	// claim about a session, and when the server disagreed the console would have to bounce
	// them somewhere else — a URL asserting a state nobody observed, which is the class of lie
	// ADR-0016 exists to stop. Reloading asks the server where you are and lands you there.
	//
	// **The document decides which of the two is on screen.** A presence document means a
	// role is held; its absence means the lobby. Nothing here remembers which was last shown,
	// because a console that remembered would keep rendering a session the server had ended.
	import Console from '$lib/Console.svelte';
	import Lobby from '$lib/Lobby.svelte';
	import { theFrame } from '$lib/frame.js';

	const frame = theFrame();
</script>

{#if frame.presence}
	<Console
		presence={frame.presence}
		lost={frame.lost}
		refused={frame.refused}
		onRelinquish={frame.relinquish}
		onSubscribe={frame.subscribe}
		onUnsubscribe={frame.unsubscribe}
		onArm={frame.arm}
		onDisarm={frame.disarm}
		onKeying={frame.keying}
	/>
{:else}
	<Lobby
		lobby={frame.lobby}
		lost={frame.lost}
		refused={frame.refused}
		relinquished={frame.relinquished}
		onAssume={frame.assume}
	/>
{/if}

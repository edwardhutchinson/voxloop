<script>
	// One component and one data file is the whole icon mechanism (ADR-0069).
	//
	// An icon carries no size and no colour of its own: it is `1em` wide and stroked in
	// `currentColor`, so it takes both from whatever it sits beside and never needs a token
	// or a variant. `icons.js` holds the shapes; everything that makes them look like one set
	// is the wrapper here.
	import { icons } from './icons.js';

	// `label` is the exception rather than the rule. Most icons here sit beside text that
	// already says the thing, and reading it twice is worse than not reading it at all.
	let { name, label = null } = $props();
</script>

<svg
	viewBox="0 0 24 24"
	fill="none"
	stroke="currentColor"
	stroke-width="2"
	stroke-linecap="round"
	stroke-linejoin="round"
	aria-hidden={label ? undefined : 'true'}
	role={label ? 'img' : undefined}
	aria-label={label}
>
	<!-- Keyed because ESLint asks for one, and on the attributes because each is a distinct
	     object in a module constant that is built once and never rebuilt. -->
	{#each icons[name] ?? [] as [shape, attributes] (attributes)}
		<svelte:element this={shape} {...attributes} />
	{/each}
</svg>

<style>
	svg {
		width: 1em;
		height: 1em;
		/* An icon on a text baseline sits high, because `1em` is the whole line box and the
		   glyphs beside it do not fill it. This is the nudge that puts it back on the line;
		   an icon in a flex row is already centred and unaffected by it. */
		vertical-align: -0.15em;
	}
</style>

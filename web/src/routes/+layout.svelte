<script>
	// The one place the token file is pulled in (ADR-0069). Every route is inside this, so a
	// component never imports a stylesheet and cannot forget to.
	import '../app.css';

	let { children } = $props();
</script>

{@render children()}

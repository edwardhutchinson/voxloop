<script>
	// The transmit bar: the console's answer to *who am I about to talk to*, and — because it
	// stays live while the key is held — to *who am I talking to* (ADR-0034, ADR-0058).
	//
	// **It is one component so that it is one wording.** Both views carry it and each puts it
	// somewhere different, and the thing that must not vary between them is what it says: the
	// bar is the whole of VoxLoop's compensation for emitting to several places at once, and a
	// board and a ledger disagreeing about the audience would be worse than neither saying
	// anything. Placing it is the view's business; wording it is this file's, and there is
	// nowhere else to write a word of it.
	//
	// **It is empty at this point**, and says so rather than rendering a strip that reads as a
	// control room console with its transmitting lamp out. The armed set in words and the key
	// state arrive with #41, the two audience counts with #49.
	//
	// The act it is about is **emission** rather than *transmit*, which `CONTEXT.md` avoids;
	// the strip itself is the *transmit bar*, which is the glossary's own name for it.
</script>

<section aria-label="Transmit bar">
	<p class="quiet">
		VoxLoop cannot emit yet. The armed set, the audience and the key control belong here.
	</p>
</section>

// Rendering a component in a test, without a component test runner.
//
// The console has no jsdom, no testing-library and no second runner: it has the Svelte
// compiler, which is already a dependency, and `svelte/server`, which renders a component to
// the HTML a server would send. That answers the questions worth asking of these components —
// what is on the page, in what order, and whether two views of one document agree — for one
// import rather than a dependency tree.
//
// Everything in `src/lib/` is compiled together, because a component that renders another
// needs it beside it. The routes are not: a page is the composition of these rather than a
// thing with props of its own, and their filenames collide anyway — there are several
// `+page.svelte`.

import { mkdirSync, writeFileSync } from 'node:fs';
import { basename, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { compile } from 'svelte/compiler';
import { render } from 'svelte/server';
import { read, src, under } from './console.js';

const lib = join(src, 'lib');
// `.svelte-kit/` is the generated directory: git, Prettier and ESLint all ignore it.
const built = join(src, '..', '.svelte-kit', 'tests');

// The compiled components sit outside `src/`, so every specifier they carry has to be
// pointed back at the file it meant. A sibling component is beside them here too; anything
// else — `icons.js`, `server.js` — was left where it was.
const pointedBackAtTheSource = (code) =>
	code.replaceAll(/'\.\/([A-Za-z0-9.-]+)'/g, (_, file) =>
		file.endsWith('.svelte') ? `'./${file}.js'` : JSON.stringify(join(lib, file))
	);

let done = false;

function compileTheLibrary() {
	if (done) return;
	mkdirSync(built, { recursive: true });

	for (const path of under(/\.svelte$/, lib)) {
		const { js } = compile(read(path), { generate: 'server', filename: basename(path) });
		writeFileSync(join(built, `${basename(path)}.js`), pointedBackAtTheSource(js.code));
	}
	done = true;
}

/** One compiled component, by the name of its file. */
export async function component(name) {
	compileTheLibrary();
	return (await import(pathToFileURL(join(built, `${name}.js`)).href)).default;
}

/** What a component renders, given its props: the HTML, as a server would send it. */
export async function rendered(name, props = {}) {
	// Without the anchors Svelte wraps a whole render in. They are how hydration finds the
	// fragment it is picking up, so they belong to the render rather than to the component,
	// and one component's markup is then literally a substring of another's that holds it —
	// which is how two views are checked for saying the same words.
	return render(await component(name), { props })
		.body.replace(/^<!--\[-->/, '')
		.replace(/<!--\]-->$/, '');
}

// What every console test file needs: where the source is, and how to read one.

import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const src = fileURLToPath(new URL('../src/', import.meta.url));

/** Everything under `src/` whose name matches — components, or the modules beside them. */
export function under(matching, dir = src) {
	return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
		const path = join(dir, entry.name);
		if (entry.isDirectory()) return under(matching, path);
		return matching.test(entry.name) ? [path] : [];
	});
}

export const components = () => under(/\.svelte$/);

// Paths are reported relative to `src/`, because that is how a component is talked about.
export const named = (path) => path.slice(src.length);

export const read = (path) => readFileSync(path, 'utf8');

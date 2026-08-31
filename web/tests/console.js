// What both console test files need: where the components are, and how to read one.

import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const src = fileURLToPath(new URL('../src/', import.meta.url));

export function components(dir = src) {
	return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
		const path = join(dir, entry.name);
		if (entry.isDirectory()) return components(path);
		return entry.name.endsWith('.svelte') ? [path] : [];
	});
}

// Paths are reported relative to `src/`, because that is how a component is talked about.
export const named = (path) => path.slice(src.length);

export const read = (path) => readFileSync(path, 'utf8');

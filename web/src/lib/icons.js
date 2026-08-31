// The console's icons, hand-picked from Lucide rather than pulled as a dependency. The
// console needs about twenty of them, and one file is also the place to audit what the
// product actually uses: an icon with no caller is deleted rather than kept in case.
//
// Each entry is the shapes inside Lucide's 24×24 viewBox, copied unaltered. The wrapper —
// the viewBox, the stroke width, the line caps — lives in `Icon.svelte`, so an icon here is
// only what makes it that icon. `docs/agents/styling.md` has the procedure for adding one.
//
// ---
//
// ISC License
//
// Copyright (c) 2026 Lucide Icons and Contributors
//
// Permission to use, copy, modify, and/or distribute this software for any purpose with or
// without fee is hereby granted, provided that the above copyright notice and this
// permission notice appear in all copies.
//
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO
// THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT
// SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR
// ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
// OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE
// USE OR PERFORMANCE OF THIS SOFTWARE.
//
// ---
//
// Of the icons below, `arrow-down`, `arrow-left`, `arrow-up`, `check`, `chevron-down`,
// `chevron-right`, `plus`, `trash-2`, `triangle-alert` and `x` are derived from the Feather
// project and carry its licence as well. `triangle-alert` appears in Lucide's own LICENSE
// under its former name, `alert-triangle`, which is why checking that list needs care rather
// than a text search.
//
// The MIT License (MIT)
//
// Copyright (c) 2013-present Cole Bemis
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of this
// software and associated documentation files (the "Software"), to deal in the Software
// without restriction, including without limitation the rights to use, copy, modify, merge,
// publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
// to whom the Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all copies or
// substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED,
// INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
// PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE
// FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
// OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

export const icons = {
	// The board and the loop list (v1 §8): mute, the per-loop volume cog, reordering, and
	// dismissing the mark on a loop that has just entered reach.
	'volume-2': [
		[
			'path',
			{
				d: 'M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z'
			}
		],
		['path', { d: 'M16 9a5 5 0 0 1 0 6' }],
		['path', { d: 'M19.364 18.364a9 9 0 0 0 0-12.728' }]
	],
	'volume-x': [
		[
			'path',
			{
				d: 'M11 4.702a.7.7 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.7.7 0 0 0 11 19.298z'
			}
		],
		['path', { d: 'm16.5 14.5 5-5' }],
		['path', { d: 'm16.5 9.5 5 5' }]
	],
	settings: [
		[
			'path',
			{
				d: 'M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915'
			}
		],
		['circle', { cx: '12', cy: '12', r: '3' }]
	],
	'grip-vertical': [
		['circle', { cx: '9', cy: '12', r: '1' }],
		['circle', { cx: '9', cy: '5', r: '1' }],
		['circle', { cx: '9', cy: '19', r: '1' }],
		['circle', { cx: '15', cy: '12', r: '1' }],
		['circle', { cx: '15', cy: '5', r: '1' }],
		['circle', { cx: '15', cy: '19', r: '1' }]
	],
	x: [
		['path', { d: 'M18 6 6 18' }],
		['path', { d: 'm6 6 12 12' }]
	],

	// The transmit bar (v1 §4, §8): the key control, a push-to-talk source that has died, and
	// the warning that nobody is hearing this.
	mic: [
		['path', { d: 'M12 19v3' }],
		['path', { d: 'M19 10v2a7 7 0 0 1-14 0v-2' }],
		['rect', { x: '9', y: '2', width: '6', height: '13', rx: '3' }]
	],
	'mic-off': [
		['path', { d: 'M12 19v3' }],
		['path', { d: 'M15 9.34V5a3 3 0 0 0-5.68-1.33' }],
		['path', { d: 'M16.95 16.95A7 7 0 0 1 5 12v-2' }],
		['path', { d: 'M18.89 13.23A7 7 0 0 0 19 12v-2' }],
		['path', { d: 'm2 2 20 20' }],
		['path', { d: 'M9 9v3a3 3 0 0 0 5.12 2.12' }]
	],
	'triangle-alert': [
		['path', { d: 'm21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3' }],
		['path', { d: 'M12 9v4' }],
		['path', { d: 'M12 17h.01' }]
	],

	// The hail picker (ADR-0048): a hail, the person holding a seat, and the two levels of the
	// one list expanding.
	bell: [
		['path', { d: 'M10.268 21a2 2 0 0 0 3.464 0' }],
		[
			'path',
			{
				d: 'M3.262 15.326A1 1 0 0 0 4 17h16a1 1 0 0 0 .74-1.673C19.41 13.956 18 12.499 18 8A6 6 0 0 0 6 8c0 4.499-1.411 5.956-2.738 7.326'
			}
		]
	],
	user: [
		['path', { d: 'M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2' }],
		['circle', { cx: '12', cy: '7', r: '4' }]
	],
	'chevron-right': [['path', { d: 'm9 18 6-6-6-6' }]],
	'chevron-down': [['path', { d: 'm6 9 6 6 6-6' }]],

	// Announcements (v1 §11).
	megaphone: [
		[
			'path',
			{
				d: 'M11 6a13 13 0 0 0 8.4-2.8A1 1 0 0 1 21 4v12a1 1 0 0 1-1.6.8A13 13 0 0 0 11 14H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2z'
			}
		],
		['path', { d: 'M6 14a12 12 0 0 0 2.4 7.2 2 2 0 0 0 3.2-2.4A8 8 0 0 1 10 14' }],
		['path', { d: 'M8 6v8' }]
	],

	// The admin console: back out of a row or a column, move a loop in the base order,
	// rename, remove, add, commit.
	'arrow-left': [
		['path', { d: 'm12 19-7-7 7-7' }],
		['path', { d: 'M19 12H5' }]
	],
	// Arrows rather than chevrons for reordering: the loop list moves a row one place, and a
	// chevron is the console's mark for a list opening (ADR-0048's picker), not for travel.
	'arrow-up': [
		['path', { d: 'm5 12 7-7 7 7' }],
		['path', { d: 'M12 19V5' }]
	],
	'arrow-down': [
		['path', { d: 'M12 5v14' }],
		['path', { d: 'm19 12-7 7-7-7' }]
	],
	pencil: [
		[
			'path',
			{
				d: 'M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z'
			}
		],
		['path', { d: 'm15 5 4 4' }]
	],
	'trash-2': [
		['path', { d: 'M10 11v6' }],
		['path', { d: 'M14 11v6' }],
		['path', { d: 'M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6' }],
		['path', { d: 'M3 6h18' }],
		['path', { d: 'M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2' }]
	],
	plus: [
		['path', { d: 'M5 12h14' }],
		['path', { d: 'M12 5v14' }]
	],
	check: [['path', { d: 'M20 6 9 17l-5-5' }]]
};

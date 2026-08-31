// What the frame knows, handed down to the surfaces inside it.
//
// The signalling channel is **one socket per tab, opened at sign-in** (ADR-0054), and the
// frame is what holds it: an administrator reading a configuration page has not left the
// lobby and their socket has not closed. So the lobby document and who the server says is
// signed in are read from here rather than asked for again a page at a time. A second read
// is a second answer, and two answers to one question is how a console comes to show a
// system-administration flag that has already gone.

import { getContext, setContext } from 'svelte';

const FRAME = Symbol('the frame');

/** Held once, by the frame itself. */
export const holdFrame = (frame) => setContext(FRAME, frame);

/** What the frame knows, live: it is the frame's own state rather than a copy of it. */
export const theFrame = () => getContext(FRAME);

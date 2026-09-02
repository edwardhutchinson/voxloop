# Styling the console

The rules for writing a console component. The reasoning is in
[ADR-0069](../adr/0069-styling-is-scoped-css-over-one-token-file.md) and you do not need it to
follow these.

There is **no CSS framework and no component library**. Svelte's scoped `<style>` blocks are
the mechanism, `web/src/app.css` holds the scale and the palette, and the console is **dark
only**.

## The rules

**No literal values for `margin`, `padding`, `gap`, `font-size` or `border-radius`.** Use the
tokens. `0` needs no unit and so needs no token. Position offsets — `inset`, `top`, `left` —
are spacing under another name and go by the same rule, and so does a size hidden in the
`font` shorthand.

**A layout rule that needs justifying carries a comment saying why.** `Matrix.svelte` is the
example: *past roughly 26 to 30 loops this scrolls sideways*. If the next author would undo
the rule without knowing something, write the something down.

**Shared furniture is a bare element selector in `app.css`.** `button`, `a`, `input`,
`select`, `table`, `th`, `td`, `h1`, `h2`, plus the utility classes `.quiet`, `.refusal`,
`.destructive`, `.acts`, `.new`, `.name`, `.lesser`, `.wayin`, `.field`, `.back`, `.meaning`
and `.awaiting`. Styling the bare element means an unstyled `<button>` is already correct.
**A component never declares `:global()`** — if a rule has to escape the component, it is
furniture and it belongs in `app.css`. **And never an inline `style` attribute**: it is out
of reach of every rule here, which is the whole of the case against it.

The second time a block is written out identically in two components, it is furniture and it
moves. `.awaiting` — the panel in the corner holding something out to the administrator — got
here that way, from `Confirm.svelte` and the enrolment code in `Users.svelte`, and #71 moved
`h1`, `.wayin`, `.field`, `.back`, `.meaning` and `.lesser` the same way. `SignIn.svelte`,
`Enrol.svelte`, `WhichRoles.svelte`, `WhoMayAssume.svelte` and `RolePage.svelte` were left
with no `<style>` block at all, which is what the rule looks like when it is working.

**A link is drawn as a control, because every one of them is one.** `a` sits with `button` in
`app.css` and takes the same chrome: the console's links are its nav, a row's acts and the way
back out of a page, and each of them stands among buttons. There is no prose here for a link to
sit inside, and the day there is, that is the day to split the rule. Write the navigation as a
link rather than a button — the page it opens has a URL, so it middle-clicks and it copies —
and put the path through SvelteKit's `resolve()`, which the lint config already insists on.

**A component exists where there is behaviour, not where there is appearance.** `Confirm.svelte`
is a component because it holds a decision. A wrapper that only sets classes is not.

**Class names say what a thing is in the domain, never how it looks.** `.refusal`, never
`.red`. `.unreviewed`, never `.dimmed`.

**Colour is never the only thing carrying a state.** The spec demands this twice already —
blind-armed loops are named in words, not just coloured (v1 §4), and say so in words, not just
a border (§8). It holds for every state added from here.

**The console renders no motion.** Cognitive load is the thing being minimised, and motion is
permitted in exactly one place — the talking indicator, one fixed rate and one fixed shape,
reading unambiguously as on or off (v1 §8,
[ADR-0033](../adr/0033-the-console-shows-that-someone-is-talking-never-who.md)). Until #41
builds it there is no such place, so `animation`, `transition`, `@keyframes` and Svelte's
`transition:`, `in:`, `out:` and `animate:` directives are refused outright by
`tests/styling.test.js`. The indicator gets written **into** that check rather than around it.

**No raw colour anywhere outside `app.css`**, and **a new colour token must cite the spec line
demanding it**. The palette is seven colours and stays small on purpose.

## The tokens

**The values are in `web/src/app.css` and nowhere else** — read them there. They are asserted
a second time in `web/tests/styling.test.js`, which is what makes an accidental change to one
fail the build, and a third copy in this file would be prose that quietly goes out of date.

| | |
|---|---|
| **Spacing** | `--space-1` to `--space-6`, smallest to largest |
| | `--space-page-bottom` — clears the transmit bar, which is never scrolled away |
| **Type** | `--type-1` to `--type-5`, smallest to largest |
| **Radius** | `--radius`, the only one |
| **Colour** | `--ground` the page · `--raised` a surface on it · `--ink` text · `--quiet` secondary text · `--rule` a border · `--refusal` a denial or a destructive act · `--warning` *this is true and you should look at it* — v1 §8's `0 hearing`, a frozen lobby, a media path that is impaired or lost |

Nothing else has a value to reach for: widths, `letter-spacing` and border widths are decided
per rule and carry a comment when they are not obvious.

## Adding an icon

`Icon.svelte` takes a `name` and reads the shapes from `icons.js` beside it. An icon is
`1em` wide and stroked in `currentColor`, so it inherits size and colour from whatever it sits
next to and never needs a token.

```svelte
<a href={resolve('/admin/loops')}><Icon name="arrow-left" /> All loops</a>
```

It is `aria-hidden` by default, because most icons sit beside text that already says the
thing. Pass `label` — `<Icon name="bell" label="Hail" />` — for one that stands alone, and it
becomes `role="img"` with that label.

**Write the name out.** `tests/icon.test.js` checks every `name="…"` against `icons.js`, so a
typo fails the build rather than rendering a hole; a computed `name={…}` is past what it can
see.

To add one:

1. Copy it from [lucide.dev](https://lucide.dev). Keep the 24×24 coordinates exactly; the
   viewBox and the stroke attributes live on `Icon.svelte`'s wrapper and are not copied.
2. Add the entry to `icons.js` as `['tag', { …attributes }]` per shape, in the group for the
   surface that uses it. `path`, `circle` and `rect` are what `Icon.svelte` draws — an icon
   built out of anything else fails `tests/icon.test.js` rather than rendering a hole.
3. Check Lucide's `LICENSE` for the icon. If it is in the Feather-derived list, add it to the
   names above the MIT notice at the top of `icons.js` — **by the name that list uses**, which
   for a renamed icon is not the name you copied. `triangle-alert` is in it as
   `alert-triangle`, and a text search for the new name finds nothing.

**`icons.js` is the audit of what the product uses**, and that is what it is for rather than a
package nobody reads. The set was seeded from the surfaces v1 names, grouped in the file by
the surface that wants each one, so what is in there can be checked against the spec. **An
icon whose surface ships without it is deleted**, and so is one whose last caller goes.

## Checking it

`npm run format`, `npm run lint` and `npm test` in `web/`. The rules above that a machine can
read are in `tests/styling.test.js`; the rest are read by whoever reviews the change.

# Styling is scoped CSS over one token file

The console's styling is **Svelte's scoped `<style>` blocks over one token file**, `web/src/app.css`,
with **no CSS framework and no component library**. It is **dark only**. The rules a component
author follows are in [`docs/agents/styling.md`](../agents/styling.md); this records why they
are those rules.

## The question, and why it was asked

The console is fifteen components and every one of them was written without a standard.
[#38](https://github.com/edwardhutchinson/voxloop/issues/38) onward writes the operator
console — the board, the ledger, the transmit bar, the hail picker — and none of it is
started. The standard lands while the answer costs a tidying rather than a migration.

Svelte already scopes CSS per component. The usual case for a utility framework is escaping
the cascade, and there is no cascade here to escape. What a framework would have bought is a
**fixed scale**, and a token file buys that directly, for the price of one file.

What it would have cost is specific. The console's CSS carries comments explaining why a rule
exists, and `Matrix.svelte` is the example: *past roughly 26 to 30 loops this scrolls
sideways, and a column read stops being a glance*. That is the kind of fact that stops
somebody undoing the rule six months later, and a class attribute has nowhere to put it. With
**cognitive load the thing being minimised** (v1 standing requirements), an explanation the
next author can read is not decoration.

## Alternatives

**Tailwind.** Buys the scale and charges for it in the one currency this codebase spends
freely. Every layout decision moves into a class attribute where it cannot be explained, and
the build gains a dependency for a product that must stay installable air-gapped with no CDN.
The scale is the part worth having and `app.css` is all of it.

**shadcn-svelte, or bits-ui underneath it.** v1's whole interactive surface is one modal — the
per-loop volume cog — one bespoke two-level picker, tables and a banner. Native `<dialog>`
with `showModal()` gives focus trapping, escape and focus return, which is the modal's entire
reason for wanting a library. The picker is bespoke either way: it lists roles permitted to
hear a loop, each expanding to whoever occupies it ([ADR-0048](./0048-the-hail-picker-is-the-only-place-the-console-names-a-person.md)),
and nothing ships that. Revisit if a specific primitive proves hard, not before.

**A light theme.** `color-scheme: dark` was already hardcoded, v1 §10 lists every
personalisation item and no theme is among them, and control rooms are dark. The tokens are
named semantically anyway — `--ground`, `--ink`, `--quiet` rather than shades — so a light
theme stays a redefinition of seven values later, at no cost today. Supporting it now would
mean every state added from here gets checked twice against a reading nobody has asked for.

**A colour per state.** The pressure to add `--priority`, `--staffing` and `--armed` will
arrive with the operator console, and the spec has already refused it twice: blind-armed loops
are **named in words, not just coloured** (§4), and say so **in words, not just a border**
(§8). A palette that grows a colour per state ends up distinguishing five states by hue in a
dark room, which is the failure both of those lines were written against. A new colour token
must cite the spec line demanding it — which is how `--warning` got in, on §8's `0 hearing`.

**A `Button.svelte`.** Rejected on the same reasoning as the framework. In a system with
scoped styles, styling the bare `<button>` element means an unstyled button is already
correct and nobody can forget an import. A component that only sets classes is a layer with
no behaviour in it. Components exist where there is behaviour, which is what `Confirm.svelte`
already is.

**Lucide as a dependency.** The console needs about twenty icons, and one file holding their
path data is also the place to audit what the product uses. The set is **seeded from the
surfaces v1 names** — the board, the transmit bar, the hail picker, announcements, the admin
console — and grouped in the file by the one that wants each icon, so the file can be read
against the spec. That is the whole of the audit argument, and it is unavailable from
`node_modules`: an icon nobody calls is visible and deletable here and invisible there. The
cost is honest — most of the set has no caller until [#38](https://github.com/edwardhutchinson/voxloop/issues/38)
onward builds the surfaces, and an icon whose surface ships without it goes.

Both licences — Lucide's ISC and Feather's MIT for the derived icons — require the notice
travels with the copy, so both are at the top of `icons.js`. This is a self-hosted product
that ships to customers, and that is not optional. Lucide's Feather list is by the icon's
**old** name where one was renamed, which is a trap worth one line: `triangle-alert` is in it
as `alert-triangle`.

## Consequences

- **`app.css` is the only file that names a colour, and the only file with global rules.** A
  component that needs one is a signal the furniture is wrong, not a reason for `:global()`.
- **The scale is enforced, not encouraged.** `web/tests/styling.test.js` fails the build on a
  literal margin, padding, gap, font size, radius or offset, on a colour outside `app.css` —
  every CSS named colour, not a shortlist — on a token no `:root` defines, on a `:global()`
  and on an inline `style` attribute. [#69](https://github.com/edwardhutchinson/voxloop/issues/69)
  put it that a standard nobody can check is prose; this is the check.
- **The second identical block is furniture.** `Confirm.svelte` and the enrolment code in
  `Users.svelte` had grown the same seven-declaration panel, and tokenising them made that
  visible by making them identical. It is `.awaiting` in `app.css` now, and `Confirm.svelte`
  has no `<style>` block at all — which is the ADR's own claim about what a component is for,
  arriving one commit after it was written down.
- **Snapping to the scale changed a few values.** Twelve spacing values became six, three type
  sizes moved by a step, and the sign-in form's negative-margin coupling to the enrolment
  link went with them. This is the whole cost of the decision, and it was paid at fifteen
  components rather than thirty-five.
- **An icon inherits its size and its colour** — `1em` and `currentColor` — so it never needs
  a token, a variant, or a decision at the call site.
- **The palette stays small on purpose, and growing it is an argument.** Seven colours, and
  the eighth has to point at a line in the spec.

# A role with no limit is the limit left unset

`max_occupants` is optional. A role carrying no limit admits any number of occupants, and
that is **the same concept with the limit left unset** rather than a second kind of role — v1
§1 says single-occupant and multi-occupant roles differ only in their limits, and this is the
end of that same line. A limit of zero is refused: a role is a **staffable position**, so one
nobody may occupy is not a role at all.

## The question, and why it was asked

Install seeds `Observer`, and **every user is eligible for it** ([ADR-0025](./0025-credentials-are-administered-because-there-is-no-email.md)).
Seeding it with a number means VoxLoop guessing how many people work here. The guess is
discovered by exactly one person — the one it turns away — at the moment they are trying to
start a shift, and it reads to them as a permission fault rather than as a limit somebody
picked in 2026.

## Alternatives

**Seed a large number.** `500` turns the guess into a bigger guess and keeps every one of its
properties: still arbitrary, still silent until it bites, still discovered by whoever is late
to a console. It also has to be re-guessed by every site that copies the seed.

**A flag beside the number.** Two fields that can disagree, and a third state — *unlimited,
limit 6* — that nothing can act on. The absence of a limit is already a value the store can
hold, so buying a second column to say it is buying a contradiction.

**A kind of role that has no limit.** Rejected for the reason [ADR-0055](./0055-there-is-no-conference-loop.md)
declined a loop kind: a second concept bought to express something the first already
expresses, and every later feature then has to answer *which kind*.

## Consequences

- **The console renders absence as `no limit`, and an empty box is how one is set.** It is not
  a field somebody forgot; it is what an administrator says when the answer is *anybody who is
  eligible*.
- **An audit snapshot renders it in words** — `max_occupants=no limit` — so an entry can be
  read without knowing what an empty column would have meant.
- **An edit tells *leave it alone* apart from *take it away*.** Absent leaves the limit
  standing and `null` removes it, so renaming a role cannot silently unbound it.
- **A limit too large to read back is answered as a very large limit, never as no limit.** No
  limit is an administered decision, and a fault must not be answered with one.
- **Occupancy enforcement has one question to ask** (#37): is there a limit, and is it reached.
  There is no kind to switch on first.

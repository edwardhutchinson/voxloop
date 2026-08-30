-- Roles and loops: the two configuration objects voice authority is expressed over.
--
-- Both are named by an immutable opaque internal id and carry a mutable name, for the reason
-- `users` does (ADR-0024): everything else references the id, so a rename breaks nothing. A
-- stray join on a name works perfectly until the first rename, which is why there is not one
-- anywhere in this schema.

-- A role is a staffable position with a `max_occupants` limit, not a group of users (v1 §1).
-- Single-occupant and multi-occupant roles are the same concept under different limits.
CREATE TABLE roles (
    -- 128 random bits, never reused, exactly as a user id is.
    id            TEXT    PRIMARY KEY,
    -- NOCASE so that `Flight` and `flight` cannot be two positions on one console.
    name          TEXT    NOT NULL COLLATE NOCASE UNIQUE,
    -- Null is *no limit* (ADR-0068), the same concept with the limit left unset rather than
    -- a second kind of role. `Observer` is seeded that way: every user is eligible for it, so
    -- any number this column held instead would be VoxLoop guessing how many people work
    -- here, and the guess is only ever discovered by the person it turns away.
    max_occupants INTEGER,
    created_at    INTEGER NOT NULL,
    CHECK (max_occupants IS NULL OR max_occupants >= 1)
) STRICT;

-- A loop is an audio conference and the only thing voice can be addressed to (ADR-0001).
--
-- There is no kind, type or category column, and there will not be one: a private room is an
-- ordinary loop an administrator configured, and VoxLoop neither knows nor cares (ADR-0055).
CREATE TABLE loops (
    id            TEXT    PRIMARY KEY,
    name          TEXT    NOT NULL COLLATE NOCASE UNIQUE,
    -- A loop created after install is unreviewed until an administrator has set or
    -- explicitly dismissed each role's cell (ADR-0015). Absent-because-denied and
    -- absent-because-nobody-ruled render identically otherwise. It is a display state and a
    -- prompt: the evaluator enforces an unreviewed loop's cells as `none` like any other.
    is_unreviewed INTEGER NOT NULL DEFAULT 1,
    -- The deployment-wide base loop order, which is administered rather than derived
    -- (ADR-0053): not alphabetical, and not creation order. It is deliberately not unique —
    -- an order is set by rewriting every position, and a unique index would refuse the
    -- half-written states on the way through.
    position      INTEGER NOT NULL,
    created_at    INTEGER NOT NULL,
    CHECK (is_unreviewed IN (0, 1))
) STRICT;

CREATE INDEX loops_in_the_base_order ON loops (position);

-- Install seeds the `Observer` role (v1 §10, ADR-0015).
--
-- Seeding it here rather than at every start is what makes it *install*: it happens once,
-- in the same transaction as the tables, and a deployment that deletes or renames it does
-- not find it back the next morning.
--
-- Its reach is seeded only against loops present at install, and a deployment has none at
-- the moment this runs — so it is seeded with nothing, which is the whole of the rule. A
-- loop created later gets no Observer cell, deliberately: auto-seeding one would silently
-- grant reach on every loop created, including the ones created precisely because something
-- should not be broadly audible.
INSERT INTO roles (id, name, max_occupants, created_at)
VALUES (lower(hex(randomblob(16))), 'Observer', NULL, CAST(strftime('%s', 'now') AS INTEGER) * 1000);

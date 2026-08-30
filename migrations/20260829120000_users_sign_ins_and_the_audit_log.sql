-- The first schema VoxLoop persists: the user record, the sign-ins held against it, and the
-- audit log that outlives both.
--
-- Times are milliseconds since the Unix epoch. An integer sorts and compares without a date
-- library on either side of the wire, and rendering one for a human is the console's job.

-- A user record carries three things with strictly separate jobs (ADR-0024): an immutable
-- opaque internal id that is the only thing anything else references, a mutable username for
-- humans to type, and a nullable external identity that v1 stores and never writes.
--
-- There is no email column. Email is never an identity key and never a join key, and the
-- cheapest way to keep that true is to have nowhere to put one.
CREATE TABLE users (
    -- 128 random bits, never reused. Renaming a user changes nothing that references them.
    id                      TEXT    PRIMARY KEY,
    -- NOCASE so that `Alice` and `alice` cannot be two accounts on one console.
    username                TEXT    NOT NULL COLLATE NOCASE UNIQUE,
    -- Nullable: system administration creates the record and an enrolment code sets the
    -- password afterwards, so a user with no password yet is an ordinary state, not a fault.
    password_hash           TEXT,
    is_system_administrator INTEGER NOT NULL DEFAULT 0,
    -- The (issuer, subject) pair of ADR-0024. Both or neither.
    external_issuer         TEXT,
    external_subject        TEXT,
    created_at              INTEGER NOT NULL,
    CHECK ((external_issuer IS NULL) = (external_subject IS NULL)),
    CHECK (is_system_administrator IN (0, 1))
) STRICT;

CREATE UNIQUE INDEX users_by_external_identity
    ON users (external_issuer, external_subject)
    WHERE external_issuer IS NOT NULL;

-- A sign-in outlives a restart (v1 §2, "Lifetime"), which is what puts it in the store
-- rather than with the live state the state authority holds.
--
-- What is stored is a fingerprint of the token the browser holds, never the token itself: a
-- backup of this file is a real artefact sitting on somebody's disk, and it should not be a
-- drawer full of usable sign-ins.
CREATE TABLE sign_ins (
    fingerprint TEXT    PRIMARY KEY,
    -- Deleting a user signs them out. There is no state in which a deleted user is signed in.
    user_id     TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    started_at  INTEGER NOT NULL
) STRICT;

CREATE INDEX sign_ins_by_user ON sign_ins (user_id);

-- The audit log records decisions, not traffic (ADR-0028).
--
-- `actor_id` is deliberately not a foreign key. The log outlives the records it references,
-- so a deleted user's entries stay readable and attributed: the id keeps them correct and
-- the name snapshot keeps them legible.
CREATE TABLE audit_entries (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    recorded_at INTEGER NOT NULL,
    event       TEXT    NOT NULL,
    actor_id    TEXT,
    actor_name  TEXT    NOT NULL,
    -- Where the attempt came from. A failed sign-in with no source cannot show a brute-force
    -- attempt, which is the compensating control for rate-limiting rather than auto-locking.
    source      TEXT
) STRICT;

CREATE INDEX audit_entries_by_actor ON audit_entries (actor_id);

-- Append-only is an application discipline rather than a database guarantee (ADR-0038):
-- anything holding this file can drop these triggers. They are here because the discipline
-- has to be testable, and a test that can only assert the absence of a code path asserts
-- nothing.
CREATE TRIGGER audit_entries_are_never_amended
BEFORE UPDATE ON audit_entries
BEGIN
    SELECT RAISE(ABORT, 'the audit log is append-only');
END;

CREATE TRIGGER audit_entries_are_never_removed
BEFORE DELETE ON audit_entries
BEGIN
    SELECT RAISE(ABORT, 'the audit log is append-only');
END;

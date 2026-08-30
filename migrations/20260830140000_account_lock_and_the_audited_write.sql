-- What user administration adds: an account can be locked, and a configuration write is
-- recorded with what it changed, on whom, and what it did to anything live.

-- Account lock is a deliberate administrative act, never a consequence of failed attempts
-- (ADR-0025). It ends the user's sign-in and their session, and it is on the system
-- administration side of the role/user split — forced relinquish is the operational one.
ALTER TABLE users ADD COLUMN is_locked INTEGER NOT NULL DEFAULT 0;

-- Every configuration write is audited with before and after plus the blast radius (v1 §12).
--
-- `blast_radius` is the discriminator: an entry carrying one is a configuration write, and
-- an entry carrying none is an authentication event that changed no record. The radius is
-- computed on the live side and handed to this transaction as a value (ADR-0039), so an
-- empty one says *nothing live was touched* rather than *nobody worked it out*.
--
-- The target is stored the way the actor is — the internal id and a snapshot of the name as
-- it stood — because the log outlives the records it references (ADR-0028). Neither is a
-- foreign key: deleting a user must leave their entries readable and attributed, on both
-- sides of the entry.
ALTER TABLE audit_entries ADD COLUMN target_id TEXT;
ALTER TABLE audit_entries ADD COLUMN target_name TEXT;
ALTER TABLE audit_entries ADD COLUMN state_before TEXT;
ALTER TABLE audit_entries ADD COLUMN state_after TEXT;
ALTER TABLE audit_entries ADD COLUMN blast_radius TEXT;
-- Present where the write was refused and therefore did not happen. Refused administration
-- writes are audited; refused reads are not (v1 §3).
ALTER TABLE audit_entries ADD COLUMN refusal TEXT;

-- The log is filterable by actor and by target (v1 §12).
CREATE INDEX audit_entries_by_target ON audit_entries (target_id);

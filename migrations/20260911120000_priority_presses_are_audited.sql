-- Every priority press, with no minimum duration (v1 §12, ADR-0046).
--
-- A press is an **operational authority act** rather than a configuration write: keying
-- priority changes no record, so there is no before, no after and no blast radius — and the
-- radius staying NULL keeps it the discriminator it already is. The role it was keyed under is
-- written into the target the way a session entry names its seat, because the log is
-- filterable by target and outlives the records it references (ADR-0028).
--
-- Three columns carry what a session entry has no use for:
--
-- - `armed_on` — the armed loop set **at the moment of the press**, by name as the grid had
--   them, one per line. A priority transmission applies to the whole arm set (ADR-0045), so the
--   set is what the override was keyed over; names rather than ids, for the reason the loop
--   order is snapshotted by name: an entry is read by somebody asking *what did that press
--   reach*, and a line of opaque ids answers nothing.
-- - `pressed_at` — when the key went down, in milliseconds since the Unix epoch. `recorded_at`
--   is when it came back up and the entry could be written, which is a different moment.
-- - `lasted_ms` — how long it was held. **There is no floor**: a 200 ms fumble is still a
--   decision that overrode everybody's volume, and abuse may look like a hundred short jabs, so
--   filtering belongs to whoever reads the log.
ALTER TABLE audit_entries ADD COLUMN armed_on TEXT;
ALTER TABLE audit_entries ADD COLUMN pressed_at INTEGER;
ALTER TABLE audit_entries ADD COLUMN lasted_ms INTEGER;

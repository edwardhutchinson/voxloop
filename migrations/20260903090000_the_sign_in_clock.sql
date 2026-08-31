-- A sign-in ends after 24 hours with no deliberate act (v1 §2), so a sign-in has to record
-- when it last saw one.
--
-- The clock **runs only in the lobby**: assuming a role stops it, because an occupied role
-- is by definition not abandoned and an operator holding one through a thirty-hour incident
-- must not be signed out for failing to click anything (ADR-0023). Which sign-ins hold a
-- session is a live fact the state authority answers, so it is not a column here — this
-- column is only when the sign-in last did something, and the rule is applied over it.
--
-- There is deliberately no idle timeout on a session and no absolute cap on a sign-in, so
-- there is nothing else here to store.
ALTER TABLE sign_ins ADD COLUMN last_active_at INTEGER NOT NULL DEFAULT 0;

-- Every sign-in that already exists has done exactly one deliberate thing, which is start.
-- The default above is what SQLite needs to add the column at all; it is never the value a
-- row keeps, because opening a sign-in writes this and so does every act on it.
UPDATE sign_ins SET last_active_at = started_at;

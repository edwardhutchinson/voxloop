-- The enrolment code: what an email link would have been, if VoxLoop had a mail path.
--
-- Single-use, expiring, issued by an administrator against a user, and handed over out of
-- band (ADR-0025). Redeeming one sets that user's password. A reset is the same act again.
--
-- What is stored is a fingerprint of the code, never the code itself, for the reason
-- `sign_ins` stores one: the store is a file a deployment is obliged to back up, and a
-- backup should not be a drawer full of usable credentials.
CREATE TABLE enrolment_codes (
    fingerprint TEXT    PRIMARY KEY,
    -- Deleting a user takes their outstanding code with them. A code that enrols nobody is
    -- a credential nobody can spend, and there is no state in which one is left lying about.
    user_id     TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    issued_at   INTEGER NOT NULL,
    -- Milliseconds since the Unix epoch, like every other time in this store. A code is
    -- spent by being deleted, so an unexpired row is an outstanding code and there is no
    -- second column to disagree with this one.
    expires_at  INTEGER NOT NULL
) STRICT;

-- One outstanding code per user, enforced here rather than remembered: issuing a second one
-- invalidates the first, exactly as every start invalidates the bootstrap code the start
-- before it minted. Two live codes against one account would be two credentials to chase
-- when one of them turns out to be in a chat log.
CREATE UNIQUE INDEX enrolment_codes_by_user ON enrolment_codes (user_id);

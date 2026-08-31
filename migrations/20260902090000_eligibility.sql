-- Eligibility: who may assume which role.
--
-- It is the unconditional grant permitting a user to take up a role, and it carries no
-- permissions of its own — everything operational is one cell on the grid (ADR-0011). A row
-- here says *this person may sit in that seat* and nothing else; what the seat can hear or
-- say is `grid_cells`, and there is no path by which a row here widens one.
--
-- It is deliberately **not a second grid**. There is no permission column, no rung and no
-- value: the pair is present or it is absent, which is the whole of the model. The table
-- looks like the grid's and the resemblance stops at the shape — nothing reads this by role
-- and user together except to answer one yes or no, and nothing reads it whole at all
-- (ADR-0015: rendered as a matrix, 190 users by 15 roles was the least legible object the
-- prototype produced).
CREATE TABLE eligibility (
    -- Deleting either record takes the grant with it. An eligibility whose user or whose
    -- role is gone is not a grant anybody could exercise, and leaving one behind would mean
    -- a later record minted with the same id inheriting it — which never happens, because
    -- ids are never reused, but is exactly the kind of thing a cascade stops being a worry.
    user_id    TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role_id    TEXT    NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    granted_at INTEGER NOT NULL,
    -- The pair is the identity of a grant. Granting twice is the same grant, not two of
    -- them, and there is no second row to disagree with the first.
    PRIMARY KEY (user_id, role_id)
) STRICT;

-- The two directions eligibility is administered from (ADR-0015): *which roles may this
-- person assume* is the primary key, and *who may assume this role* is this index. There is
-- no third read, because there is no view of the whole.
CREATE INDEX eligibility_by_role ON eligibility (role_id);

-- Every user record starts with seeded `Observer` eligibility (v1 §2), and the users a
-- deployment already has were created before there was anywhere to record that. Seeding
-- them here is what makes the rule true of the deployment rather than only of the users
-- created after this migration ran.
--
-- The role is found by name, because the seeded `Observer` carries an id minted at install
-- and nothing else marks it. A deployment that renamed or deleted it has decided what its
-- listen-only position is, and VoxLoop guessing which role replaced it would be worse than
-- seeding nothing — so nothing is what it seeds.
INSERT INTO eligibility (user_id, role_id, granted_at)
SELECT users.id, roles.id, CAST(strftime('%s', 'now') AS INTEGER) * 1000
FROM users, roles
WHERE roles.name = 'Observer';

-- Per-loop volume, remembered as personalisation per (user, role, loop).
--
-- It is the one personalisation item scoped to the triple (ADR-0051): a volume is about one
-- loop, as heard from one seat, by one person's ears and headset. The subscription set beside
-- it is about the seat, which is why the two are different tables rather than a column on
-- one.
--
-- **A loop with no row is at unity**, which is where every loop starts (v1 §10) and which no
-- role default may move (ADR-0052). So this table is the loops somebody has set, and nothing
-- is written for a loop nobody has touched.
--
-- The write rides the live act and is **best effort**, exactly as the subscription set's is
-- (ADR-0050): a row missing here is a preference lost and never a console that is wrong.
--
-- **Mute is not here and never will be.** A stale mute drops every loop its owner staffs to
-- `away` the moment they assume, before they have looked at anything, so it is live state
-- and dies with the session (ADR-0050).
CREATE TABLE remembered_volumes (
    -- All three cascade, for the reason the subscription set's do: there is nothing to
    -- preserve once the thing being personalised is gone (ADR-0050).
    user_id  TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role_id  TEXT    NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    loop_id  TEXT    NOT NULL REFERENCES loops (id) ON DELETE CASCADE,
    -- A percentage of full gain, from silence to unity and no further. **Volume is an
    -- attenuation control** (v1 §5), so there is nothing above unity to store.
    volume   INTEGER NOT NULL CHECK (volume BETWEEN 0 AND 100),
    set_at   INTEGER NOT NULL,
    PRIMARY KEY (user_id, role_id, loop_id)
) STRICT;

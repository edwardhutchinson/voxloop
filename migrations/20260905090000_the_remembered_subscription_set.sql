-- The subscription set, remembered as personalisation per (user, role).
--
-- **A subscription itself is live state** and lives with the state authority, dying with the
-- session like everything else there (ADR-0039). What is here is the memory of it: the set a
-- role last had, so that assuming rebuilds a console rather than leaving somebody to
-- reassemble one by hand during whatever incident caused the restart (ADR-0050). The two are
-- deliberately different things with different lifetimes, and the table is named for the one
-- it holds.
--
-- It is scoped to (user, role) because that is the smallest thing a subscription set is
-- about (ADR-0051): a person's loops are a property of the seat they are in, not of them.
--
-- The write rides the live act and is **best effort** — it can never fail one — so a row
-- missing here is a preference lost and never a console that is wrong.
CREATE TABLE remembered_subscriptions (
    -- All three cascade: deleting a user, a role or a loop takes the personalisation about
    -- it with it, because there is nothing to preserve once the thing being personalised is
    -- gone (ADR-0050).
    user_id       TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role_id       TEXT    NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    loop_id       TEXT    NOT NULL REFERENCES loops (id) ON DELETE CASCADE,
    subscribed_at INTEGER NOT NULL,
    -- The triple is the identity. A row present is *this pair had this loop up*, and there
    -- is no value beside it: a subscription is held or it is not.
    PRIMARY KEY (user_id, role_id, loop_id)
) STRICT;

-- There is no index beside the key. The one read is *the set this pair last had*, and the
-- primary key's own index answers it on its leading two columns.
--
-- **A pair with no rows is a pair with no subscriptions**, and this schema cannot tell that
-- from a pair that has never assumed the role. Today the two are the same answer, because
-- there is no role default to seed a first assume from; the column that tells them apart
-- belongs to #27, which builds the default that needs it, rather than sitting here now with
-- nothing reading it.

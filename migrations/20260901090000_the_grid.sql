-- The grid: one value per (role, loop), and the only place voice authority is configured.
--
-- There is one row per cell and no second table beside it. No per-user grant, no per-user
-- deny, no explicit deny that beats a grant and no precedence rule — evaluating a permission
-- is one lookup (ADR-0011), and every one of those would be a second lookup that could
-- disagree with the first.
CREATE TABLE grid_cells (
    role_id    TEXT    NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    loop_id    TEXT    NOT NULL REFERENCES loops (id) ON DELETE CASCADE,
    -- One of an ordered four, each rung carrying everything below it: `none`, `monitor`,
    -- `emit`, `control`. Stored as the word rather than as a number, because a log or a
    -- store somebody reads by hand should say `emit` rather than `2` — the ordering lives
    -- in the binary, where the ladder is one enum.
    --
    -- A row holding `none` is a **deliberate** `none`, written when an administrator ruled
    -- on the cell or dismissed the loop's unreviewed mark. It is indistinguishable from an
    -- absent row to the evaluator, and that is the point: the difference is a prompt for an
    -- administrator, never an input to a permission decision.
    permission TEXT    NOT NULL,
    set_at     INTEGER NOT NULL,
    -- The pair is the identity of a cell. A role holds exactly one permission on a loop, so
    -- a second row for the same pair is not a second opinion, it is a bug that would make
    -- the lookup depend on which row was read first.
    PRIMARY KEY (role_id, loop_id),
    CHECK (permission IN ('none', 'monitor', 'emit', 'control'))
) STRICT;

-- A loop's column — *who may hear this loop* — is read as often as a role's row, and the
-- primary key only serves the row.
CREATE INDEX grid_cells_by_loop ON grid_cells (loop_id);

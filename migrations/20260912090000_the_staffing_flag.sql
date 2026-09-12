-- The staffing flag: whether a (role, loop) pair counts toward that loop's staffing state.
--
-- It is a column on the cell rather than a table of its own because it is set per (role,
-- loop) and a cell is already that pair (ADR-0011). A second table keyed on the same two ids
-- would be a second row to keep in step with the first, and the one rule the flag has —
-- **only where the role may emit** — is a rule about the value in the column beside it.
--
-- **The flag reports; it never subscribes** (ADR-0065). Nothing about it seeds a console,
-- which is why nothing is written anywhere else by the migration that adds it: every
-- existing cell starts unmarked, and a deployment upgrading into this schema has no loop
-- with a staffing state until an administrator says which roles staff which loops.
ALTER TABLE grid_cells
    ADD COLUMN staffs INTEGER NOT NULL DEFAULT 0
    -- A role that cannot answer cannot staff (v1 §1), enforced here as well as in the
    -- binary: the flag and the permission are one row, so the store can hold the rule rather
    -- than trust every writer to remember it. Lowering a cell below `emit` therefore has to
    -- clear the flag in the same statement, which is what `set_cell` does.
    CHECK (staffs IN (0, 1) AND (staffs = 0 OR permission IN ('emit', 'control')));

-- A loop's staffing roles are read per loop — *what staffs this* — every time a presence
-- document or a lobby is worked out, which is several times a second across a deployment.
-- The column index serves the whole of that read.
CREATE INDEX grid_cells_that_staff ON grid_cells (loop_id) WHERE staffs <> 0;

-- Warehouses (owner notes: the warehouses page): where it is, how to reach
-- it, a note, and who keeps it.
ALTER TABLE warehouses ADD COLUMN address TEXT NOT NULL DEFAULT '';
ALTER TABLE warehouses ADD COLUMN phone TEXT NOT NULL DEFAULT '';
ALTER TABLE warehouses ADD COLUMN keeper TEXT NOT NULL DEFAULT '';
ALTER TABLE warehouses ADD COLUMN note TEXT NOT NULL DEFAULT '';

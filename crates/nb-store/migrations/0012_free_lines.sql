-- A sale line may be an item that is not in the catalog (typed at the
-- till: its name, price and VAT rate): its product is then empty. SQLite
-- cannot relax NOT NULL in place, so the table is made again, as it was
-- otherwise (append-only).

CREATE TABLE sale_lines_new (
    sale_id               TEXT NOT NULL REFERENCES sales(id),
    position              INTEGER NOT NULL,
    product_id            TEXT REFERENCES products(id),
    name                  TEXT NOT NULL,
    unit                  TEXT NOT NULL,
    qty_milli             INTEGER NOT NULL,
    base_qty_milli        INTEGER NOT NULL,
    unit_price_minor      INTEGER NOT NULL,
    line_discount_minor   INTEGER NOT NULL,
    sale_discount_minor   INTEGER NOT NULL,
    tax_bps               INTEGER NOT NULL,
    net_minor             INTEGER NOT NULL,
    tax_minor             INTEGER NOT NULL,
    gross_minor           INTEGER NOT NULL,
    unit_cost_minor       INTEGER NOT NULL,
    movement_id           TEXT,
    PRIMARY KEY (sale_id, position)
);
INSERT INTO sale_lines_new SELECT * FROM sale_lines;
DROP TRIGGER sale_lines_no_update;
DROP TRIGGER sale_lines_no_delete;
DROP TABLE sale_lines;
ALTER TABLE sale_lines_new RENAME TO sale_lines;
CREATE TRIGGER sale_lines_no_update BEFORE UPDATE ON sale_lines
BEGIN SELECT RAISE(ABORT, 'completed sales are immutable'); END;
CREATE TRIGGER sale_lines_no_delete BEFORE DELETE ON sale_lines
BEGIN SELECT RAISE(ABORT, 'completed sales are immutable'); END;

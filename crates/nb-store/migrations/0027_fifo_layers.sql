-- FIFO valuation (M6.10): the organization's method, and the cost layers
-- (what is left of each receipt, at its cost) — a projection of the
-- movements like the balances, kept with every movement and rebuilt from
-- them.
ALTER TABLE organizations ADD COLUMN valuation TEXT NOT NULL DEFAULT 'average'
    CHECK (valuation IN ('average', 'fifo'));

CREATE TABLE stock_layers (
    product_id      TEXT NOT NULL REFERENCES products(id),
    warehouse_id    TEXT NOT NULL REFERENCES warehouses(id),
    seq             INTEGER NOT NULL,
    qty_milli       INTEGER NOT NULL CHECK (qty_milli > 0),
    unit_cost_minor INTEGER NOT NULL,
    PRIMARY KEY (product_id, warehouse_id, seq)
);

-- The layers of the stock already there: one per product and warehouse,
-- at its average cost (its history before is not split by receipt).
INSERT INTO stock_layers (product_id, warehouse_id, seq, qty_milli, unit_cost_minor)
SELECT product_id, warehouse_id, last_seq, on_hand_milli, avg_cost_minor
FROM stock_balances WHERE on_hand_milli > 0;

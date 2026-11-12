-- 100 000 products (M14.5): the newest first without sorting them all, and
-- a product's stock read from the index alone.

CREATE INDEX products_recent ON products (org_id, created_at);
CREATE INDEX stock_balances_product_qty ON stock_balances (product_id, on_hand_milli);

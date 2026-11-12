-- Reports on years of data (M14.5): returns and payments found by time
-- and by sale, not by reading them all.

CREATE INDEX sale_returns_recent ON sale_returns (org_id, created_at);
CREATE INDEX payments_sale ON payments (sale_id);
CREATE INDEX sale_lines_product ON sale_lines (product_id);

-- Ready for several replicas (M11/M0, ADR-0006): counters per device, so
-- two devices offline never give the same number or code, and the audit
-- chain per device, so chains from several devices can live side by side.

CREATE TABLE counters (
    name           TEXT NOT NULL,
    scope          TEXT NOT NULL,
    device_prefix  TEXT NOT NULL,
    last           INTEGER NOT NULL,
    PRIMARY KEY (name, scope, device_prefix)
);

INSERT INTO counters (name, scope, device_prefix, last)
SELECT 'doc:' || kind, branch_id || ':' || year,
       COALESCE((SELECT prefix FROM device LIMIT 1), 'A'), last
FROM doc_counters;
INSERT INTO counters (name, scope, device_prefix, last)
SELECT 'customer', org_id, COALESCE((SELECT prefix FROM device LIMIT 1), 'A'), last
FROM customer_counters;
INSERT INTO counters (name, scope, device_prefix, last)
SELECT 'supplier', org_id, COALESCE((SELECT prefix FROM device LIMIT 1), 'A'), last
FROM supplier_counters;
INSERT INTO counters (name, scope, device_prefix, last)
SELECT 'plan', org_id, COALESCE((SELECT prefix FROM device LIMIT 1), 'A'), last
FROM plan_counters;

-- Automatic product codes (P-00012) were the highest one plus one; they
-- become a counter too.
INSERT INTO counters (name, scope, device_prefix, last)
SELECT 'product', org_id, COALESCE((SELECT prefix FROM device LIMIT 1), 'A'),
       MAX(CAST(substr(sku, 3) AS INTEGER))
FROM products WHERE sku GLOB 'P-[0-9]*' GROUP BY org_id;

DROP TABLE doc_counters;
DROP TABLE customer_counters;
DROP TABLE supplier_counters;
DROP TABLE plan_counters;

-- The device that wrote each audit record; the chain runs per device.
-- Records from before (the log is append-only, never updated) keep the
-- empty device: their own chain, complete as it was.
ALTER TABLE audit_log ADD COLUMN device_id TEXT NOT NULL DEFAULT '';
CREATE INDEX audit_log_device ON audit_log(device_id, seq);

-- The operation log: each operation keeps the call that made it (use case
-- and arguments) and the device that ran it, so another replica can run
-- it again (`ops::replay`). Older operations have none.
ALTER TABLE operations ADD COLUMN call TEXT;
ALTER TABLE operations ADD COLUMN device_id TEXT;

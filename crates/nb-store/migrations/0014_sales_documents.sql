-- Sales documents before the invoice (M8.13): quotes and orders, with
-- their states. A draft changes freely; an approved document only moves on
-- (to an order, to an invoice) or is cancelled; a document that has moved
-- on or was cancelled never changes again. The invoice is the sale itself,
-- which remembers the document it came from.

CREATE TABLE sales_documents (
    id              TEXT PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES organizations(id),
    branch_id       TEXT NOT NULL REFERENCES branches(id),
    warehouse_id    TEXT NOT NULL REFERENCES warehouses(id),
    kind            TEXT NOT NULL CHECK (kind IN ('quote', 'order')),
    number          TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('draft', 'approved', 'converted', 'invoiced', 'cancelled')),
    customer_id     TEXT,
    customer_name   TEXT NOT NULL DEFAULT '',
    valid_until     TEXT,
    discount_bps    INTEGER NOT NULL DEFAULT 0,
    discount_minor  INTEGER NOT NULL DEFAULT 0,
    net_minor       INTEGER NOT NULL,
    tax_minor       INTEGER NOT NULL,
    total_minor     INTEGER NOT NULL,
    note            TEXT NOT NULL DEFAULT '',
    -- The quote this order came from; the order this quote became.
    source_id       TEXT REFERENCES sales_documents(id),
    next_id         TEXT REFERENCES sales_documents(id),
    sale_id         TEXT REFERENCES sales(id),
    cancel_reason   TEXT NOT NULL DEFAULT '',
    version         INTEGER NOT NULL DEFAULT 1,
    created_at      INTEGER NOT NULL,
    created_by      TEXT,
    updated_at      INTEGER NOT NULL,
    approved_at     INTEGER,
    approved_by     TEXT,
    UNIQUE (org_id, number)
);
CREATE INDEX sales_documents_list ON sales_documents (org_id, kind, status, created_at);

-- What has moved on stays as it is.
CREATE TRIGGER sales_documents_final BEFORE UPDATE ON sales_documents
WHEN OLD.status IN ('converted', 'invoiced', 'cancelled')
BEGIN SELECT RAISE(ABORT, 'this document is final'); END;
-- An approved document keeps its contents: only its state moves.
CREATE TRIGGER sales_documents_approved BEFORE UPDATE ON sales_documents
WHEN OLD.status = 'approved' AND (
    NEW.total_minor IS NOT OLD.total_minor OR NEW.customer_id IS NOT OLD.customer_id
    OR NEW.customer_name IS NOT OLD.customer_name OR NEW.version IS NOT OLD.version
    OR NEW.discount_bps IS NOT OLD.discount_bps OR NEW.discount_minor IS NOT OLD.discount_minor)
BEGIN SELECT RAISE(ABORT, 'an approved document keeps its contents'); END;
CREATE TRIGGER sales_documents_no_delete BEFORE DELETE ON sales_documents
BEGIN SELECT RAISE(ABORT, 'documents are cancelled, not deleted'); END;

CREATE TABLE sales_document_lines (
    document_id       TEXT NOT NULL REFERENCES sales_documents(id),
    position          INTEGER NOT NULL,
    product_id        TEXT REFERENCES products(id),
    name              TEXT NOT NULL,
    unit              TEXT NOT NULL,
    qty_milli         INTEGER NOT NULL CHECK (qty_milli > 0),
    unit_price_minor  INTEGER NOT NULL,
    discount_bps      INTEGER NOT NULL DEFAULT 0,
    tax_bps           INTEGER NOT NULL,
    net_minor         INTEGER NOT NULL,
    tax_minor         INTEGER NOT NULL,
    gross_minor       INTEGER NOT NULL,
    PRIMARY KEY (document_id, position)
);
-- Only a draft's lines change.
CREATE TRIGGER sales_document_lines_insert BEFORE INSERT ON sales_document_lines
WHEN (SELECT status FROM sales_documents WHERE id = NEW.document_id) <> 'draft'
BEGIN SELECT RAISE(ABORT, 'only a draft changes'); END;
CREATE TRIGGER sales_document_lines_update BEFORE UPDATE ON sales_document_lines
WHEN (SELECT status FROM sales_documents WHERE id = OLD.document_id) <> 'draft'
BEGIN SELECT RAISE(ABORT, 'only a draft changes'); END;
CREATE TRIGGER sales_document_lines_delete BEFORE DELETE ON sales_document_lines
WHEN (SELECT status FROM sales_documents WHERE id = OLD.document_id) <> 'draft'
BEGIN SELECT RAISE(ABORT, 'only a draft changes'); END;

-- The invoice remembers the document it came from.
ALTER TABLE sales ADD COLUMN document_id TEXT REFERENCES sales_documents(id);

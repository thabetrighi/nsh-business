-- Catalog (M5): categories, products, their units, barcodes and prices,
-- and a full-text index for search.

CREATE TABLE categories (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    parent_id     TEXT REFERENCES categories(id),
    name          TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    deleted_at    INTEGER
);

CREATE TABLE products (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    sku           TEXT NOT NULL,
    name          TEXT NOT NULL,
    kind          TEXT NOT NULL CHECK (kind IN ('goods', 'service')),
    category_id   TEXT REFERENCES categories(id),
    tax_bps       INTEGER NOT NULL CHECK (tax_bps BETWEEN 0 AND 10000),
    price_minor   INTEGER NOT NULL CHECK (price_minor >= 0),
    min_price_minor INTEGER,
    min_stock_milli INTEGER CHECK (min_stock_milli >= 0),
    active        INTEGER NOT NULL DEFAULT 1,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    deleted_at    INTEGER
);
CREATE UNIQUE INDEX products_sku ON products(org_id, sku) WHERE deleted_at IS NULL;

CREATE TABLE product_units (
    product_id    TEXT NOT NULL REFERENCES products(id),
    position      INTEGER NOT NULL,
    code          TEXT NOT NULL,
    factor        TEXT NOT NULL,
    precision     INTEGER NOT NULL CHECK (precision BETWEEN 0 AND 3),
    PRIMARY KEY (product_id, code)
);

CREATE TABLE product_barcodes (
    org_id        TEXT NOT NULL,
    code          TEXT NOT NULL,
    product_id    TEXT NOT NULL REFERENCES products(id),
    unit_code     TEXT NOT NULL,
    PRIMARY KEY (org_id, code)
);
CREATE INDEX product_barcodes_product ON product_barcodes(product_id);

-- Prices per list; the retail price of the base unit is also on products.
CREATE TABLE product_prices (
    id            TEXT PRIMARY KEY,
    product_id    TEXT NOT NULL REFERENCES products(id),
    list          TEXT NOT NULL,
    price_minor   INTEGER NOT NULL CHECK (price_minor >= 0),
    min_qty_milli INTEGER NOT NULL DEFAULT 0,
    branch_id     TEXT,
    customer_id   TEXT
);
CREATE INDEX product_prices_product ON product_prices(product_id);

-- Price history (M5.7): every change of a list price is kept.
CREATE TABLE price_history (
    seq           INTEGER PRIMARY KEY AUTOINCREMENT,
    product_id    TEXT NOT NULL,
    list          TEXT NOT NULL,
    old_minor     INTEGER,
    new_minor     INTEGER NOT NULL,
    changed_at    INTEGER NOT NULL,
    changed_by    TEXT
);

-- Search by name, code or barcode, in any script (diacritics removed).
CREATE VIRTUAL TABLE products_fts USING fts5(
    name, sku, barcodes,
    tokenize = 'unicode61 remove_diacritics 2',
    prefix = '2 3'
);

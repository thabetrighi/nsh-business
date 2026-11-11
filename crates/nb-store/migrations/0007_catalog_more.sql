-- Catalog, continued (M5): brands, tags, tax classes, price lists, custom
-- units, scale label layouts, product images, and product fields for
-- variants, purchase cost, weighed article codes and descriptions.

ALTER TABLE categories ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
CREATE INDEX categories_parent ON categories(parent_id) WHERE deleted_at IS NULL;

CREATE TABLE brands (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    name          TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    deleted_at    INTEGER
);
CREATE UNIQUE INDEX brands_name ON brands(org_id, lower(name)) WHERE deleted_at IS NULL;

CREATE TABLE tags (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    name          TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    device_id     TEXT NOT NULL,
    deleted_at    INTEGER
);
CREATE UNIQUE INDEX tags_name ON tags(org_id, lower(name)) WHERE deleted_at IS NULL;

CREATE TABLE product_tags (
    product_id    TEXT NOT NULL REFERENCES products(id),
    tag_id        TEXT NOT NULL REFERENCES tags(id),
    PRIMARY KEY (product_id, tag_id)
);

-- Tax classes (M5.8): a named rate products refer to (TVA 19 %, 9 %, exempt).
CREATE TABLE tax_classes (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    name          TEXT NOT NULL,
    rate_bps      INTEGER NOT NULL CHECK (rate_bps BETWEEN 0 AND 10000),
    is_default    INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    deleted_at    INTEGER
);

-- Price lists (M5.7): `retail` and `wholesale` always exist; others are the
-- organization's (semi-wholesale, distributors…). Prices refer to the code.
CREATE TABLE price_lists (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    code          TEXT NOT NULL,
    name          TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    device_id     TEXT NOT NULL,
    deleted_at    INTEGER
);
CREATE UNIQUE INDEX price_lists_code ON price_lists(org_id, code) WHERE deleted_at IS NULL;

-- Units the organization adds to the standard ones (which live in code).
CREATE TABLE units (
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    code          TEXT NOT NULL,
    name          TEXT NOT NULL,
    precision     INTEGER NOT NULL CHECK (precision BETWEEN 0 AND 3),
    created_at    INTEGER NOT NULL,
    deleted_at    INTEGER,
    PRIMARY KEY (org_id, code)
);

-- Scale label layouts (M5.6), tried in `position` order on a scan.
CREATE TABLE scale_formats (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    position      INTEGER NOT NULL,
    prefix        TEXT NOT NULL,
    item_digits   INTEGER NOT NULL,
    value_digits  INTEGER NOT NULL,
    value_kind    TEXT NOT NULL CHECK (value_kind IN ('weight', 'price')),
    decimals      INTEGER NOT NULL CHECK (decimals BETWEEN 0 AND 3),
    created_at    INTEGER NOT NULL,
    deleted_at    INTEGER
);

-- Product pictures (M5.9), kept in the database so they back up and sync
-- with the rest.
CREATE TABLE product_images (
    id            TEXT PRIMARY KEY,
    product_id    TEXT NOT NULL REFERENCES products(id),
    position      INTEGER NOT NULL,
    mime          TEXT NOT NULL CHECK (mime IN ('image/png', 'image/jpeg', 'image/webp')),
    bytes         BLOB NOT NULL,
    sha256        TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    created_by    TEXT,
    deleted_at    INTEGER
);
CREATE INDEX product_images_product ON product_images(product_id) WHERE deleted_at IS NULL;

ALTER TABLE products ADD COLUMN brand_id TEXT REFERENCES brands(id);
ALTER TABLE products ADD COLUMN tax_class_id TEXT REFERENCES tax_classes(id);
ALTER TABLE products ADD COLUMN alt_name TEXT;
ALTER TABLE products ADD COLUMN description TEXT;
ALTER TABLE products ADD COLUMN cost_minor INTEGER CHECK (cost_minor >= 0);
-- Article code printed in scale labels (weighed goods).
ALTER TABLE products ADD COLUMN plu TEXT;
-- Variants: the product they derive from, and their attribute values (JSON
-- array of [name, value]); on a parent, its attributes (JSON).
ALTER TABLE products ADD COLUMN parent_id TEXT REFERENCES products(id);
ALTER TABLE products ADD COLUMN variant_attrs TEXT;
CREATE INDEX products_parent ON products(parent_id) WHERE parent_id IS NOT NULL;
CREATE UNIQUE INDEX products_plu ON products(org_id, plu) WHERE plu IS NOT NULL AND deleted_at IS NULL;

-- Every existing organization gets the two standard lists and, in Algeria,
-- the TVA classes.
INSERT INTO price_lists (id, org_id, code, name, created_at, updated_at, device_id)
SELECT 'pl-retail-' || id, id, 'retail', 'retail', created_at, created_at, device_id FROM organizations;
INSERT INTO price_lists (id, org_id, code, name, created_at, updated_at, device_id)
SELECT 'pl-wholesale-' || id, id, 'wholesale', 'wholesale', created_at, created_at, device_id FROM organizations;
INSERT INTO tax_classes (id, org_id, name, rate_bps, is_default, created_at, updated_at, device_id)
SELECT 'tx-19-' || id, id, 'TVA 19 %', 1900, 1, created_at, created_at, device_id FROM organizations WHERE country = 'DZ';
INSERT INTO tax_classes (id, org_id, name, rate_bps, is_default, created_at, updated_at, device_id)
SELECT 'tx-9-' || id, id, 'TVA 9 %', 900, 0, created_at, created_at, device_id FROM organizations WHERE country = 'DZ';
INSERT INTO tax_classes (id, org_id, name, rate_bps, is_default, created_at, updated_at, device_id)
SELECT 'tx-0-' || id, id, 'Exonéré', 0, 0, created_at, created_at, device_id FROM organizations WHERE country = 'DZ';
INSERT INTO scale_formats (id, org_id, position, prefix, item_digits, value_digits, value_kind, decimals, created_at)
SELECT 'sf-21-' || id, id, 0, '21', 5, 5, 'weight', 3, created_at FROM organizations;

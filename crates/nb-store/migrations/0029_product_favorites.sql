-- Favourite products (owner notes): marked in the catalog, offered first
-- at the till (its favourites panel).
ALTER TABLE products ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1));
CREATE INDEX products_favorite ON products(org_id) WHERE favorite = 1 AND deleted_at IS NULL;

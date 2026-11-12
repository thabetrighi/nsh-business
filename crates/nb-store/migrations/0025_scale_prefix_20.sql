-- Scale labels starting with 20 (5-digit article, 5-digit grams), as the
-- common Algerian scales print them, for organizations that read none yet.
INSERT INTO scale_formats (id, org_id, position, prefix, item_digits, value_digits, value_kind, decimals, created_at)
SELECT 'sf-20-' || o.id, o.id,
       COALESCE((SELECT MAX(position) + 1 FROM scale_formats f WHERE f.org_id = o.id AND f.deleted_at IS NULL), 0),
       '20', 5, 5, 'weight', 3, o.created_at
FROM organizations o
WHERE NOT EXISTS (
    SELECT 1 FROM scale_formats f WHERE f.org_id = o.id AND f.prefix = '20' AND f.deleted_at IS NULL
);

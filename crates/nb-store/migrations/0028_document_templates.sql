-- Document templates (M12.11): the shop's own layouts for receipts,
-- invoices, vouchers, schedules, labels and reports (a JSON template the
-- app validates), and which layout each kind of document uses — one of
-- the shop's or a built-in one (`builtin:<code>`).
CREATE TABLE document_templates (
    id          TEXT PRIMARY KEY,
    org_id      TEXT NOT NULL REFERENCES organizations(id),
    kind        TEXT NOT NULL
        CHECK (kind IN ('receipt', 'invoice', 'voucher', 'schedule', 'label', 'report')),
    name        TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 60),
    body        TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    deleted_at  INTEGER,
    version     INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX document_templates_kind ON document_templates(org_id, kind) WHERE deleted_at IS NULL;

CREATE TABLE document_template_choice (
    org_id      TEXT NOT NULL REFERENCES organizations(id),
    kind        TEXT NOT NULL,
    choice      TEXT NOT NULL,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (org_id, kind)
);

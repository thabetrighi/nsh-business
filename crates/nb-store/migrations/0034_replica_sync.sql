-- Several devices on one organization (M11/M2): where this device is in
-- the server's log, and what a replica noticed when it ran operations made
-- elsewhere (stock that went below zero because two devices sold the same
-- goods offline, …) for someone to look at.

ALTER TABLE sync_link ADD COLUMN pull_cursor INTEGER NOT NULL DEFAULT 0;
-- Which copy of the server's log the cursor counts in: a device that sends
-- its whole database again starts a new one, and the others catch up.
ALTER TABLE sync_link ADD COLUMN pull_epoch INTEGER NOT NULL DEFAULT 0;

CREATE TABLE reviews (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL,
    entity        TEXT NOT NULL,
    entity_id     TEXT NOT NULL,
    detail        TEXT NOT NULL DEFAULT '{}',
    operation_id  TEXT,
    created_at    INTEGER NOT NULL,
    resolved_at   INTEGER,
    resolved_by   TEXT
);
CREATE INDEX reviews_open ON reviews(created_at) WHERE resolved_at IS NULL;

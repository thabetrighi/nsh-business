-- What this device has sent to its server (M11/M1): an operation is sent
-- once the server has taken it (or the whole database was sent as a
-- snapshot). Where the device syncs to, and as what.

ALTER TABLE operations ADD COLUMN pushed_at INTEGER;
CREATE INDEX operations_unpushed ON operations(created_at) WHERE pushed_at IS NULL AND call IS NOT NULL;

CREATE TABLE sync_link (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    server_url   TEXT NOT NULL,
    token        TEXT NOT NULL,
    linked_at    INTEGER NOT NULL,
    last_push_at INTEGER,
    last_error   TEXT NOT NULL DEFAULT ''
);

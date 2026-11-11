-- Identity and access (M4): organization settings and legal profile, cash
-- registers, custom roles, role grants over scopes, credentials, feature
-- switches, supervisor approvals.

ALTER TABLE organizations ADD COLUMN language TEXT NOT NULL DEFAULT 'ar';
ALTER TABLE organizations ADD COLUMN fiscal_year_start INTEGER NOT NULL DEFAULT 1
    CHECK (fiscal_year_start BETWEEN 1 AND 12);
-- Minutes without activity before the screen locks (0 = never).
ALTER TABLE organizations ADD COLUMN auto_lock_min INTEGER NOT NULL DEFAULT 5
    CHECK (auto_lock_min BETWEEN 0 AND 240);

ALTER TABLE users ADD COLUMN phone TEXT;
ALTER TABLE users ADD COLUMN active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1));

-- Legal identity printed on documents (Algeria: NIF, NIS, RC, AI).
CREATE TABLE business_profiles (
    org_id        TEXT PRIMARY KEY REFERENCES organizations(id),
    legal_name    TEXT NOT NULL DEFAULT '',
    nif           TEXT NOT NULL DEFAULT '',
    nis           TEXT NOT NULL DEFAULT '',
    rc            TEXT NOT NULL DEFAULT '',
    ai            TEXT NOT NULL DEFAULT '',
    address       TEXT NOT NULL DEFAULT '',
    phone         TEXT NOT NULL DEFAULT '',
    email         TEXT NOT NULL DEFAULT '',
    updated_at    INTEGER NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE cash_registers (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    branch_id     TEXT NOT NULL REFERENCES branches(id),
    code          TEXT NOT NULL,
    name          TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    deleted_at    INTEGER
);
CREATE UNIQUE INDEX cash_registers_code ON cash_registers(branch_id, code) WHERE deleted_at IS NULL;

-- Roles defined by the organization; built-in roles live in the code.
-- `permissions` is a JSON array of permission codes.
CREATE TABLE roles (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    name          TEXT NOT NULL,
    permissions   TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    deleted_at    INTEGER
);
CREATE UNIQUE INDEX roles_name ON roles(org_id, lower(name)) WHERE deleted_at IS NULL;

-- A role (built-in code or custom role id) granted over a scope.
CREATE TABLE user_roles (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    user_id       TEXT NOT NULL REFERENCES users(id),
    role          TEXT NOT NULL,
    scope_kind    TEXT NOT NULL CHECK (scope_kind IN ('org', 'branch', 'warehouse', 'register')),
    scope_id      TEXT,
    created_at    INTEGER NOT NULL,
    created_by    TEXT,
    device_id     TEXT NOT NULL,
    deleted_at    INTEGER,
    CHECK ((scope_kind = 'org') = (scope_id IS NULL))
);
CREATE UNIQUE INDEX user_roles_unique
    ON user_roles(user_id, role, scope_kind, ifnull(scope_id, '')) WHERE deleted_at IS NULL;
CREATE INDEX user_roles_user ON user_roles(user_id) WHERE deleted_at IS NULL;

-- Sign-in secrets (argon2id hashes, PHC strings). Never in events or audit.
CREATE TABLE user_credentials (
    user_id       TEXT PRIMARY KEY REFERENCES users(id),
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    login         TEXT,
    password_hash TEXT,
    pin_hash      TEXT,
    failures      INTEGER NOT NULL DEFAULT 0,
    locked_until  INTEGER,
    updated_at    INTEGER NOT NULL
);
CREATE UNIQUE INDEX user_credentials_login ON user_credentials(org_id, login) WHERE login IS NOT NULL;

-- Feature switches over the activity profile (M4.4).
CREATE TABLE org_features (
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    feature       TEXT NOT NULL,
    enabled       INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    updated_at    INTEGER NOT NULL,
    PRIMARY KEY (org_id, feature)
);

-- A supervisor's approval of a sensitive action, usable once, shortly
-- (M4.8). Only `used_*` is ever filled in afterwards.
CREATE TABLE approvals (
    id            TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES organizations(id),
    permission    TEXT NOT NULL,
    requested_by  TEXT NOT NULL REFERENCES users(id),
    approved_by   TEXT NOT NULL REFERENCES users(id),
    reason        TEXT NOT NULL,
    branch_id     TEXT,
    warehouse_id  TEXT,
    register_id   TEXT,
    created_at    INTEGER NOT NULL,
    expires_at    INTEGER NOT NULL,
    used_at       INTEGER,
    used_by_operation TEXT
);
CREATE TRIGGER approvals_frozen BEFORE UPDATE ON approvals
WHEN OLD.used_at IS NOT NULL
  OR NEW.id IS NOT OLD.id OR NEW.permission IS NOT OLD.permission
  OR NEW.requested_by IS NOT OLD.requested_by OR NEW.approved_by IS NOT OLD.approved_by
  OR NEW.expires_at IS NOT OLD.expires_at
BEGIN SELECT RAISE(ABORT, 'approvals are used once and never changed'); END;
CREATE TRIGGER approvals_no_delete BEFORE DELETE ON approvals
BEGIN SELECT RAISE(ABORT, 'approvals are never deleted'); END;

-- Existing users keep their role, over the whole organization.
INSERT INTO user_roles (id, org_id, user_id, role, scope_kind, scope_id, created_at, created_by, device_id)
SELECT 'ur-' || id, org_id, id, role, 'org', NULL, created_at, created_by, device_id FROM users;

INSERT INTO business_profiles (org_id, legal_name, updated_at)
SELECT id, name, updated_at FROM organizations;

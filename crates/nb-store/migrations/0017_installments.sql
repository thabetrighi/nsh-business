-- Instalment plans (M9.6–M9.8). A plan schedules what an invoice on
-- account still owes; its instalments never change. A payment is a receipt
-- given to the invoice, and to the instalments oldest first. Rescheduling
-- makes a new version of the plan from what is left: the old one stays,
-- with who approved it and why.

CREATE TABLE installment_plans (
    id              TEXT PRIMARY KEY,
    org_id          TEXT NOT NULL REFERENCES organizations(id),
    customer_id     TEXT NOT NULL REFERENCES customers(id),
    sale_id         TEXT NOT NULL REFERENCES sales(id),
    number          TEXT NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1,
    previous_id     TEXT REFERENCES installment_plans(id),
    -- What the invoice came to, what was paid at the sale, what is financed.
    total_minor     INTEGER NOT NULL,
    down_minor      INTEGER NOT NULL,
    financed_minor  INTEGER NOT NULL CHECK (financed_minor > 0),
    count           INTEGER NOT NULL CHECK (count BETWEEN 1 AND 60),
    frequency       TEXT NOT NULL CHECK (frequency IN ('weekly', 'monthly', 'quarterly')),
    first_due       TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'done', 'rescheduled')),
    reason          TEXT NOT NULL DEFAULT '',
    approval_id     TEXT,
    created_at      INTEGER NOT NULL,
    created_by      TEXT,
    device_id       TEXT NOT NULL,
    UNIQUE (org_id, number, version)
);
CREATE INDEX installment_plans_customer ON installment_plans (customer_id, status);
-- One plan in force per invoice.
CREATE UNIQUE INDEX installment_plans_one_active ON installment_plans (sale_id) WHERE status = 'active';
-- Only the state of a plan moves.
CREATE TRIGGER installment_plans_contents BEFORE UPDATE ON installment_plans
WHEN NEW.financed_minor IS NOT OLD.financed_minor OR NEW.count IS NOT OLD.count
  OR NEW.first_due IS NOT OLD.first_due OR NEW.frequency IS NOT OLD.frequency
  OR OLD.status <> 'active'
BEGIN SELECT RAISE(ABORT, 'a plan keeps its schedule'); END;
CREATE TRIGGER installment_plans_no_delete BEFORE DELETE ON installment_plans
BEGIN SELECT RAISE(ABORT, 'plans are kept'); END;

CREATE TABLE installments (
    plan_id         TEXT NOT NULL REFERENCES installment_plans(id),
    seq             INTEGER NOT NULL,
    due_date        TEXT NOT NULL,
    amount_minor    INTEGER NOT NULL CHECK (amount_minor > 0),
    PRIMARY KEY (plan_id, seq)
);
CREATE INDEX installments_due ON installments (due_date);
CREATE TRIGGER installments_no_update BEFORE UPDATE ON installments
BEGIN SELECT RAISE(ABORT, 'instalments are kept as scheduled'); END;
CREATE TRIGGER installments_no_delete BEFORE DELETE ON installments
BEGIN SELECT RAISE(ABORT, 'instalments are kept as scheduled'); END;

CREATE TABLE installment_payments (
    receipt_id      TEXT NOT NULL REFERENCES receipts(id),
    plan_id         TEXT NOT NULL,
    seq             INTEGER NOT NULL,
    amount_minor    INTEGER NOT NULL CHECK (amount_minor > 0),
    PRIMARY KEY (receipt_id, plan_id, seq),
    FOREIGN KEY (plan_id, seq) REFERENCES installments (plan_id, seq)
);
CREATE TRIGGER installment_payments_no_update BEFORE UPDATE ON installment_payments
BEGIN SELECT RAISE(ABORT, 'payments are immutable'); END;
CREATE TRIGGER installment_payments_no_delete BEFORE DELETE ON installment_payments
BEGIN SELECT RAISE(ABORT, 'payments are immutable'); END;

CREATE TABLE plan_counters (
    org_id  TEXT PRIMARY KEY REFERENCES organizations(id),
    last    INTEGER NOT NULL
);

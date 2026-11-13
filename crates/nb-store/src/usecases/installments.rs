//! Instalment plans (M9.6–M9.8): an invoice on account paid in instalments,
//! their payment (a receipt given to the invoice, then to the instalments
//! oldest first), and rescheduling as a new version that keeps the old one.

use nb_core::Money;
use nb_domain::RuleError;
use nb_domain::access::{Permission, Target};
use nb_domain::fiscal::Date;
use nb_domain::installment::{self, Frequency, State};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::uow::{OpMeta, Writer};
use crate::usecases::debts::{AllocationInput, ReceiptInput, ReceiptResult, collect_in};
use crate::usecases::sales::the_org;
use crate::{Error, Result, Store};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlanInput {
    pub sale_id: String,
    pub count: u32,
    /// `weekly`, `monthly` or `quarterly`.
    pub frequency: String,
    /// `YYYY-MM-DD`.
    pub first_due: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallmentRow {
    pub seq: u32,
    pub due_date: String,
    pub amount_minor: i64,
    pub paid_minor: i64,
    /// `upcoming`, `due`, `partial`, `paid` or `late` (today).
    pub state: String,
}

impl InstallmentRow {
    pub fn open_minor(&self) -> i64 {
        (self.amount_minor - self.paid_minor).max(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub number: String,
    pub version: u32,
    pub customer_id: String,
    pub customer_name: String,
    pub sale_id: String,
    pub sale_number: String,
    pub total_minor: i64,
    pub down_minor: i64,
    pub financed_minor: i64,
    pub count: u32,
    pub frequency: String,
    pub first_due: String,
    /// `active`, `done` or `rescheduled`.
    pub status: String,
    pub reason: String,
    pub created_at: i64,
    pub installments: Vec<InstallmentRow>,
}

impl Plan {
    pub fn paid_minor(&self) -> i64 {
        self.installments.iter().map(|i| i.paid_minor).sum()
    }
    pub fn open_minor(&self) -> i64 {
        self.installments
            .iter()
            .map(InstallmentRow::open_minor)
            .sum()
    }
    pub fn late_minor(&self) -> i64 {
        self.installments
            .iter()
            .filter(|i| i.state == "late")
            .map(InstallmentRow::open_minor)
            .sum()
    }
    /// The next instalment still owing something.
    pub fn next(&self) -> Option<&InstallmentRow> {
        self.installments.iter().find(|i| i.open_minor() > 0)
    }
}

/// Today in the store's calendar.
fn today(store_now: i64, offset: i32) -> Date {
    Date::parse(&crate::time::iso_date(store_now, offset)).unwrap_or(Date {
        y: 1970,
        m: 1,
        d: 1,
    })
}

fn w_today(w: &Writer<'_>) -> Date {
    let (y, m, d) = w.date();
    Date { y, m, d }
}

fn load(conn: &Connection, id: &str, today: Date) -> Result<Plan> {
    let mut plan = conn
        .query_row(
            "SELECT p.id, p.number, p.version, p.customer_id, c.name, p.sale_id, s.number, p.total_minor,
                    p.down_minor, p.financed_minor, p.count, p.frequency, p.first_due, p.status, p.reason,
                    p.created_at
             FROM installment_plans p JOIN customers c ON c.id = p.customer_id JOIN sales s ON s.id = p.sale_id
             WHERE p.id = ?1",
            [id],
            |r| {
                Ok(Plan {
                    id: r.get(0)?,
                    number: r.get(1)?,
                    version: r.get(2)?,
                    customer_id: r.get(3)?,
                    customer_name: r.get(4)?,
                    sale_id: r.get(5)?,
                    sale_number: r.get(6)?,
                    total_minor: r.get(7)?,
                    down_minor: r.get(8)?,
                    financed_minor: r.get(9)?,
                    count: r.get(10)?,
                    frequency: r.get(11)?,
                    first_due: r.get(12)?,
                    status: r.get(13)?,
                    reason: r.get(14)?,
                    created_at: r.get(15)?,
                    installments: Vec::new(),
                })
            },
        )
        .optional()?
        .ok_or(Error::Rules(vec![RuleError::on("plan", "plan.unknown")]))?;
    let mut s = conn.prepare(
        "SELECT i.seq, i.due_date, i.amount_minor,
                COALESCE((SELECT SUM(p.amount_minor) FROM installment_payments p
                          WHERE p.plan_id = i.plan_id AND p.seq = i.seq), 0)
                + COALESCE((SELECT SUM(a.amount_minor) FROM installment_adjustments a
                            WHERE a.plan_id = i.plan_id AND a.seq = i.seq), 0)
         FROM installments i WHERE i.plan_id = ?1 ORDER BY i.seq",
    )?;
    let rows = s
        .query_map([id], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let rescheduled = plan.status == "rescheduled";
    plan.installments = rows
        .into_iter()
        .map(|(seq, due_date, amount, paid)| {
            let due = Date::parse(&due_date).unwrap_or(today);
            let state = installment::state(due, amount, paid, today);
            InstallmentRow {
                seq,
                // A rescheduled plan's open instalments moved to the new one.
                state: if rescheduled && state != State::Paid {
                    "rescheduled".into()
                } else {
                    state.code().into()
                },
                due_date,
                amount_minor: amount,
                paid_minor: paid,
            }
        })
        .collect();
    Ok(plan)
}

pub fn get_plan(store: &Store, id: &str) -> Result<Plan> {
    let t = today(store.clock.now_ms(), store.utc_offset_min);
    store.read(|r| load(r.conn(), id, t))
}

/// A customer's plans, the ones in force first, each with its versions.
pub fn plans_of(store: &Store, customer: &str) -> Result<Vec<Plan>> {
    let t = today(store.clock.now_ms(), store.utc_offset_min);
    store.read(|r| {
        let mut s = r.conn().prepare(
            "SELECT id FROM installment_plans WHERE customer_id = ?1
             ORDER BY status = 'active' DESC, created_at DESC, version DESC",
        )?;
        let ids = s
            .query_map([customer], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.iter().map(|id| load(r.conn(), id, t)).collect()
    })
}

/// An instalment owed now or soon, for the instalments page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DueRow {
    pub plan_id: String,
    pub plan_number: String,
    pub customer_id: String,
    pub customer_name: String,
    pub phone: String,
    pub seq: u32,
    pub count: u32,
    pub due_date: String,
    pub open_minor: i64,
    pub state: String,
    /// Days past its date (negative: days to go).
    pub days_late: i64,
}

/// Instalments of plans in force, unpaid and due within `days` (or late),
/// the latest due first.
pub fn due_installments(store: &Store, days: i64) -> Result<Vec<DueRow>> {
    let t = today(store.clock.now_ms(), store.utc_offset_min);
    let until = installment::add_days(t, days).iso();
    store.read(|r| {
        let (org, _) = the_org(r.conn())?;
        let mut s = r.conn().prepare(
            "SELECT p.id, p.number, p.customer_id, c.name, c.phone, i.seq, p.count, i.due_date,
                    i.amount_minor - COALESCE((SELECT SUM(x.amount_minor) FROM installment_payments x
                                               WHERE x.plan_id = i.plan_id AND x.seq = i.seq), 0)
                    - COALESCE((SELECT SUM(a.amount_minor) FROM installment_adjustments a
                                WHERE a.plan_id = i.plan_id AND a.seq = i.seq), 0)
             FROM installments i JOIN installment_plans p ON p.id = i.plan_id JOIN customers c ON c.id = p.customer_id
             WHERE p.org_id = ?1 AND p.status = 'active' AND i.due_date <= ?2
             ORDER BY i.due_date, c.name",
        )?;
        let rows = s
            .query_map(params![org.to_string(), until], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, u32>(5)?,
                    r.get::<_, u32>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, i64>(8)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter(|row| row.8 > 0)
            .map(
                |(plan_id, plan_number, customer_id, customer_name, phone, seq, count, due_date, open)| {
                    let due = Date::parse(&due_date).unwrap_or(t);
                    DueRow {
                        state: installment::state(due, open, 0, t).code().into(),
                        days_late: installment::days_between(due, t),
                        plan_id,
                        plan_number,
                        customer_id,
                        customer_name,
                        phone,
                        seq,
                        count,
                        due_date,
                        open_minor: open,
                    }
                },
            )
            .collect())
    })
}

fn check(input: &PlanInput) -> Result<(Frequency, Date)> {
    let f = Frequency::from_code(&input.frequency)
        .ok_or(Error::from(RuleError::on("frequency", "invalid_value")))?;
    let first = Date::parse(&input.first_due)
        .map_err(|_| Error::from(RuleError::on("first_due", "invalid_date")))?;
    Ok((f, first))
}

/// Writes a plan and its instalments; returns its id.
#[allow(clippy::too_many_arguments)]
fn write_plan(
    w: &mut Writer<'_>,
    customer: &str,
    sale: &str,
    number: &str,
    version: u32,
    previous: Option<&str>,
    total: i64,
    financed: i64,
    input: &PlanInput,
    reason: &str,
    approval: Option<&str>,
    currency: nb_core::Currency,
) -> Result<String> {
    let (f, first) = check(input)?;
    let dues = installment::schedule(Money::from_minor(financed, currency), input.count, f, first)?;
    let id = nb_core::id::idgen::uuid().to_string();
    w.tx().execute(
        "INSERT INTO installment_plans (id, org_id, customer_id, sale_id, number, version, previous_id, total_minor,
                down_minor, financed_minor, count, frequency, first_due, reason, approval_id, created_at, created_by,
                device_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            id,
            w.org()?.to_string(),
            customer,
            sale,
            number,
            version,
            previous,
            total,
            total - financed,
            financed,
            input.count,
            f.code(),
            first.iso(),
            reason.trim(),
            approval,
            w.now(),
            w.actor_text(),
            w.device().id.to_string()
        ],
    )?;
    for (i, d) in dues.iter().enumerate() {
        w.tx().execute(
            "INSERT INTO installments (plan_id, seq, due_date, amount_minor) VALUES (?1, ?2, ?3, ?4)",
            params![id, i64::try_from(i + 1).unwrap_or(0), d.date.iso(), d.amount.minor()],
        )?;
    }
    Ok(id)
}

/// Splits what an invoice on account still owes into instalments.
pub fn create_plan(store: &mut Store, meta: OpMeta, input: PlanInput) -> Result<Plan> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::InstallmentsCreatePlan {
            input: input.clone(),
        },
    )?;
    check(&input)?;
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let now = store.clock.now_ms();
    let id = store.write(meta, |w| {
        w.require(Permission::DebtsCollect, &Target::org())?;
        let (customer, total): (Option<String>, i64) = w
            .tx()
            .query_row(
                "SELECT customer_id, total_minor FROM sales WHERE id = ?1",
                [&input.sale_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(Error::Rules(vec![RuleError::on(
                "sale",
                "return.sale_unknown",
            )]))?;
        let customer = customer.ok_or(Error::Rules(vec![RuleError::on(
            "sale",
            "plan.no_customer",
        )]))?;
        let open = crate::usecases::debts::open_invoices_of(w.tx(), &customer, now)?
            .into_iter()
            .find(|o| o.sale_id == input.sale_id)
            .map_or(0, |o| o.open_minor);
        if open <= 0 {
            return Err(RuleError::on("sale", "plan.nothing_to_finance").into());
        }
        let taken: i64 = w.tx().query_row(
            "SELECT COUNT(*) FROM installment_plans WHERE sale_id = ?1 AND status = 'active'",
            [&input.sale_id],
            |r| r.get(0),
        )?;
        if taken > 0 {
            return Err(RuleError::on("sale", "plan.already").into());
        }
        let number = w.next_code("plan", "TQ")?;
        let id = write_plan(
            w,
            &customer,
            &input.sale_id,
            &number,
            1,
            None,
            total,
            open,
            &input,
            "",
            None,
            currency,
        )?;
        let t = w_today(w);
        let plan = load(w.tx(), &id, t)?;
        w.event("plan.created", "installment_plan", &id, &plan)?;
        w.audit("create", "installment_plan", &id, &input)?;
        Ok(id)
    })?;
    get_plan(store, &id)
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RescheduleInput {
    pub count: u32,
    pub frequency: String,
    pub first_due: String,
    pub reason: String,
    #[serde(default)]
    pub approval: Option<String>,
}

/// A new schedule for what a plan still owes; the plan as it was is kept
/// (rescheduled), with who approved it and why.
pub fn reschedule(
    store: &mut Store,
    meta: OpMeta,
    plan_id: &str,
    input: RescheduleInput,
) -> Result<Plan> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::InstallmentsReschedule {
            plan_id: plan_id.to_owned(),
            input: input.clone(),
        },
    )?;
    if input.reason.trim().is_empty() {
        return Err(RuleError::on("reason", "required").into());
    }
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let id = store.write(meta, |w| {
        let approval = input
            .approval
            .as_deref()
            .map(crate::repo::parse_id)
            .transpose()?;
        w.require_approved(Permission::InstallmentsReschedule, &Target::org(), approval)?;
        let t = w_today(w);
        let old = load(w.tx(), plan_id, t)?;
        if old.status != "active" {
            return Err(RuleError::on("plan", "plan.not_active").into());
        }
        let left = old.open_minor();
        let plan_input = PlanInput {
            sale_id: old.sale_id.clone(),
            count: input.count,
            frequency: input.frequency.clone(),
            first_due: input.first_due.clone(),
        };
        check(&plan_input)?;
        w.tx().execute(
            "UPDATE installment_plans SET status = 'rescheduled' WHERE id = ?1",
            [plan_id],
        )?;
        let id = write_plan(
            w,
            &old.customer_id,
            &old.sale_id,
            &old.number,
            old.version + 1,
            Some(plan_id),
            old.total_minor,
            left,
            &plan_input,
            &input.reason,
            input.approval.as_deref(),
            currency,
        )?;
        let plan = load(w.tx(), &id, t)?;
        w.event("plan.rescheduled", "installment_plan", &id, &plan)?;
        w.audit("reschedule", "installment_plan", plan_id, &input)?;
        Ok(id)
    })?;
    get_plan(store, &id)
}

/// After money was given to `sale`, gives it to its plan's instalments
/// (oldest first) and closes the plan once all is paid.
pub(crate) fn settle_plan(
    w: &mut Writer<'_>,
    receipt: &str,
    sale: &str,
    amount: i64,
) -> Result<()> {
    settle_plan_by(w, Some(receipt), None, sale, amount)
}

/// [`settle_plan`] for a receipt or for an adjustment (write-off, allowance).
pub(crate) fn settle_plan_by(
    w: &mut Writer<'_>,
    receipt: Option<&str>,
    adjustment: Option<&str>,
    sale: &str,
    amount: i64,
) -> Result<()> {
    let plan: Option<String> = w
        .tx()
        .query_row(
            "SELECT id FROM installment_plans WHERE sale_id = ?1 AND status = 'active'",
            [sale],
            |r| r.get(0),
        )
        .optional()?;
    let Some(plan) = plan else {
        return Ok(());
    };
    let t = w_today(w);
    let p = load(w.tx(), &plan, t)?;
    let open: Vec<i64> = p
        .installments
        .iter()
        .map(InstallmentRow::open_minor)
        .collect();
    let (parts, _) = installment::settle(&open, amount);
    for (i, take) in parts {
        let seq = p.installments[i].seq;
        match (receipt, adjustment) {
            (Some(r), _) => w.tx().execute(
                "INSERT INTO installment_payments (receipt_id, plan_id, seq, amount_minor) VALUES (?1, ?2, ?3, ?4)",
                params![r, plan, seq, take],
            )?,
            (None, Some(a)) => w.tx().execute(
                "INSERT INTO installment_adjustments (adjustment_id, plan_id, seq, amount_minor)
                 VALUES (?1, ?2, ?3, ?4)",
                params![a, plan, seq, take],
            )?,
            (None, None) => 0,
        };
    }
    if load(w.tx(), &plan, t)?.open_minor() == 0 {
        w.tx().execute(
            "UPDATE installment_plans SET status = 'done' WHERE id = ?1",
            [&plan],
        )?;
    }
    Ok(())
}

/// Pays instalments of a plan: a receipt given to its invoice.
pub fn pay_installments(
    store: &mut Store,
    meta: OpMeta,
    plan_id: &str,
    amount_minor: i64,
    method: &str,
    register_id: Option<String>,
) -> Result<ReceiptResult> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::InstallmentsPayInstallments {
            plan_id: plan_id.to_owned(),
            amount_minor: amount_minor.clone(),
            method: method.to_owned(),
            register_id: register_id.clone(),
        },
    )?;
    let (_, currency) = store.read(|r| the_org(r.conn()))?;
    let offset = store.utc_offset_min;
    let plan = get_plan(store, plan_id)?;
    if plan.status != "active" {
        return Err(RuleError::on("plan", "plan.not_active").into());
    }
    if amount_minor > plan.open_minor() {
        return Err(RuleError::on("amount", "plan.more_than_left").into());
    }
    let input = ReceiptInput {
        customer_id: plan.customer_id.clone(),
        amount_minor,
        method: method.into(),
        register_id,
        allocations: vec![AllocationInput {
            sale_id: plan.sale_id.clone(),
            amount_minor,
        }],
        unallocated: false,
        reference: plan.number.clone(),
        note: String::new(),
    };
    store.write(meta, |w| collect_in(w, &input, currency, offset))
}

//! A sale in progress (M8.1–M8.3): lines with their quantity, unit price,
//! tax rate and discount; a discount on the whole sale shared across its
//! lines; totals that always add up; and how it is paid (cash with change,
//! card, transfer, on the customer's account, or a mix).

use nb_core::{Money, Quantity, Rate};

use crate::RuleError;
use crate::tax::{RateTotal, TaxMode, Taxed, split, summary};

/// A reduction: a share of the amount (basis points) or a fixed amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Discount {
    #[default]
    None,
    /// 1000 = 10 %.
    Percent(i64),
    Amount(Money),
}

impl Discount {
    /// The reduction of `amount` (never more than it).
    fn of(self, amount: Money) -> Result<Money, RuleError> {
        let d = match self {
            Discount::None => Money::zero(amount.currency()),
            Discount::Percent(bps) => {
                if !(0..=10_000).contains(&bps) {
                    return Err(RuleError::on("discount", "sale.discount_invalid"));
                }
                amount.mul_ratio(bps, 10_000)?
            }
            Discount::Amount(m) => {
                if m.is_negative() {
                    return Err(RuleError::on("discount", "sale.discount_invalid"));
                }
                m
            }
        };
        if d.minor() > amount.minor() {
            return Err(RuleError::on("discount", "sale.discount_too_big"));
        }
        Ok(d)
    }
}

/// One line of a sale. Prices are per sale unit, typed in the cart's tax
/// mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CartLine {
    /// Quantity in the sale unit.
    pub qty: Quantity,
    pub unit_price: Money,
    /// The lowest price per sale unit allowed without permission.
    pub min_price: Option<Money>,
    pub tax_rate: Rate,
    pub discount: Discount,
}

/// A sale: its lines, a discount on the whole, and how prices are typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cart {
    pub mode: TaxMode,
    pub lines: Vec<CartLine>,
    pub discount: Discount,
}

/// The figures of one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineTotal {
    /// Quantity × price.
    pub base: Money,
    pub line_discount: Money,
    /// Its share of the sale's discount.
    pub sale_discount: Money,
    /// What the line comes to, split into net, tax and gross.
    pub taxed: Taxed,
    /// Sold below its floor price (needs permission).
    pub below_min: bool,
    /// Its discount (line and share of the sale's) in basis points of its
    /// base: checked against what the seller may give.
    pub discount_bps: i64,
}

/// Every figure of the sale; `net + tax = total` and the lines add up to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Totals {
    pub lines: Vec<LineTotal>,
    /// Before any discount.
    pub gross_before: Money,
    pub discounts: Money,
    pub net: Money,
    pub tax: Money,
    pub total: Money,
    pub by_rate: Vec<RateTotal>,
}

impl Cart {
    pub fn totals(&self) -> Result<Totals, RuleError> {
        let Some(first) = self.lines.first() else {
            return Err(RuleError::on("lines", "sale.empty"));
        };
        let currency = first.unit_price.currency();
        let zero = Money::zero(currency);
        let mut after_line = Vec::with_capacity(self.lines.len());
        let mut lines = Vec::with_capacity(self.lines.len());
        for l in &self.lines {
            if l.qty.is_zero() || l.qty.is_negative() {
                return Err(RuleError::on("qty", "sale.qty_positive"));
            }
            if l.unit_price.is_negative() {
                return Err(RuleError::on("price", "ledger.negative"));
            }
            let base = l.qty.times_price(l.unit_price)?;
            let line_discount = l.discount.of(base)?;
            after_line.push(base.checked_sub(line_discount)?);
            lines.push((base, line_discount));
        }
        let subtotal = Money::sum(currency, &after_line)?;
        let sale_discount = self.discount.of(subtotal)?;
        // The sale's discount, shared in proportion to what each line comes
        // to (to the cent: the shares add up to it).
        let shares = if sale_discount.is_zero() || subtotal.is_zero() {
            vec![zero; self.lines.len()]
        } else {
            let weights: Vec<u64> = after_line
                .iter()
                .map(|m| u64::try_from(m.minor()).unwrap_or(0))
                .collect();
            sale_discount.allocate(&weights)?
        };
        let mut out = Vec::with_capacity(self.lines.len());
        let mut taxed_all = Vec::with_capacity(self.lines.len());
        for (i, l) in self.lines.iter().enumerate() {
            let (base, line_discount) = lines[i];
            let amount = after_line[i].checked_sub(shares[i])?;
            let taxed = split(amount, l.tax_rate, self.mode)?;
            let below_min = match l.min_price {
                Some(min) => amount.minor() < l.qty.times_price(min)?.minor(),
                None => false,
            };
            let off = line_discount.checked_add(shares[i])?;
            let discount_bps = if base.is_zero() {
                0
            } else {
                i64::try_from(i128::from(off.minor()) * 10_000 / i128::from(base.minor()))
                    .unwrap_or(i64::MAX)
            };
            taxed_all.push((l.tax_rate, taxed));
            out.push(LineTotal {
                base,
                line_discount,
                sale_discount: shares[i],
                taxed,
                below_min,
                discount_bps,
            });
        }
        let sum = |f: fn(&LineTotal) -> Money| -> Result<Money, RuleError> {
            let v: Vec<Money> = out.iter().map(f).collect();
            Ok(Money::sum(currency, &v)?)
        };
        let gross_before = sum(|l| l.base)?;
        let line_discounts = sum(|l| l.line_discount)?;
        Ok(Totals {
            gross_before,
            discounts: line_discounts.checked_add(sale_discount)?,
            net: sum(|l| l.taxed.net)?,
            tax: sum(|l| l.taxed.tax)?,
            total: sum(|l| l.taxed.gross)?,
            by_rate: summary(&taxed_all)?,
            lines: out,
        })
    }
}

/// How money is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Cash,
    Card,
    Transfer,
    /// On the customer's account (paid later).
    Credit,
}

impl Method {
    pub fn code(self) -> &'static str {
        match self {
            Method::Cash => "cash",
            Method::Card => "card",
            Method::Transfer => "transfer",
            Method::Credit => "credit",
        }
    }
    pub fn from_code(s: &str) -> Option<Self> {
        [Method::Cash, Method::Card, Method::Transfer, Method::Credit]
            .into_iter()
            .find(|m| m.code() == s)
    }
}

/// What a sale of `due` was paid with: cash kept (after change), card,
/// transfer, and what remains on the customer's account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settlement {
    pub cash: Money,
    pub card: Money,
    pub transfer: Money,
    pub credit: Money,
    /// Cash given back.
    pub change: Money,
}

/// Settles `due` with the `given` amounts: card and transfer cannot exceed
/// it; cash covers the rest and gives change; what is still missing goes on
/// the customer's account when `credit_allowed`, else the sale is
/// underpaid. An explicit `Credit` amount is taken as asked.
pub fn settle(
    due: Money,
    given: &[(Method, Money)],
    credit_allowed: bool,
) -> Result<Settlement, RuleError> {
    let zero = Money::zero(due.currency());
    let by = |m: Method| -> Result<Money, RuleError> {
        let mut total = zero;
        for (method, amount) in given {
            if amount.is_negative() {
                return Err(RuleError::on("payments", "ledger.negative"));
            }
            if *method == m {
                total = total.checked_add(*amount)?;
            }
        }
        Ok(total)
    };
    let (cash_given, card, transfer, credit_asked) = (
        by(Method::Cash)?,
        by(Method::Card)?,
        by(Method::Transfer)?,
        by(Method::Credit)?,
    );
    let non_cash = card.checked_add(transfer)?.checked_add(credit_asked)?;
    if non_cash.minor() > due.minor() {
        return Err(RuleError::on("payments", "sale.overpaid"));
    }
    let rest = due.checked_sub(non_cash)?;
    let cash = Money::from_minor(cash_given.minor().min(rest.minor()), due.currency());
    let change = cash_given.checked_sub(cash)?;
    let missing = rest.checked_sub(cash)?;
    if !missing.is_zero() && !credit_allowed {
        return Err(RuleError::on("payments", "sale.underpaid"));
    }
    if (!credit_asked.is_zero() || !missing.is_zero()) && !credit_allowed {
        return Err(RuleError::on("payments", "sale.credit_needs_customer"));
    }
    Ok(Settlement {
        cash,
        card,
        transfer,
        credit: credit_asked.checked_add(missing)?,
        change,
    })
}

/// What the till shows a customer paying in cash ("bon par bon"): of the
/// cash `given` for a sale of `due`, what goes to their older debt
/// (`owed`), the change, and what they owe after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BonParBon {
    /// Paid on the older debt.
    pub settle: i64,
    pub change: i64,
    /// Left on the account from this sale (cash short of it).
    pub on_account: i64,
    /// What the customer owes after the sale.
    pub after: i64,
}

/// Cash beyond the sale pays the older debt first, then comes back as
/// change; cash short of the sale goes on the account. In minor units.
pub fn bon_par_bon(due: i64, given: i64, owed: i64) -> BonParBon {
    let due = due.max(0);
    let given = given.max(0);
    if given >= due {
        let settle = (given - due).min(owed.max(0));
        BonParBon {
            settle,
            change: given - due - settle,
            on_account: 0,
            after: owed - settle,
        }
    } else {
        BonParBon {
            settle: 0,
            change: 0,
            on_account: due - given,
            after: owed + due - given,
        }
    }
}

/// "Bon par bon": of what the customer hands over at a sale, `settle`
/// pays their older debt (`owed`) instead of the sale. It comes out of one
/// method — cash first, then card, then transfer — and what is left must
/// still pay the whole sale (`due`): paying old debt while leaving new debt
/// would only move it. Returns what is left for the sale, and the method
/// the debt is paid with.
pub fn take_settlement(
    due: Money,
    given: &[(Method, Money)],
    settle: Money,
    owed: Money,
) -> Result<(Vec<(Method, Money)>, Method), RuleError> {
    if settle.is_negative() {
        return Err(RuleError::on("settle", "ledger.negative"));
    }
    if settle.minor() > owed.minor().max(0) {
        return Err(RuleError::on("settle", "sale.settle_more_than_owed"));
    }
    let total = |m: Method| {
        given
            .iter()
            .filter(|(g, _)| *g == m)
            .map(|(_, a)| a.minor())
            .sum::<i64>()
    };
    let method = [Method::Cash, Method::Card, Method::Transfer]
        .into_iter()
        .find(|m| total(*m) >= settle.minor())
        .ok_or(RuleError::on("settle", "sale.settle_one_method"))?;
    let mut left = settle.minor();
    let mut out = Vec::with_capacity(given.len());
    for (m, a) in given {
        if *m == method && left > 0 {
            let take = a.minor().min(left);
            left -= take;
            out.push((*m, Money::from_minor(a.minor() - take, a.currency())));
        } else {
            out.push((*m, *a));
        }
    }
    let paid: i64 = out
        .iter()
        .filter(|(m, _)| *m != Method::Credit)
        .map(|(_, a)| a.minor())
        .sum();
    if paid < due.minor()
        || out
            .iter()
            .any(|(m, a)| *m == Method::Credit && !a.is_zero())
    {
        return Err(RuleError::on("settle", "sale.settle_needs_full_payment"));
    }
    Ok((out, method))
}

/// In an exchange, the part of what came back that pays the new sale;
/// the rest is given back, or the customer pays what the new sale needs
/// beyond it.
pub fn exchange_used(returned: Money, new_sale: Money) -> Money {
    if returned.minor() <= new_sale.minor() {
        returned
    } else {
        new_sale
    }
}

/// What a sold line came to (amounts of the whole line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sold {
    /// Quantity sold, in thousandths.
    pub qty_milli: i64,
    pub net: i64,
    pub tax: i64,
    /// Cost of one base unit.
    pub unit_cost: i64,
}

/// What a return gives back for a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Refund {
    pub qty_milli: i64,
    pub net: i64,
    pub tax: i64,
    pub cost: i64,
}

impl Refund {
    pub fn gross(&self) -> i64 {
        self.net + self.tax
    }
}

/// A return of `qty_milli` that the line no longer has: two devices took
/// the same goods back while apart, and the money already went. Its share
/// of the whole sale, as the device that gave it computed it alone.
pub fn refund_again(sold: Sold, qty_milli: i64) -> Result<Refund, RuleError> {
    if qty_milli <= 0 || sold.qty_milli <= 0 {
        return Err(RuleError::on("qty", "sale.qty_positive"));
    }
    let share = |total: i64| -> i64 {
        let part = i128::from(total) * i128::from(qty_milli.min(sold.qty_milli))
            / i128::from(sold.qty_milli);
        i64::try_from(part).unwrap_or(0)
    };
    Ok(Refund {
        qty_milli,
        net: share(sold.net),
        tax: share(sold.tax),
        cost: unit_cost(sold.unit_cost, qty_milli),
    })
}

/// Returning `qty_milli` more of a line already returned by `before`: in
/// proportion to what was sold, and the exact remainder when the last of
/// it comes back — so the returns of a line never exceed its sale.
pub fn refund(sold: Sold, before: Refund, qty_milli: i64) -> Result<Refund, RuleError> {
    if qty_milli <= 0 {
        return Err(RuleError::on("qty", "sale.qty_positive"));
    }
    let left = sold.qty_milli - before.qty_milli;
    if qty_milli > left {
        return Err(RuleError::on("qty", "return.too_much"));
    }
    if qty_milli == left {
        return Ok(Refund {
            qty_milli,
            net: sold.net - before.net,
            tax: sold.tax - before.tax,
            cost: unit_cost(sold.unit_cost, qty_milli),
        });
    }
    // Shares of the whole, minus what already went back (no drift).
    let share = |total: i64| -> i64 {
        let upto = i128::from(total) * i128::from(before.qty_milli + qty_milli)
            / i128::from(sold.qty_milli);
        i64::try_from(upto).unwrap_or(0)
    };
    let net = share(sold.net) - before.net;
    let tax = share(sold.tax) - before.tax;
    Ok(Refund {
        qty_milli,
        net,
        tax,
        cost: unit_cost(sold.unit_cost, qty_milli),
    })
}

fn unit_cost(unit: i64, qty_milli: i64) -> i64 {
    let v = (i128::from(unit) * i128::from(qty_milli) + 500) / 1_000;
    i64::try_from(v).unwrap_or(0)
}

#[cfg(test)]
mod tests {

    #[test]
    fn cash_beyond_the_sale_pays_old_debt_then_comes_back() {
        // The screenshot: 297 500 due, 1 224 943.58 owed, all of it handed over.
        assert_eq!(
            bon_par_bon(29_750_000, 152_244_358, 122_494_358),
            BonParBon {
                settle: 122_494_358,
                change: 0,
                on_account: 0,
                after: 0
            }
        );
        assert_eq!(
            bon_par_bon(1_000, 5_000, 3_000),
            BonParBon {
                settle: 3_000,
                change: 1_000,
                on_account: 0,
                after: 0
            }
        );
        assert_eq!(
            bon_par_bon(1_000, 600, 3_000),
            BonParBon {
                settle: 0,
                change: 0,
                on_account: 400,
                after: 3_400
            }
        );
        // Nothing owed, or an advance: all extra cash is change.
        assert_eq!(bon_par_bon(1_000, 1_500, -200).change, 500);
    }

    #[test]
    fn bon_par_bon_pays_old_debt_from_one_method() {
        let d = |v: i64| Money::from_minor(v, Currency::DZD);
        // The screenshot: 297 500.00 due, 1 224 943.58 owed, all handed over.
        let given = [(Method::Cash, d(152_244_358))];
        let (left, m) = take_settlement(d(29_750_000), &given, d(122_494_358), d(122_494_358))
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(m, Method::Cash);
        assert_eq!(left, [(Method::Cash, d(29_750_000))]);
        let ok = |due, given: &[(Method, Money)], settle, owed| {
            take_settlement(d(due), given, d(settle), d(owed)).map_err(|e| e.code)
        };
        assert_eq!(
            ok(1_000, &[(Method::Cash, d(5_000))], 5_000, 4_000).err(),
            Some("sale.settle_more_than_owed")
        );
        assert_eq!(
            ok(1_000, &[(Method::Cash, d(1_500))], 1_000, 4_000).err(),
            Some("sale.settle_needs_full_payment")
        );
        assert_eq!(
            ok(
                1_000,
                &[(Method::Cash, d(500)), (Method::Card, d(500))],
                800,
                4_000
            )
            .err(),
            Some("sale.settle_one_method")
        );
        assert_eq!(
            ok(
                1_000,
                &[(Method::Cash, d(1_000)), (Method::Credit, d(1_000))],
                1_000,
                4_000
            )
            .err(),
            Some("sale.settle_needs_full_payment")
        );
        // Card pays the debt when cash is short of it.
        let (left, m) = take_settlement(
            d(1_000),
            &[(Method::Cash, d(1_000)), (Method::Card, d(3_000))],
            d(3_000),
            d(3_000),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!((m, left[1].1), (Method::Card, d(0)));
        // Change still comes out of cash.
        let (left, _) = take_settlement(d(1_000), &[(Method::Cash, d(5_000))], d(2_000), d(2_000))
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(left, [(Method::Cash, d(3_000))]);
    }
    use super::*;
    use nb_core::Currency;

    fn da(minor: i64) -> Money {
        Money::from_minor(minor, Currency::DZD)
    }

    #[test]
    fn an_exchange_uses_what_came_back_up_to_the_new_sale() {
        assert_eq!(exchange_used(da(1_000), da(2_500)), da(1_000));
        assert_eq!(exchange_used(da(3_000), da(2_500)), da(2_500));
        assert_eq!(exchange_used(da(0), da(2_500)), da(0));
    }
    fn q(s: &str) -> Quantity {
        Quantity::new(s.parse().unwrap_or_default())
    }
    fn line(qty: &str, price: i64, bps: i64) -> CartLine {
        CartLine {
            qty: q(qty),
            unit_price: da(price),
            min_price: None,
            tax_rate: Rate::from_bps(bps),
            discount: Discount::None,
        }
    }
    fn cart(lines: Vec<CartLine>, discount: Discount) -> Cart {
        Cart {
            mode: TaxMode::Inclusive,
            lines,
            discount,
        }
    }

    #[test]
    fn totals_add_up() -> Result<(), RuleError> {
        // 2 × 119.00 at 19 % and 1.5 kg × 100.00 at 9 %, prices with tax.
        let t = cart(
            vec![line("2", 11_900, 1_900), line("1.5", 10_000, 900)],
            Discount::None,
        )
        .totals()?;
        assert_eq!(t.total, da(23_800 + 15_000));
        assert_eq!(t.tax, da(3_800 + 1_239));
        assert_eq!(t.net.checked_add(t.tax)?, t.total);
        assert_eq!(t.by_rate.len(), 2);
        // Prices without tax: the tax comes on top.
        let mut c = cart(vec![line("2", 10_000, 1_900)], Discount::None);
        c.mode = TaxMode::Exclusive;
        let t = c.totals()?;
        assert_eq!((t.net, t.tax, t.total), (da(20_000), da(3_800), da(23_800)));
        Ok(())
    }

    #[test]
    fn discounts_on_lines_and_on_the_sale() -> Result<(), RuleError> {
        let mut a = line("1", 10_000, 1_900);
        a.discount = Discount::Percent(1_000);
        let mut b = line("3", 3_333, 1_900);
        b.discount = Discount::Amount(da(1));
        // Lines: 90.00 and 99.98; then 10.00 off the sale, shared.
        let t = cart(vec![a, b], Discount::Amount(da(1_000))).totals()?;
        assert_eq!(t.gross_before, da(19_999));
        assert_eq!(t.discounts, da(1_000 + 1 + 1_000));
        assert_eq!(t.total, da(17_998));
        let shares: i64 = t.lines.iter().map(|l| l.sale_discount.minor()).sum();
        assert_eq!(shares, 1_000, "the shares make the whole discount");
        assert_eq!(
            t.lines[0].discount_bps, 1_474,
            "10 % + its 4.74 share of the 10.00"
        );
        // 15 % off the sale.
        let t = cart(vec![line("1", 10_000, 0)], Discount::Percent(1_500)).totals()?;
        assert_eq!(t.total, da(8_500));
        Ok(())
    }

    #[test]
    fn wrong_carts_are_refused() {
        let code = |c: Cart| c.totals().err().map(|e| e.code);
        assert_eq!(code(cart(vec![], Discount::None)), Some("sale.empty"));
        assert_eq!(
            code(cart(vec![line("0", 100, 0)], Discount::None)),
            Some("sale.qty_positive")
        );
        let mut l = line("1", 100, 0);
        l.discount = Discount::Amount(da(101));
        assert_eq!(
            code(cart(vec![l], Discount::None)),
            Some("sale.discount_too_big")
        );
        assert_eq!(
            code(cart(vec![line("1", 100, 0)], Discount::Percent(10_001))),
            Some("sale.discount_invalid")
        );
    }

    #[test]
    fn floor_prices_are_flagged() -> Result<(), RuleError> {
        let mut l = line("2", 10_000, 0);
        l.min_price = Some(da(9_500));
        l.discount = Discount::Percent(600);
        let t = cart(vec![l.clone()], Discount::None).totals()?;
        assert!(t.lines[0].below_min, "188.00 < 2 × 95.00");
        l.discount = Discount::Percent(500);
        assert!(!cart(vec![l], Discount::None).totals()?.lines[0].below_min);
        Ok(())
    }

    #[test]
    fn settling_cash_card_and_credit() -> Result<(), RuleError> {
        let due = da(17_998);
        // Cash with change.
        let s = settle(due, &[(Method::Cash, da(20_000))], false)?;
        assert_eq!((s.cash, s.change, s.credit), (due, da(2_002), da(0)));
        // Card for part, cash for the rest.
        let s = settle(
            due,
            &[(Method::Card, da(10_000)), (Method::Cash, da(8_000))],
            false,
        )?;
        assert_eq!((s.card, s.cash, s.change), (da(10_000), da(7_998), da(2)));
        // Short: on the customer's account, or refused without one.
        let s = settle(due, &[(Method::Cash, da(5_000))], true)?;
        assert_eq!((s.cash, s.credit), (da(5_000), da(12_998)));
        let code = |r: Result<Settlement, RuleError>| r.err().map(|e| e.code);
        assert_eq!(
            code(settle(due, &[(Method::Cash, da(5_000))], false)),
            Some("sale.underpaid")
        );
        assert_eq!(
            code(settle(due, &[(Method::Card, da(20_000))], false)),
            Some("sale.overpaid")
        );
        assert_eq!(
            code(settle(due, &[(Method::Credit, da(17_998))], false)),
            Some("sale.credit_needs_customer")
        );
        // Every settlement adds up to what was due.
        for given in [
            vec![(Method::Transfer, due)],
            vec![(Method::Cash, da(1)), (Method::Credit, da(17_000))],
        ] {
            let s = settle(due, &given, true)?;
            assert_eq!(
                s.cash.minor() + s.card.minor() + s.transfer.minor() + s.credit.minor(),
                due.minor()
            );
        }
        Ok(())
    }

    #[test]
    fn returns_never_exceed_the_sale() -> Result<(), RuleError> {
        // 3 sold for 100.00 net + 19.00 tax: returned one by one.
        let sold = Sold {
            qty_milli: 3_000,
            net: 10_000,
            tax: 1_900,
            unit_cost: 2_000,
        };
        let a = refund(sold, Refund::default(), 1_000)?;
        assert_eq!((a.net, a.tax, a.cost), (3_333, 633, 2_000));
        let b = refund(sold, a, 1_000)?;
        let both = Refund {
            qty_milli: 2_000,
            net: a.net + b.net,
            tax: a.tax + b.tax,
            cost: 0,
        };
        assert_eq!((b.net, b.tax), (3_333, 633));
        let c = refund(sold, both, 1_000)?;
        assert_eq!(
            (c.net, c.tax),
            (3_334, 634),
            "the last one takes the remainder"
        );
        assert_eq!(a.gross() + b.gross() + c.gross(), 11_900);
        let all = Refund {
            qty_milli: 3_000,
            ..both
        };
        assert_eq!(
            refund(sold, all, 1).err().map(|e| e.code),
            Some("return.too_much")
        );
        assert_eq!(
            refund(sold, Refund::default(), 0).err().map(|e| e.code),
            Some("sale.qty_positive")
        );
        Ok(())
    }
}

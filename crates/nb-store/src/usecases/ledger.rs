//! Accounting use cases (M7): manual entries and reversals. Posted entries
//! are never edited (the database refuses it); a correction is a reversal.

use nb_core::Money;
use nb_domain::ledger::{Entry, Line};
use nb_domain::numbering::DocKind;
use nb_domain::{EntryId, RuleError};
use serde::{Deserialize, Serialize};

use crate::repo::{ledger, org, parse_id};
use crate::time::iso_date;
use crate::uow::OpMeta;
use crate::{Error, Result, Store};
use nb_domain::access::{Permission, Target};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryLineInput {
    pub account: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostedEntry {
    pub id: String,
    pub number: String,
}

fn branch_of(store: &Store) -> Result<(nb_core::Currency, String, String, i32)> {
    store.read(|r| {
        let o = org::the_org(r.conn())?.ok_or(Error::NotFound("organization"))?;
        let b = org::branches(r.conn(), o.org_id()?)?
            .into_iter()
            .next()
            .ok_or(Error::NotFound("branch"))?;
        Ok((o.currency()?, b.id, b.code, r.utc_offset_min))
    })
}

pub fn post_entry(
    store: &mut Store,
    meta: OpMeta,
    memo: &str,
    lines: Vec<EntryLineInput>,
) -> Result<PostedEntry> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::LedgerPostEntry {
            memo: memo.to_owned(),
            lines: lines.clone(),
        },
    )?;
    if memo.trim().is_empty() {
        return Err(RuleError::on("memo", "required").into());
    }
    let (currency, branch_id, branch_code, offset) = branch_of(store)?;
    let entry = Entry::new(
        lines
            .iter()
            .map(|l| Line {
                account: l.account.trim().to_owned(),
                debit: Money::from_minor(l.debit_minor, currency),
                credit: Money::from_minor(l.credit_minor, currency),
                memo: None,
            })
            .collect(),
    )?;
    store.write(meta, |w| {
        w.require(Permission::AccountingPost, &Target::org())?;
        let org = w.org()?;
        let number = w.next_number(DocKind::Journal, &branch_id, &branch_code)?;
        let date = iso_date(w.now(), offset);
        let id = ledger::post(
            w,
            org,
            ledger::Header {
                number: &number,
                date: &date,
                memo: memo.trim(),
                source: "manual",
                reverses: None,
                party: None,
            },
            &entry,
        )?;
        let out = PostedEntry {
            id: id.to_string(),
            number,
        };
        w.event("journal.posted", "journal_entry", &out.id, &out)?;
        w.audit("post", "journal_entry", &out.id, &lines)?;
        Ok(out)
    })
}

pub fn reverse_entry(
    store: &mut Store,
    meta: OpMeta,
    id: &str,
    reason: &str,
) -> Result<PostedEntry> {
    #[allow(clippy::clone_on_copy)]
    store.record(
        meta,
        &crate::ops::Call::LedgerReverseEntry {
            id: id.to_owned(),
            reason: reason.to_owned(),
        },
    )?;
    if reason.trim().is_empty() {
        return Err(RuleError::on("reason", "required").into());
    }
    let entry_id: EntryId = parse_id(id)?;
    let (currency, branch_id, branch_code, offset) = branch_of(store)?;
    store.write(meta, |w| {
        w.require(Permission::AccountingReverse, &Target::org())?;
        if ledger::is_reversed(w.tx(), entry_id)? {
            return Err(RuleError::new("ledger.already_reversed").into());
        }
        let original = ledger::load(w.tx(), entry_id, currency)?;
        // The reversal moves the same customer's debt back.
        let party = ledger::party_of(w.tx(), entry_id)?;
        let org = w.org()?;
        let number = w.next_number(DocKind::Journal, &branch_id, &branch_code)?;
        let date = iso_date(w.now(), offset);
        let header = ledger::Header {
            number: &number,
            date: &date,
            memo: reason.trim(),
            source: "reversal",
            reverses: Some(entry_id),
            party: party.as_deref(),
        };
        let rid = ledger::post(w, org, header, &original.reversal())?;
        let out = PostedEntry {
            id: rid.to_string(),
            number,
        };
        w.event("journal.reversed", "journal_entry", id, &out)?;
        w.audit(
            "reverse",
            "journal_entry",
            id,
            &serde_json::json!({ "reversal": out, "reason": reason }),
        )?;
        Ok(out)
    })
}

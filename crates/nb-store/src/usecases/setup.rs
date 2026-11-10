//! First setup (M4.1/M4.2): the organization, its first branch and
//! warehouse, the owner, and a default chart of accounts.

use nb_core::{Currency, OrganizationId};
use nb_domain::{BranchId, RuleError, UserId, WarehouseId};
use serde::{Deserialize, Serialize};

use crate::repo::ledger::{Account, insert_account};
use crate::repo::org::{self, Branch, Organization, Warehouse};
use crate::uow::OpMeta;
use crate::{Error, Result, Store};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupInput {
    pub org_name: String,
    /// Activity profile code (`grocery`, `wholesale`, `distribution`…).
    pub activity: String,
    /// ISO 3166 country code.
    pub country: String,
    pub currency: String,
    pub currency_exp: u8,
    pub branch_name: String,
    pub owner_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupResult {
    pub org: OrganizationId,
    pub branch: BranchId,
    pub warehouse: WarehouseId,
    pub owner: UserId,
}

/// The default chart of accounts for Algeria (SCF classes), a proposal to
/// be reviewed by an accountant before real use (gate G2).
pub const DZ_ACCOUNTS: &[(&str, &str, &str)] = &[
    ("101", "Capital", "equity"),
    ("30", "Stocks de marchandises", "asset"),
    ("401", "Fournisseurs", "liability"),
    ("411", "Clients", "asset"),
    ("4456", "TVA déductible", "asset"),
    ("4457", "TVA collectée", "liability"),
    ("447", "Autres impôts et taxes (timbre)", "liability"),
    ("512", "Banque", "asset"),
    ("53", "Caisse", "asset"),
    ("58", "Virements internes", "asset"),
    ("600", "Achats de marchandises vendues", "expense"),
    ("603", "Variation des stocks de marchandises", "expense"),
    ("61", "Services extérieurs", "expense"),
    ("62", "Autres services extérieurs", "expense"),
    ("658", "Autres charges (pertes sur stock)", "expense"),
    ("700", "Ventes de marchandises", "income"),
    ("709", "Rabais, remises et ristournes accordés", "income"),
    ("758", "Autres produits (gains sur stock)", "income"),
];

pub fn setup_organization(store: &mut Store, input: SetupInput) -> Result<SetupResult> {
    let mut errors = Vec::new();
    if input.org_name.trim().is_empty() {
        errors.push(RuleError::on("org_name", "required"));
    }
    if input.owner_name.trim().is_empty() {
        errors.push(RuleError::on("owner_name", "required"));
    }
    if input.country.len() != 2 {
        errors.push(RuleError::on("country", "setup.country"));
    }
    if Currency::new(&input.currency, input.currency_exp).is_err() {
        errors.push(RuleError::on("currency", "setup.currency"));
    }
    if !errors.is_empty() {
        return Err(Error::Rules(errors));
    }
    if store.read(|r| org::the_org(r.conn()))?.is_some() {
        return Err(Error::Duplicate("organization"));
    }
    let org = OrganizationId::new();
    let owner = UserId::new();
    let meta = OpMeta::new("org.setup", Some(org), Some(owner));
    store.write(meta, |w| {
        let o = Organization {
            id: org.to_string(),
            name: input.org_name.trim().to_owned(),
            country: input.country.to_uppercase(),
            currency: input.currency.clone(),
            currency_exp: input.currency_exp,
            activity: input.activity.clone(),
        };
        org::insert_org(w, &o)?;
        let branch = BranchId::new();
        org::insert_branch(
            w,
            org,
            &Branch {
                id: branch.to_string(),
                code: "01".into(),
                name: input.branch_name.trim().to_owned(),
            },
        )?;
        let warehouse = WarehouseId::new();
        org::insert_warehouse(
            w,
            org,
            &Warehouse {
                id: warehouse.to_string(),
                branch_id: branch.to_string(),
                name: input.branch_name.trim().to_owned(),
                negative_stock: "permission".into(),
            },
        )?;
        org::insert_user(w, org, owner, input.owner_name.trim(), "owner")?;
        if o.country == "DZ" {
            for (code, name, kind) in DZ_ACCOUNTS {
                insert_account(
                    w,
                    org,
                    &Account {
                        code: (*code).into(),
                        name: (*name).into(),
                        parent_code: None,
                        kind: (*kind).into(),
                    },
                )?;
            }
        }
        w.event("org.created", "organization", &o.id, &o)?;
        w.audit("create", "organization", &o.id, &o)?;
        Ok(SetupResult {
            org,
            branch,
            warehouse,
            owner,
        })
    })
}

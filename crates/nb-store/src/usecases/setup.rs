//! First setup (M4.1/M4.2): the organization, its first branch and
//! warehouse, the owner, and a default chart of accounts.

use nb_core::{Currency, OrganizationId};
use nb_domain::{BranchId, RuleError, UserId, WarehouseId};
use serde::{Deserialize, Serialize};

use crate::repo::access;
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
    /// Default language (`ar`, `fr`, `en`).
    #[serde(default = "default_language")]
    pub language: String,
    /// First month of the fiscal year.
    #[serde(default = "default_fiscal")]
    pub fiscal_year_start: u8,
}

fn default_language() -> String {
    "ar".into()
}

fn default_fiscal() -> u8 {
    1
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
/// `(code, name, parent, kind, group)`; groups total their accounts and
/// take no entry.
pub type ChartRow = (
    &'static str,
    &'static str,
    Option<&'static str>,
    &'static str,
    bool,
);

pub const DZ_CHART: &[ChartRow] = &[
    ("1", "Comptes de capitaux", None, "equity", true),
    (
        "10",
        "Capital, réserves et assimilés",
        Some("1"),
        "equity",
        true,
    ),
    ("101", "Capital", Some("10"), "equity", false),
    ("12", "Résultat de l'exercice", Some("1"), "equity", false),
    ("3", "Comptes de stocks et en-cours", None, "asset", true),
    ("30", "Stocks de marchandises", Some("3"), "asset", false),
    ("4", "Comptes de tiers", None, "liability", true),
    (
        "40",
        "Fournisseurs et comptes rattachés",
        Some("4"),
        "liability",
        true,
    ),
    ("401", "Fournisseurs", Some("40"), "liability", false),
    (
        "41",
        "Clients et comptes rattachés",
        Some("4"),
        "asset",
        true,
    ),
    ("411", "Clients", Some("41"), "asset", false),
    (
        "44",
        "État, collectivités publiques",
        Some("4"),
        "liability",
        true,
    ),
    (
        "445",
        "État, taxes sur le chiffre d'affaires",
        Some("44"),
        "liability",
        true,
    ),
    ("4456", "TVA déductible", Some("445"), "asset", false),
    ("4457", "TVA collectée", Some("445"), "liability", false),
    (
        "447",
        "Autres impôts et taxes (timbre)",
        Some("44"),
        "liability",
        false,
    ),
    ("5", "Comptes financiers", None, "asset", true),
    (
        "51",
        "Banques, établissements financiers",
        Some("5"),
        "asset",
        true,
    ),
    ("512", "Banque", Some("51"), "asset", true),
    ("5121", "Banque principale", Some("512"), "asset", false),
    ("53", "Caisse", Some("5"), "asset", true),
    ("531", "Caisse principale", Some("53"), "asset", false),
    ("58", "Virements internes", Some("5"), "asset", false),
    ("6", "Comptes de charges", None, "expense", true),
    ("60", "Achats consommés", Some("6"), "expense", true),
    (
        "600",
        "Achats de marchandises vendues",
        Some("60"),
        "expense",
        false,
    ),
    (
        "603",
        "Variation des stocks de marchandises",
        Some("60"),
        "expense",
        false,
    ),
    ("61", "Services extérieurs", Some("6"), "expense", true),
    (
        "62",
        "Autres services extérieurs",
        Some("6"),
        "expense",
        true,
    ),
    (
        "607",
        "Achats non stockés (eau, énergie)",
        Some("60"),
        "expense",
        false,
    ),
    ("613", "Locations", Some("61"), "expense", false),
    (
        "615",
        "Entretien et réparations",
        Some("61"),
        "expense",
        false,
    ),
    ("616", "Primes d'assurance", Some("61"), "expense", false),
    ("624", "Transports", Some("62"), "expense", false),
    (
        "626",
        "Frais postaux et de télécommunications",
        Some("62"),
        "expense",
        false,
    ),
    ("627", "Services bancaires", Some("62"), "expense", false),
    ("63", "Charges de personnel", Some("6"), "expense", true),
    (
        "631",
        "Rémunérations du personnel",
        Some("63"),
        "expense",
        false,
    ),
    (
        "64",
        "Impôts, taxes et versements assimilés",
        Some("6"),
        "expense",
        true,
    ),
    (
        "642",
        "Impôts et taxes non récupérables",
        Some("64"),
        "expense",
        false,
    ),
    (
        "65",
        "Autres charges opérationnelles",
        Some("6"),
        "expense",
        true,
    ),
    (
        "654",
        "Pertes sur créances irrécouvrables",
        Some("65"),
        "expense",
        false,
    ),
    (
        "658",
        "Autres charges (pertes sur stock)",
        Some("65"),
        "expense",
        false,
    ),
    ("7", "Comptes de produits", None, "income", true),
    (
        "70",
        "Ventes de marchandises et produits",
        Some("7"),
        "income",
        true,
    ),
    ("700", "Ventes de marchandises", Some("70"), "income", false),
    (
        "709",
        "Rabais, remises et ristournes accordés",
        Some("70"),
        "income",
        false,
    ),
    (
        "75",
        "Autres produits opérationnels",
        Some("7"),
        "income",
        true,
    ),
    (
        "752",
        "Revenus des immeubles (loyers perçus)",
        Some("75"),
        "income",
        false,
    ),
    (
        "758",
        "Autres produits (gains sur stock)",
        Some("75"),
        "income",
        false,
    ),
];

/// The chart's names in Arabic (SCF), for stores that work in Arabic.
pub const DZ_CHART_AR: &[(&str, &str)] = &[
    ("1", "حسابات رؤوس الأموال"),
    ("10", "رأس المال والاحتياطات وما يماثلها"),
    ("101", "رأس المال"),
    ("12", "نتيجة السنة المالية"),
    ("3", "حسابات المخزونات والمنتجات قيد الصنع"),
    ("30", "مخزونات البضائع"),
    ("4", "حسابات الغير"),
    ("40", "الموردون والحسابات الملحقة"),
    ("401", "الموردون"),
    ("41", "الزبائن والحسابات الملحقة"),
    ("411", "الزبائن"),
    ("44", "الدولة والجماعات العمومية"),
    ("445", "الدولة، الرسوم على رقم الأعمال"),
    ("4456", "الرسم على القيمة المضافة القابل للاسترجاع"),
    ("4457", "الرسم على القيمة المضافة المحصّل"),
    ("447", "ضرائب ورسوم أخرى (الطابع)"),
    ("5", "الحسابات المالية"),
    ("51", "البنوك والمؤسسات المالية"),
    ("512", "البنك"),
    ("5121", "البنك الرئيسي"),
    ("53", "الصندوق"),
    ("531", "الصندوق الرئيسي"),
    ("58", "التحويلات الداخلية"),
    ("6", "حسابات الأعباء"),
    ("60", "المشتريات المستهلكة"),
    ("600", "مشتريات البضائع المبيعة"),
    ("603", "تغيّر مخزونات البضائع"),
    ("61", "الخدمات الخارجية"),
    ("62", "الخدمات الخارجية الأخرى"),
    ("607", "المشتريات غير المخزنة (الماء، الطاقة)"),
    ("613", "الإيجارات"),
    ("615", "الصيانة والتصليحات"),
    ("616", "أقساط التأمين"),
    ("624", "النقل"),
    ("626", "مصاريف البريد والاتصالات"),
    ("627", "الخدمات المصرفية"),
    ("63", "أعباء المستخدمين"),
    ("631", "أجور المستخدمين"),
    ("64", "الضرائب والرسوم والمدفوعات المماثلة"),
    ("642", "الضرائب والرسوم غير القابلة للاسترجاع"),
    ("65", "الأعباء العملياتية الأخرى"),
    ("654", "خسائر الديون غير القابلة للتحصيل"),
    ("658", "أعباء أخرى (خسائر المخزون)"),
    ("7", "حسابات المنتجات"),
    ("70", "مبيعات البضائع والمنتجات"),
    ("700", "مبيعات البضائع"),
    ("709", "التخفيضات والتنزيلات الممنوحة"),
    ("75", "المنتجات العملياتية الأخرى"),
    ("752", "إيرادات العقارات (إيجارات محصّلة)"),
    ("758", "منتجات أخرى (أرباح المخزون)"),
];

/// The expense and income categories a shop starts with: (kind, French,
/// Arabic, account).
pub const DZ_EXPENSE_CATEGORIES: &[(&str, &str, &str, &str)] = &[
    ("expense", "Loyer", "الإيجار", "613"),
    (
        "expense",
        "Électricité, eau et gaz",
        "الكهرباء والماء والغاز",
        "607",
    ),
    ("expense", "Salaires", "الأجور", "631"),
    ("expense", "Transport", "النقل", "624"),
    ("expense", "Téléphone et internet", "الهاتف والإنترنت", "626"),
    (
        "expense",
        "Entretien et réparations",
        "الصيانة والتصليح",
        "615",
    ),
    ("expense", "Frais bancaires", "مصاريف بنكية", "627"),
    ("expense", "Impôts et taxes", "الضرائب والرسوم", "642"),
    ("expense", "Divers", "متفرقات", "658"),
    ("income", "Loyers perçus", "إيجارات محصّلة", "752"),
    ("income", "Autres produits", "إيرادات أخرى", "758"),
];

/// An account's name in the store's language.
fn chart_name(code: &str, french: &'static str, language: &str) -> String {
    match language {
        "ar" => DZ_CHART_AR
            .iter()
            .find(|(c, _)| *c == code)
            .map_or(french, |(_, n)| n)
            .to_owned(),
        _ => french.to_owned(),
    }
}

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
    if let Err(mut e) = crate::usecases::org::check_settings(&crate::usecases::org::OrgSettings {
        name: input.org_name.clone(),
        activity: input.activity.clone(),
        language: input.language.clone(),
        fiscal_year_start: input.fiscal_year_start,
        auto_lock_min: 5,
    }) {
        e.retain(|r| r.field != Some("name"));
        errors.append(&mut e);
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
        w.tx().execute(
            "UPDATE organizations SET language = ?2, fiscal_year_start = ?3 WHERE id = ?1",
            rusqlite::params![o.id, input.language, input.fiscal_year_start],
        )?;
        w.tx().execute(
            "INSERT INTO business_profiles (org_id, legal_name, updated_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![o.id, o.name, w.now()],
        )?;
        crate::usecases::org::first_register(w, org, &branch.to_string(), "01")?;
        // The current fiscal year and its periods.
        let (y, m, d) = w.date();
        crate::repo::fiscal::ensure_period(w, org, &format!("{y:04}-{m:02}-{d:02}"))?;
        crate::usecases::catalog::seed_defaults(w, org, &o.country, &input.language)?;
        org::insert_user(w, org, owner, input.owner_name.trim(), "owner")?;
        access::insert_grant(w, org, owner, &access::GrantRow::org("owner"))?;
        access::ensure_credentials(w, org, owner)?;
        if o.country == "DZ" {
            for (code, name, parent, kind, group) in DZ_CHART {
                insert_account(
                    w,
                    org,
                    &Account {
                        code: (*code).into(),
                        name: chart_name(code, name, &input.language),
                        parent_code: parent.map(str::to_owned),
                        kind: (*kind).into(),
                        is_group: *group,
                        active: true,
                    },
                )?;
            }
            for (role, code) in nb_domain::posting::AccountMap::dz().iter() {
                crate::repo::ledger::set_role(w, org, role, code)?;
            }
            for (kind, name, code) in [
                ("cash", "Caisse principale", "531"),
                ("bank", "Banque principale", "5121"),
            ] {
                let name = chart_name(code, name, &input.language);
                crate::usecases::accounting::insert_money_account(w, org, None, kind, &name, code)?;
            }
            for (kind, fr, ar, code) in DZ_EXPENSE_CATEGORIES {
                w.tx().execute(
                    "INSERT INTO expense_categories (id, org_id, kind, name, account_code, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        nb_core::id::idgen::uuid().to_string(),
                        org.to_string(),
                        kind,
                        if input.language == "ar" { ar } else { fr },
                        code,
                        w.now()
                    ],
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

//! Access control (M4.5–M4.8, PRD §12): a fixed, numbered list of
//! permissions, roles that bundle them, grants of a role over a scope
//! (organization, branch, warehouse, cash register), and the decision for a
//! request: allowed, allowed with a supervisor's approval (sensitive
//! actions), or denied.
//!
//! Permission numbers and codes are stable: they are stored in custom roles
//! and in audit records. New permissions get new numbers; none is reused.

use std::collections::BTreeSet;

use crate::{BranchId, RegisterId, RuleError, WarehouseId};

macro_rules! permissions {
    ($( $num:literal $name:ident $code:literal $sensitive:literal ),* $(,)?) => {
        /// What a user may do. Grouped by module (hundreds).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Permission { $( $name ),* }

        impl Permission {
            /// Every permission, in number order.
            pub const ALL: &'static [Permission] = &[ $( Permission::$name ),* ];

            /// Stable number (never reused).
            pub const fn number(self) -> u16 {
                match self { $( Permission::$name => $num ),* }
            }
            /// Stable code (`sales.sell`), also the message key `perm.<code>`.
            pub const fn code(self) -> &'static str {
                match self { $( Permission::$name => $code ),* }
            }
            /// Sensitive actions (PRD §12.2): a supervisor may approve them
            /// for a user who lacks them, and every use is audited.
            pub const fn is_sensitive(self) -> bool {
                match self { $( Permission::$name => $sensitive ),* }
            }
            pub fn from_code(code: &str) -> Option<Permission> {
                match code { $( $code => Some(Permission::$name), )* _ => None }
            }
        }
    };
}

permissions! {
    // Sales and point of sale
    101 SalesSell "sales.sell" false,
    102 SalesDiscountLine "sales.discount" false,
    103 SalesDiscountExceptional "sales.discount_exceptional" true,
    104 SalesPriceOverride "sales.price_override" true,
    105 SalesBelowMinPrice "sales.below_min_price" true,
    106 SalesDeleteDraft "sales.delete_draft" true,
    107 SalesReturn "sales.return" true,
    108 SalesViewAll "sales.view_all" false,
    // Cash registers
    201 CashOpenShift "cash.open_shift" false,
    202 CashCloseShift "cash.close_shift" false,
    203 CashMovement "cash.movement" false,
    204 CashOpenDrawer "cash.open_drawer" true,
    // Catalog
    301 ProductsView "products.view" false,
    302 ProductsEdit "products.edit" false,
    303 ProductsPrice "products.price" true,
    304 ProductsArchive "products.archive" false,
    305 ProductsViewCost "products.view_cost" true,
    // Inventory
    401 StockView "stock.view" false,
    402 StockReceive "stock.receive" false,
    403 StockIssue "stock.issue" false,
    404 StockTransfer "stock.transfer" false,
    405 StockCount "stock.count" false,
    406 StockAdjust "stock.adjust" true,
    407 StockNegative "stock.negative" true,
    // Customers, credit and debts
    501 CustomersView "customers.view" false,
    502 CustomersEdit "customers.edit" false,
    503 DebtsCollect "debts.collect" false,
    504 CreditExceed "credit.exceed" true,
    505 CreditLimits "credit.limits" true,
    506 InstallmentsReschedule "installments.reschedule" true,
    // Purchasing
    601 SuppliersEdit "suppliers.edit" false,
    602 PurchasesOrder "purchases.order" false,
    603 PurchasesApprove "purchases.approve" true,
    604 PurchasesReceive "purchases.receive" false,
    // Accounting
    701 AccountingView "accounting.view" false,
    702 AccountingPost "accounting.post" true,
    703 AccountingReverse "accounting.reverse" true,
    704 AccountingClosePeriod "accounting.close_period" true,
    // Reports
    801 ReportsView "reports.view" false,
    802 ReportsProfit "reports.profit" true,
    // Administration
    901 SettingsEdit "settings.edit" false,
    902 UsersManage "users.manage" true,
    903 RolesManage "roles.manage" true,
    904 AuditView "audit.view" false,
    905 DevicesManage "devices.manage" false,
    906 BackupManage "backup.manage" false,
}

impl Permission {
    /// The module of a permission (`sales`, `stock`…), for grouping in screens.
    pub fn module(self) -> &'static str {
        let code = self.code();
        code.split_once('.').map_or(code, |(m, _)| m)
    }
}

/// A set of permissions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSet(BTreeSet<Permission>);

impl PermissionSet {
    pub fn all() -> Self {
        Self(Permission::ALL.iter().copied().collect())
    }
    pub fn of(perms: &[Permission]) -> Self {
        Self(perms.iter().copied().collect())
    }
    pub fn contains(&self, p: Permission) -> bool {
        self.0.contains(&p)
    }
    pub fn insert(&mut self, p: Permission) {
        self.0.insert(p);
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = Permission> + '_ {
        self.0.iter().copied()
    }
    /// Codes, in number order (how custom roles are stored).
    pub fn codes(&self) -> Vec<&'static str> {
        self.0.iter().map(|p| p.code()).collect()
    }
    /// Reads stored codes; unknown codes (from a newer version) are an error,
    /// so that a role is never silently narrowed or widened.
    pub fn from_codes<S: AsRef<str>>(codes: &[S]) -> Result<Self, RuleError> {
        codes
            .iter()
            .map(|c| {
                Permission::from_code(c.as_ref()).ok_or(RuleError::new("access.unknown_permission"))
            })
            .collect::<Result<BTreeSet<_>, _>>()
            .map(Self)
    }
}

impl FromIterator<Permission> for PermissionSet {
    fn from_iter<I: IntoIterator<Item = Permission>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// The roles of PRD §4.2 that come with the application. Their permissions
/// follow the application's version; organizations add custom roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinRole {
    Owner,
    GeneralManager,
    BranchManager,
    Accountant,
    SalesManager,
    Cashier,
    StockKeeper,
    Purchasing,
    CreditOfficer,
    SalesRep,
    Driver,
    Auditor,
}

impl BuiltinRole {
    pub const ALL: [BuiltinRole; 12] = [
        BuiltinRole::Owner,
        BuiltinRole::GeneralManager,
        BuiltinRole::BranchManager,
        BuiltinRole::Accountant,
        BuiltinRole::SalesManager,
        BuiltinRole::Cashier,
        BuiltinRole::StockKeeper,
        BuiltinRole::Purchasing,
        BuiltinRole::CreditOfficer,
        BuiltinRole::SalesRep,
        BuiltinRole::Driver,
        BuiltinRole::Auditor,
    ];

    /// Stable code, also the message key `role.<code>`.
    pub const fn code(self) -> &'static str {
        match self {
            BuiltinRole::Owner => "owner",
            BuiltinRole::GeneralManager => "general_manager",
            BuiltinRole::BranchManager => "branch_manager",
            BuiltinRole::Accountant => "accountant",
            BuiltinRole::SalesManager => "sales_manager",
            BuiltinRole::Cashier => "cashier",
            BuiltinRole::StockKeeper => "stock_keeper",
            BuiltinRole::Purchasing => "purchasing",
            BuiltinRole::CreditOfficer => "credit_officer",
            BuiltinRole::SalesRep => "sales_rep",
            BuiltinRole::Driver => "driver",
            BuiltinRole::Auditor => "auditor",
        }
    }

    pub fn from_code(code: &str) -> Option<BuiltinRole> {
        Self::ALL.into_iter().find(|r| r.code() == code)
    }

    pub fn permissions(self) -> PermissionSet {
        use Permission::*;
        let views = [
            ProductsView,
            StockView,
            CustomersView,
            SalesViewAll,
            AccountingView,
            ReportsView,
            AuditView,
        ];
        match self {
            BuiltinRole::Owner => PermissionSet::all(),
            // Everything except changing what roles can do.
            BuiltinRole::GeneralManager => Permission::ALL
                .iter()
                .copied()
                .filter(|p| *p != RolesManage)
                .collect(),
            BuiltinRole::BranchManager => PermissionSet::of(&[
                SalesSell,
                SalesDiscountLine,
                SalesDiscountExceptional,
                SalesPriceOverride,
                SalesBelowMinPrice,
                SalesDeleteDraft,
                SalesReturn,
                SalesViewAll,
                CashOpenShift,
                CashCloseShift,
                CashMovement,
                CashOpenDrawer,
                ProductsView,
                ProductsEdit,
                ProductsPrice,
                ProductsViewCost,
                StockView,
                StockReceive,
                StockIssue,
                StockTransfer,
                StockCount,
                StockAdjust,
                StockNegative,
                CustomersView,
                CustomersEdit,
                DebtsCollect,
                CreditExceed,
                PurchasesOrder,
                PurchasesReceive,
                ReportsView,
                ReportsProfit,
            ]),
            BuiltinRole::Accountant => PermissionSet::of(&[
                AccountingView,
                AccountingPost,
                AccountingReverse,
                AccountingClosePeriod,
                ReportsView,
                ReportsProfit,
                ProductsView,
                ProductsViewCost,
                StockView,
                CustomersView,
                SalesViewAll,
                AuditView,
            ]),
            BuiltinRole::SalesManager => PermissionSet::of(&[
                SalesSell,
                SalesDiscountLine,
                SalesDiscountExceptional,
                SalesPriceOverride,
                SalesReturn,
                SalesViewAll,
                ProductsView,
                StockView,
                CustomersView,
                CustomersEdit,
                DebtsCollect,
                ReportsView,
            ]),
            BuiltinRole::Cashier => PermissionSet::of(&[
                SalesSell,
                SalesDiscountLine,
                CashOpenShift,
                CashCloseShift,
                CashMovement,
                ProductsView,
                StockView,
                CustomersView,
                DebtsCollect,
            ]),
            BuiltinRole::StockKeeper => PermissionSet::of(&[
                ProductsView,
                StockView,
                StockReceive,
                StockIssue,
                StockTransfer,
                StockCount,
                PurchasesReceive,
            ]),
            BuiltinRole::Purchasing => PermissionSet::of(&[
                ProductsView,
                StockView,
                SuppliersEdit,
                PurchasesOrder,
                PurchasesReceive,
                ProductsViewCost,
            ]),
            BuiltinRole::CreditOfficer => PermissionSet::of(&[
                CustomersView,
                CustomersEdit,
                DebtsCollect,
                CreditLimits,
                CreditExceed,
                InstallmentsReschedule,
                ReportsView,
            ]),
            BuiltinRole::SalesRep => PermissionSet::of(&[
                SalesSell,
                ProductsView,
                StockView,
                CustomersView,
                DebtsCollect,
            ]),
            BuiltinRole::Driver => PermissionSet::of(&[CustomersView, DebtsCollect]),
            BuiltinRole::Auditor => PermissionSet::of(&views),
        }
    }
}

/// Where a grant applies (PRD §12.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Org,
    Branch(BranchId),
    Warehouse(WarehouseId),
    Register(RegisterId),
}

impl Scope {
    /// Stored kind: `org`, `branch`, `warehouse`, `register`.
    pub const fn kind(self) -> &'static str {
        match self {
            Scope::Org => "org",
            Scope::Branch(_) => "branch",
            Scope::Warehouse(_) => "warehouse",
            Scope::Register(_) => "register",
        }
    }
}

/// What a request touches. The caller fills the branch of the warehouse or
/// register it names, so a branch grant covers them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Target {
    pub branch: Option<BranchId>,
    pub warehouse: Option<WarehouseId>,
    pub register: Option<RegisterId>,
}

impl Target {
    /// An organization-wide request (settings, users, accounts).
    pub const fn org() -> Self {
        Self {
            branch: None,
            warehouse: None,
            register: None,
        }
    }
    pub const fn branch(b: BranchId) -> Self {
        Self {
            branch: Some(b),
            warehouse: None,
            register: None,
        }
    }
    pub const fn warehouse(b: BranchId, w: WarehouseId) -> Self {
        Self {
            branch: Some(b),
            warehouse: Some(w),
            register: None,
        }
    }
    pub const fn register(b: BranchId, r: RegisterId) -> Self {
        Self {
            branch: Some(b),
            warehouse: None,
            register: Some(r),
        }
    }
}

impl Scope {
    /// Whether a grant over this scope reaches `t`.
    pub fn covers(self, t: &Target) -> bool {
        match self {
            Scope::Org => true,
            Scope::Branch(b) => t.branch == Some(b),
            Scope::Warehouse(w) => t.warehouse == Some(w),
            Scope::Register(r) => t.register == Some(r),
        }
    }
}

/// A role's permissions granted over a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub permissions: PermissionSet,
    pub scope: Scope,
}

/// The answer to a request (PRD §12.1 diagram).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// A sensitive action the user lacks: a supervisor holding it may approve.
    NeedsApproval,
    Deny,
}

/// Decides whether a user holding `grants` may do `p` on `t`.
/// An inactive user is denied everything.
pub fn authorize(grants: &[Grant], active: bool, p: Permission, t: &Target) -> Decision {
    if !active {
        return Decision::Deny;
    }
    if grants
        .iter()
        .any(|g| g.permissions.contains(p) && g.scope.covers(t))
    {
        Decision::Allow
    } else if p.is_sensitive() {
        Decision::NeedsApproval
    } else {
        Decision::Deny
    }
}

/// What a user can do anywhere (to show or hide screens and buttons; the
/// use cases still decide per request).
pub fn reachable(grants: &[Grant]) -> PermissionSet {
    grants.iter().flat_map(|g| g.permissions.iter()).collect()
}

// ---------------------------------------------------------------------------
// Credentials policy (M4.7)

/// Shortest password accepted.
pub const PASSWORD_MIN: usize = 8;
/// Failed attempts before a lock.
pub const MAX_FAILURES: u32 = 5;
/// First lock duration; it doubles with each further failure, up to a day.
pub const LOCK_BASE_MS: i64 = 60_000;
const LOCK_MAX_MS: i64 = 24 * 60 * 60_000;

/// Checks a new password: long enough, not only digits or one character.
pub fn check_password(pw: &str) -> Result<(), RuleError> {
    let n = pw.chars().count();
    if n < PASSWORD_MIN {
        return Err(RuleError::on("password", "access.password_short"));
    }
    let first = pw.chars().next();
    if pw.chars().all(|c| Some(c) == first) || pw.chars().all(|c| c.is_ascii_digit()) {
        return Err(RuleError::on("password", "access.password_weak"));
    }
    Ok(())
}

/// Checks a new PIN: 4 to 6 digits, not trivial (`1111`, `1234`, `9876`).
pub fn check_pin(pin: &str) -> Result<(), RuleError> {
    let digits: Vec<u8> = pin.bytes().collect();
    if !(4..=6).contains(&digits.len()) || !digits.iter().all(u8::is_ascii_digit) {
        return Err(RuleError::on("pin", "access.pin_format"));
    }
    let steps: Vec<i16> = digits
        .windows(2)
        .map(|w| i16::from(w[1]) - i16::from(w[0]))
        .collect();
    if steps.iter().all(|s| *s == 0)
        || steps.iter().all(|s| *s == 1)
        || steps.iter().all(|s| *s == -1)
    {
        return Err(RuleError::on("pin", "access.pin_weak"));
    }
    Ok(())
}

/// After a failure: the new failure count and, past [`MAX_FAILURES`], the
/// time until which sign-in is refused.
pub fn after_failure(failures: u32, now_ms: i64) -> (u32, Option<i64>) {
    let n = failures.saturating_add(1);
    if n < MAX_FAILURES {
        return (n, None);
    }
    let doublings = (n - MAX_FAILURES).min(20);
    let lock = LOCK_BASE_MS
        .saturating_mul(1_i64 << doublings)
        .min(LOCK_MAX_MS);
    (n, Some(now_ms.saturating_add(lock)))
}

/// Whether sign-in is refused at `now_ms`.
pub fn is_locked(locked_until: Option<i64>, now_ms: i64) -> bool {
    locked_until.is_some_and(|t| now_ms < t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_codes_are_unique_and_stable() {
        let mut nums = BTreeSet::new();
        let mut codes = BTreeSet::new();
        for p in Permission::ALL {
            assert!(nums.insert(p.number()), "number {}", p.number());
            assert!(codes.insert(p.code()), "code {}", p.code());
            assert_eq!(Permission::from_code(p.code()), Some(*p));
        }
        // Pinned: changing these breaks stored roles and audit records.
        assert_eq!(Permission::SalesSell.number(), 101);
        assert_eq!(Permission::StockAdjust.code(), "stock.adjust");
        assert_eq!(Permission::RolesManage.number(), 903);
        assert_eq!(Permission::StockAdjust.module(), "stock");
    }

    #[test]
    fn stored_codes_round_trip_and_unknown_codes_are_refused() {
        let set = BuiltinRole::Cashier.permissions();
        let codes = set.codes();
        assert_eq!(PermissionSet::from_codes(&codes), Ok(set));
        assert_eq!(
            PermissionSet::from_codes(&["sales.sell", "sales.teleport"]),
            Err(RuleError::new("access.unknown_permission"))
        );
    }

    #[test]
    fn builtin_roles() {
        assert_eq!(
            BuiltinRole::Owner.permissions().len(),
            Permission::ALL.len()
        );
        assert!(
            !BuiltinRole::GeneralManager
                .permissions()
                .contains(Permission::RolesManage)
        );
        assert!(
            BuiltinRole::GeneralManager
                .permissions()
                .contains(Permission::UsersManage)
        );
        // The auditor sees and never changes.
        for p in BuiltinRole::Auditor.permissions().iter() {
            assert!(
                p.code().ends_with("view") || p.code().ends_with("view_all"),
                "{}",
                p.code()
            );
        }
        assert!(
            !BuiltinRole::Cashier
                .permissions()
                .contains(Permission::StockAdjust)
        );
        for r in BuiltinRole::ALL {
            assert_eq!(BuiltinRole::from_code(r.code()), Some(r));
            assert!(!r.permissions().is_empty());
        }
    }

    /// The matrix of M4.6: permission × scope × target.
    #[test]
    fn authorization_matrix() {
        let b1 = BranchId::new();
        let b2 = BranchId::new();
        let w1 = WarehouseId::new();
        let w2 = WarehouseId::new();
        let r1 = RegisterId::new();
        let r2 = RegisterId::new();
        let targets = [
            ("org", Target::org()),
            ("b1", Target::branch(b1)),
            ("b2", Target::branch(b2)),
            ("w1", Target::warehouse(b1, w1)),
            ("w2", Target::warehouse(b2, w2)),
            ("r1", Target::register(b1, r1)),
            ("r2", Target::register(b2, r2)),
        ];
        let scopes = [
            (
                "org",
                Scope::Org,
                ["org", "b1", "b2", "w1", "w2", "r1", "r2"].as_slice(),
            ),
            ("b1", Scope::Branch(b1), ["b1", "w1", "r1"].as_slice()),
            ("w1", Scope::Warehouse(w1), ["w1"].as_slice()),
            ("r1", Scope::Register(r1), ["r1"].as_slice()),
        ];
        for (sname, scope, reached) in scopes {
            let grants = [Grant {
                permissions: PermissionSet::of(&[Permission::SalesSell, Permission::StockAdjust]),
                scope,
            }];
            for (tname, t) in &targets {
                let inside = reached.contains(tname);
                let want = if inside {
                    Decision::Allow
                } else {
                    Decision::Deny
                };
                assert_eq!(
                    authorize(&grants, true, Permission::SalesSell, t),
                    want,
                    "{sname} → {tname}"
                );
                // A sensitive permission outside the scope can still be approved.
                let want = if inside {
                    Decision::Allow
                } else {
                    Decision::NeedsApproval
                };
                assert_eq!(
                    authorize(&grants, true, Permission::StockAdjust, t),
                    want,
                    "{sname} → {tname}"
                );
                // Never granted: denied, or approval for sensitive ones.
                assert_eq!(
                    authorize(&grants, true, Permission::CashMovement, t),
                    Decision::Deny
                );
                assert_eq!(
                    authorize(&grants, true, Permission::UsersManage, t),
                    Decision::NeedsApproval
                );
                // Inactive users can do nothing.
                assert_eq!(
                    authorize(&grants, false, Permission::SalesSell, t),
                    Decision::Deny
                );
            }
        }
        // Grants add up across scopes.
        let grants = [
            Grant {
                permissions: PermissionSet::of(&[Permission::SalesSell]),
                scope: Scope::Register(r1),
            },
            Grant {
                permissions: PermissionSet::of(&[Permission::StockCount]),
                scope: Scope::Branch(b2),
            },
        ];
        assert_eq!(
            authorize(
                &grants,
                true,
                Permission::SalesSell,
                &Target::register(b1, r1)
            ),
            Decision::Allow
        );
        assert_eq!(
            authorize(
                &grants,
                true,
                Permission::StockCount,
                &Target::warehouse(b2, w2)
            ),
            Decision::Allow
        );
        assert_eq!(
            authorize(
                &grants,
                true,
                Permission::StockCount,
                &Target::warehouse(b1, w1)
            ),
            Decision::Deny
        );
        assert_eq!(
            reachable(&grants),
            PermissionSet::of(&[Permission::SalesSell, Permission::StockCount])
        );
    }

    #[test]
    fn passwords_and_pins() {
        assert!(check_password("s3cret-shop").is_ok());
        assert_eq!(
            check_password("short1"),
            Err(RuleError::on("password", "access.password_short"))
        );
        assert_eq!(
            check_password("12345678"),
            Err(RuleError::on("password", "access.password_weak"))
        );
        assert_eq!(
            check_password("aaaaaaaaa"),
            Err(RuleError::on("password", "access.password_weak"))
        );
        // Non-ASCII passwords count characters, not bytes.
        assert_eq!(
            check_password("كلمةسر"),
            Err(RuleError::on("password", "access.password_short"))
        );
        assert!(check_password("كلمة سر قوية").is_ok());
        for ok in ["2580", "1357", "804163", "1122"] {
            assert!(check_pin(ok).is_ok(), "{ok}");
        }
        for (bad, code) in [
            ("123", "access.pin_format"),
            ("1234567", "access.pin_format"),
            ("12a4", "access.pin_format"),
            ("١٢٣٤", "access.pin_format"),
            ("0000", "access.pin_weak"),
            ("1234", "access.pin_weak"),
            ("987654", "access.pin_weak"),
        ] {
            assert_eq!(check_pin(bad), Err(RuleError::on("pin", code)), "{bad}");
        }
    }

    #[test]
    fn lockout_grows_and_expires() {
        let t = 1_000_000;
        let mut n = 0;
        for _ in 0..MAX_FAILURES - 1 {
            let (next, lock) = after_failure(n, t);
            assert_eq!(lock, None);
            n = next;
        }
        let (n, lock) = after_failure(n, t);
        assert_eq!(lock, Some(t + LOCK_BASE_MS));
        assert!(is_locked(lock, t + LOCK_BASE_MS - 1));
        assert!(!is_locked(lock, t + LOCK_BASE_MS));
        let (_, lock) = after_failure(n, t);
        assert_eq!(lock, Some(t + 2 * LOCK_BASE_MS));
        let (_, lock) = after_failure(1_000, t);
        assert_eq!(lock, Some(t + LOCK_MAX_MS));
    }
}

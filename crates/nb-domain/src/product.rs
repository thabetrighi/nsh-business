//! Products and services (PRD §6.3): what a valid product is. Every field
//! error is reported at once, so a form can show them all.

use nb_core::{Money, Quantity, Rate};

use crate::RuleError;
use crate::barcode;
use crate::units::UnitSet;

/// Goods are stocked; services are sold without stock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductKind {
    Goods,
    Service,
}

impl ProductKind {
    pub fn code(self) -> &'static str {
        match self {
            ProductKind::Goods => "goods",
            ProductKind::Service => "service",
        }
    }
    pub fn from_code(s: &str) -> Option<Self> {
        match s {
            "goods" => Some(ProductKind::Goods),
            "service" => Some(ProductKind::Service),
            _ => None,
        }
    }
}

/// A product as typed in a form or imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductDraft {
    pub name: String,
    pub sku: String,
    pub kind: ProductKind,
    pub units: UnitSet,
    pub barcodes: Vec<String>,
    pub tax_rate: Rate,
    /// Retail price of one base unit.
    pub price: Money,
    /// Lowest allowed selling price, if any.
    pub min_price: Option<Money>,
    /// Alert level, in base units.
    pub min_stock: Option<Quantity>,
}

/// A product that passed [`ProductDraft::validate`]: trimmed, barcodes
/// normalized, no duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidProduct(pub ProductDraft);

pub const NAME_MAX: usize = 200;
pub const SKU_MAX: usize = 32;

impl ProductDraft {
    pub fn validate(mut self) -> Result<ValidProduct, Vec<RuleError>> {
        let mut errors = Vec::new();
        self.name = self.name.split_whitespace().collect::<Vec<_>>().join(" ");
        if self.name.is_empty() {
            errors.push(RuleError::on("name", "required"));
        } else if self.name.chars().count() > NAME_MAX {
            errors.push(RuleError::on("name", "product.name_long"));
        }
        self.sku = self.sku.trim().to_uppercase();
        if self.sku.is_empty() {
            errors.push(RuleError::on("sku", "required"));
        } else if self.sku.len() > SKU_MAX
            || !self
                .sku
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
        {
            errors.push(RuleError::on("sku", "product.sku_chars"));
        }
        let mut codes: Vec<String> = Vec::new();
        for b in &self.barcodes {
            match barcode::normalize(b) {
                Ok(c) if codes.contains(&c) => {
                    errors.push(RuleError::on("barcodes", "barcode.duplicate"))
                }
                Ok(c) => codes.push(c),
                Err(e) => errors.push(e),
            }
        }
        self.barcodes = codes;
        if !(0..=10_000).contains(&self.tax_rate.bps()) {
            errors.push(RuleError::on("tax_rate", "tax.rate_range"));
        }
        if self.price.is_negative() {
            errors.push(RuleError::on("price", "product.price_negative"));
        }
        if let Some(min) = self.min_price {
            if min.is_negative() {
                errors.push(RuleError::on("min_price", "product.price_negative"));
            } else if min.currency() != self.price.currency() {
                errors.push(RuleError::on("min_price", "currency_mismatch"));
            } else if min.minor() > self.price.minor() {
                errors.push(RuleError::on("min_price", "product.min_above_price"));
            }
        }
        if let Some(q) = self.min_stock
            && q.is_negative()
        {
            errors.push(RuleError::on("min_stock", "product.min_stock_negative"));
        }
        if errors.is_empty() {
            Ok(ValidProduct(self))
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nb_core::Currency;

    fn draft() -> Result<ProductDraft, RuleError> {
        Ok(ProductDraft {
            name: "  Lait   entier Soummam 1L ".into(),
            sku: " p-001 ".into(),
            kind: ProductKind::Goods,
            units: UnitSet::base("pc", 0)?,
            barcodes: vec!["6130000123455".into()],
            tax_rate: Rate::from_bps(900),
            price: Money::from_minor(12_000, Currency::DZD),
            min_price: Some(Money::from_minor(11_000, Currency::DZD)),
            min_stock: Some(Quantity::whole(10)),
        })
    }

    #[test]
    fn a_valid_product_is_cleaned() -> Result<(), RuleError> {
        let v = draft()?.validate().map_err(|e| e[0].clone())?;
        assert_eq!(v.0.name, "Lait entier Soummam 1L");
        assert_eq!(v.0.sku, "P-001");
        Ok(())
    }

    #[test]
    fn all_errors_at_once() -> Result<(), RuleError> {
        let mut d = draft()?;
        d.name = "   ".into();
        d.sku = "a b".into();
        d.barcodes = vec!["6130000123454".into(), "X1".into(), "X1".into()];
        d.price = Money::from_minor(-1, Currency::DZD);
        d.min_stock = Some(Quantity::whole(-1));
        let errs = d.validate().err().unwrap_or_default();
        let codes: Vec<_> = errs.iter().map(|e| (e.field, e.code)).collect();
        assert_eq!(
            codes,
            vec![
                (Some("name"), "required"),
                (Some("sku"), "product.sku_chars"),
                (Some("barcode"), "barcode.check_digit"),
                (Some("barcodes"), "barcode.duplicate"),
                (Some("price"), "product.price_negative"),
                (Some("min_price"), "product.min_above_price"),
                (Some("min_stock"), "product.min_stock_negative"),
            ]
        );
        Ok(())
    }

    #[test]
    fn kinds_round_trip() {
        for k in [ProductKind::Goods, ProductKind::Service] {
            assert_eq!(ProductKind::from_code(k.code()), Some(k));
        }
    }
}

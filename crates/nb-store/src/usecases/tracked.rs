//! What a movement says about lots and serial numbers (M6.8, M6.9), for
//! every use case that moves stock: receipts name their lot (with its
//! expiry) or list their serial numbers; issues take a named lot, or the
//! first expiring lots, and list the serial numbers that leave.

use nb_core::Quantity;
use nb_domain::stock::{self as rules, MovementKind};
use nb_domain::{MovementId, ProductId, RuleError, WarehouseId};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::repo::tracking::{self as repo, Tracking};
use crate::uow::Writer;

/// Lot and serial numbers given with a quantity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tracked {
    /// Lot code: required to receive a lot-tracked product; optional to
    /// issue one (first expiring first otherwise).
    #[serde(default)]
    pub lot: Option<String>,
    /// Expiry of a new lot (`YYYY-MM-DD`).
    #[serde(default)]
    pub expiry: Option<String>,
    /// One per unit, for serial-tracked products.
    #[serde(default)]
    pub serials: Vec<String>,
}

impl Tracked {
    fn lot_code(&self) -> Option<String> {
        self.lot
            .as_deref()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_uppercase)
    }
    fn expiry(&self) -> Option<&str> {
        self.expiry
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
    }
}

/// What a movement moved, by lot (signed), for movements that follow it.
pub(crate) type Allocation = Vec<(String, Quantity)>;

/// Records the lots or serial numbers of `movement` (`base` units of
/// `product`, of `kind`, in `wh`). `losses` lets issues take expired lots
/// first (waste, count losses).
#[allow(clippy::too_many_arguments)]
pub(crate) fn record(
    w: &mut Writer<'_>,
    movement: MovementId,
    product: ProductId,
    wh: WarehouseId,
    kind: MovementKind,
    base: Quantity,
    t: &Tracked,
    losses: bool,
) -> Result<Allocation> {
    let incoming = kind.sign() > 0;
    match repo::tracking(w.tx(), product)? {
        Tracking::None => {
            if t.lot_code().is_some() || !t.serials.is_empty() {
                return Err(RuleError::on("lot", "tracking.not_tracked").into());
            }
            Ok(vec![])
        }
        Tracking::Lot => {
            if incoming {
                let code = t
                    .lot_code()
                    .ok_or(crate::Error::from(RuleError::on("lot", "lot.required")))?;
                if let Some(e) = t.expiry() {
                    rules::check_expiry(e)?;
                }
                let lot = repo::ensure_lot(w, w.org()?, product, &code, t.expiry())?;
                repo::move_lot(w, movement, &lot, wh, base)?;
                Ok(vec![(lot, base)])
            } else {
                let available = repo::lot_stock(w.tx(), product, wh)?;
                let take = match t.lot_code() {
                    Some(code) => {
                        let lot = repo::lot_by_code(w.tx(), product, &code)?
                            .ok_or(crate::Error::from(RuleError::on("lot", "lot.unknown")))?;
                        let have = available
                            .iter()
                            .find(|l| l.lot == lot)
                            .map_or(Quantity::whole(0), |l| l.qty);
                        if have.decimal() < base.decimal() {
                            return Err(RuleError::on("lot", "lot.insufficient").into());
                        }
                        vec![(lot, base)]
                    }
                    None => {
                        let (y, m, d) = w.date();
                        let today = format!("{y:04}-{m:02}-{d:02}");
                        rules::fefo(&available, base, &today, losses)?
                    }
                };
                for (lot, q) in &take {
                    repo::move_lot(w, movement, lot, wh, Quantity::new(-q.decimal()))?;
                }
                Ok(take)
            }
        }
        Tracking::Serial => {
            let serials = rules::check_serials(base, &t.serials)?;
            let org = w.org()?;
            for s in &serials {
                if incoming {
                    repo::serial_in(w, org, product, wh, movement, s)?;
                } else {
                    repo::serial_out(w, product, wh, movement, s)?;
                }
            }
            Ok(vec![])
        }
    }
}

/// Brings into `wh` the lots and serials another movement took out
/// (transfers): `received` of them, the first lots first.
pub(crate) fn mirror(
    w: &mut Writer<'_>,
    from_movement: &str,
    movement: MovementId,
    product: ProductId,
    wh: WarehouseId,
    received: Quantity,
) -> Result<()> {
    let mut left = received.decimal();
    for (lot, q) in repo::lots_of_movement(w.tx(), from_movement)? {
        if left.is_zero() {
            break;
        }
        let take = left.min(-q.decimal());
        repo::move_lot(w, movement, &lot, wh, Quantity::new(take))?;
        left -= take;
    }
    let serials = repo::serials_of_movement(w.tx(), from_movement)?;
    if !serials.is_empty() {
        if Some(serials.len())
            != received
                .decimal()
                .normalize()
                .to_string()
                .parse::<usize>()
                .ok()
        {
            return Err(RuleError::on("received", "transfer.serial_partial").into());
        }
        let org = w.org()?;
        for s in serials {
            repo::serial_in(w, org, product, wh, movement, &s)?;
        }
    }
    Ok(())
}

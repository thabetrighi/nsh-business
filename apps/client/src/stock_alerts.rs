//! A sale that leaves a product at its reorder level (or out of it) tells
//! the shop through the system's notifications, once per product until it
//! is restocked; pressing the notification opens the stock. A setting of
//! this device (settings › devices); where the system shows none, or the
//! user refused them, nothing shows.

use std::cell::RefCell;
use std::collections::HashSet;

use nb_store::usecases::reports::running_low;
use nsh_core::notifications::{Notification, NotificationPermission};
use nsh_core::prelude::*;

use crate::nav::Route;
use crate::state::AppState;

thread_local! {
    /// The products already told about: not again until they are restocked.
    static TOLD: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// After a sale: tells about those of `products` (what it took) that run
/// out.
pub fn after_sale(st: AppState, products: Vec<String>) {
    if !st.biz.stock_alerts.get_untracked() || products.is_empty() {
        return;
    }
    let db = st.db();
    st.root.run(|| {
        let task = spawn(async move {
            let ids = products.clone();
            let low = match db.call(move |s| running_low(s, &ids)).await {
                Ok(low) => low,
                Err(e) => {
                    log::warn!("stock alerts: {}", e.message);
                    return;
                }
            };
            // Restocked since: may be told about again.
            TOLD.with(|t| {
                let mut t = t.borrow_mut();
                for p in &products {
                    if !low.iter().any(|r| &r.product_id == p) {
                        t.remove(p);
                    }
                }
            });
            let new: Vec<_> = low
                .into_iter()
                .filter(|r| TOLD.with(|t| !t.borrow().contains(&r.product_id)))
                .collect();
            if new.is_empty() || !allowed().await {
                return;
            }
            for r in new {
                let qty = st
                    .fmt()
                    .quantity(nb_core::Quantity::from_milli(r.on_hand_milli), 3);
                let min = st.fmt().quantity(
                    nb_core::Quantity::from_milli(r.min_stock_milli.unwrap_or(0)),
                    3,
                );
                let body = st
                    .tn("stock.alert.body")
                    .replace("{name}", &r.name)
                    .replace("{qty}", &qty)
                    .replace("{min}", &min);
                let shown = show_notification(Notification::new(
                    format!("stock-{}", r.product_id),
                    st.tn("stock.alert.title"),
                    body,
                ))
                .await;
                if shown {
                    TOLD.with(|t| t.borrow_mut().insert(r.product_id.clone()));
                }
            }
        });
        drop(task); // runs to completion under the root
    });
}

/// Whether notifications show, asking the user the first time.
async fn allowed() -> bool {
    match notification_permission().await {
        NotificationPermission::Granted => true,
        NotificationPermission::Default => {
            request_notification_permission().await == NotificationPermission::Granted
        }
        _ => false,
    }
}

/// A notification pressed opens the stock, from the root.
pub fn listen(st: AppState) {
    st.root.run(|| {
        let task = spawn(async move {
            loop {
                let id = next_notification_press().await;
                if id.starts_with("stock-") {
                    st.route.set(Route::Inventory);
                }
            }
        });
        drop(task); // listens while the root lives
    });
}

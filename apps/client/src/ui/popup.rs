//! Dropdowns and menus over nsh-base's `select` and `menu`, in the design
//! system's look: a bordered trigger with a chevron, a floating list with a
//! soft shadow (a sheet on touch screens).

use std::rc::Rc;

use nsh_base::{SelectItem, select};
use nsh_core::element::{SharedString, View};
use nsh_core::prelude::*;

use super::icon::{Icon, icon};
use super::kit::label_line;
use super::tokens::{Tone, control, radius, ty};

const POPUP: &str = "bg-popover border border-border rounded-lg shadow-lg p-1 min-w-[180px]";
const ITEM: &str = "flex-row items-center gap-2 h-9 px-2.5 rounded-md text-foreground focus:bg-accent selected:bg-accent";

/// A dropdown choice among `options` (value, title). `width` 0 fills.
pub fn dropdown<T: Copy + PartialEq + 'static>(
    options: impl Fn() -> Vec<(T, String)> + 'static,
    selected: Signal<T>,
    leading: Option<Icon>,
    a11y: impl Fn() -> String + 'static,
    test_id: &'static str,
) -> View {
    let options = Rc::new(options);
    let o1 = options.clone();
    let o2 = options.clone();
    let items = move || {
        o1().into_iter()
            .enumerate()
            .map(|(i, (_, title))| SelectItem::new(i.to_string(), title))
            .collect::<Vec<_>>()
    };
    let value = move || {
        let cur = selected.get();
        options()
            .iter()
            .position(|(v, _)| *v == cur)
            .map(|i| SharedString::from(i.to_string()))
    };
    let title = {
        let o = o2.clone();
        move || {
            let cur = selected.get();
            o().into_iter()
                .find(|(v, _)| *v == cur)
                .map(|(_, t)| t)
                .unwrap_or_default()
        }
    };
    let mut trigger = view()
        .test_id(test_id)
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h(px(control::md()))
        .px(px(10.0))
        .rounded(px(radius::md()))
        .bg(Tone::Surface.color())
        .border_1()
        .border_color(Tone::BorderStrong.color())
        .shadow_xs()
        .cursor_pointer()
        .hover(|s| s.bg(Tone::Subtle.color()));
    if let Some(i) = leading {
        trigger = trigger.child(icon(i, 16.0, || Tone::Muted));
    }
    trigger = trigger
        .child(view().flex_1().min_w(px(0.0)).child(label_line(
            title,
            ty::SM,
            FontWeight::MEDIUM,
            Tone::Ink,
        )))
        .child(icon(Icon::ChevronsUpDown, 14.0, || Tone::Muted));
    let a11y = Rc::new(a11y);
    let el = select()
        .items(items)
        .value(value)
        .accessibility_label(move || SharedString::from(a11y()))
        .trigger(trigger)
        .popup_class(POPUP)
        .item_class(ITEM)
        .on_change(move |v: &Option<SharedString>| {
            if let Some(i) = v.as_deref().and_then(|s| s.parse::<usize>().ok())
                && let Some((val, _)) = o2().get(i)
            {
                selected.set(*val);
            }
        });
    view().child(el)
}

/// One entry of a row's actions menu.
pub struct Action {
    pub icon: Icon,
    pub title: String,
    pub danger: bool,
    pub run: Rc<dyn Fn()>,
}

impl Action {
    pub fn new(icon: Icon, title: String, run: impl Fn() + 'static) -> Self {
        Self {
            icon,
            title,
            danger: false,
            run: Rc::new(run),
        }
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
}

/// The ⋯ button at the end of a row, opening its actions. Destructive
/// actions come last, after a separator, in red.
pub fn row_menu(
    actions: impl Fn() -> Vec<Action> + 'static,
    a11y: impl Fn() -> String + 'static,
) -> View {
    let side = control::sm();
    let trigger = view()
        .w(px(side))
        .h(px(side))
        .items_center()
        .justify_center()
        .rounded(px(radius::md()))
        .cursor_pointer()
        .hover(|s| s.bg(Tone::Hover.color()))
        .label(move || SharedString::from(a11y()))
        .child(icon(Icon::More, 16.0, || Tone::Muted));
    // The app's one menu look (`menu.rs`).
    let acts = actions()
        .into_iter()
        .map(|a| {
            let run = a.run.clone();
            let act = super::menu::Act::with(a.icon, a.title, move || run());
            if a.danger { act.danger() } else { act }
        })
        .collect();
    super::menu::menu_on(trigger, acts)
}

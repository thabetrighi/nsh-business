//! Lucide icons (ISC license, `licenses/Lucide-ISC.txt`), recolored per
//! appearance so they render the same on native and web (where
//! `currentColor` would be black). Sizes: 14 (inline, badges), 16 (buttons,
//! menus, table), 18 (navigation), 20+ (empty states, KPI).

use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;

use super::tokens::Tone;

macro_rules! icons {
    ($($name:ident => $file:literal),* $(,)?) => {
        /// The app's icon set.
        #[allow(dead_code)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Icon { $($name),* }

        impl Icon {
            fn markup(self) -> &'static str {
                match self {
                    $(Icon::$name => include_str!(concat!("../../icons/", $file, ".svg"))),*
                }
            }
        }
    };
}

icons! {
    Home => "house",
    Dashboard => "layout-dashboard",
    Cart => "shopping-cart",
    Receipt => "receipt-text",
    Package => "package",
    Boxes => "boxes",
    Users => "users",
    Truck => "truck",
    Wallet => "wallet",
    Chart => "chart-column",
    Settings => "settings",
    Search => "search",
    Offline => "wifi-off",
    Synced => "cloud-check",
    Refresh => "refresh-cw",
    Sun => "sun",
    Moon => "moon",
    SunMoon => "sun-moon",
    Monitor => "monitor",
    Languages => "languages",
    Plus => "plus",
    Minus => "minus",
    Keyboard => "keyboard",
    PackagePlus => "package-plus",
    PlusCircle => "circle-plus",
    In => "arrow-down-left",
    Out => "arrow-up-right",
    Coins => "hand-coins",
    Alert => "triangle-alert",
    Store => "store",
    More => "ellipsis",
    MoreVertical => "ellipsis-vertical",
    User => "user-round",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    ChevronsUpDown => "chevrons-up-down",
    ArrowUp => "arrow-up",
    ArrowDown => "arrow-down",
    Banknote => "banknote",
    Check => "check",
    CheckCircle => "circle-check",
    Close => "x",
    XCircle => "circle-x",
    Pencil => "pencil",
    Trash => "trash",
    Copy => "copy",
    Eye => "eye",
    Download => "download",
    Upload => "upload",
    Bell => "bell",
    Help => "circle-question-mark",
    Brush => "paintbrush",
    Palette => "palette",
    Type => "type",
    Filter => "list-filter",
    Sliders => "sliders-horizontal",
    Calendar => "calendar",
    TrendUp => "trending-up",
    TrendDown => "trending-down",
    Printer => "printer",
    Layers => "layers",
    Columns => "columns-3",
    Tag => "tag",
    Info => "info",
    Clock => "clock",
    PanelStart => "panel-left",
    BookOpen => "book-open",
    Map => "map",
    FileText => "file-text",
    Undo => "undo-2",
    Percent => "badge-percent",
    Ruler => "ruler",
    Tags => "tags",
    ArrowsLR => "arrow-left-right",
    ClipboardCheck => "clipboard-check",
    Warehouse => "warehouse",
    CalendarClock => "calendar-clock",
    Landmark => "landmark",
    NotebookPen => "notebook-pen",
    ListTree => "list-tree",
    Scale => "scale",
    Book => "book",
    MapPin => "map-pin",
    ClipboardList => "clipboard-list",
    Building => "building",
    PercentPlain => "percent",
    Shield => "shield-check",
    Grip => "grip-vertical",
    LayoutTemplate => "layout-template",
    CloudUpload => "cloud-upload",
    Barcode => "scan-barcode",
    HardDrive => "hard-drive",
    ChartLine => "chart-line",
    ChartPie => "chart-pie",
    UserCog => "user-cog",
    Dollar => "circle-dollar-sign",
    FileChart => "file-chart-column",
    Key => "key-round",
    Lock => "lock",
    UserPlus => "user-plus",
    Backspace => "delete",
    Folder => "folder",
    FolderTree => "folder-tree",
    FileUp => "file-up",
    SubItem => "corner-down-right",
    ImagePlus => "image-plus",
    Wand => "wand-sparkles",
    ArrowBack => "arrow-left",
    ArrowForward => "arrow-right",
    Pause => "pause",
    LogOut => "log-out",
    History => "history",
    Zap => "zap",
    Maximize => "maximize-2",
    TextSize => "a-large-small",
    Box => "box",
    UserCheck => "user-round-check",
    Rotate => "rotate-ccw",
    BellRing => "bell-ring",
    LifeBuoy => "life-buoy",
    PanelBottom => "panel-bottom",
    Bag => "shopping-bag",
    Hash => "hash",
    UserX => "user-round-x",
    ArrowUpDown => "arrow-up-down",
    AlignCenter => "align-center",
    EyeOff => "eye-off",
    Star => "star",
    LayoutDashboard => "layout-dashboard",
    Apps => "layout-grid",
    PanelTop => "panel-top",
    Frame => "square-dashed",
    Move => "move",
    Scaling => "scaling",
    TrendingUp => "trending-up",
    Target => "target",
    Phone => "phone",
    Navigation => "navigation",
    Message => "message-circle",
    Play => "play",
    Flag => "flag",
    Route => "route",
    Locate => "locate-fixed",
}

/// An icon `size` points square, stroked in `tone`.
pub fn icon(which: Icon, size: f32, tone: impl Fn() -> Tone + 'static) -> impl IntoElement {
    let src = move || {
        let markup = which
            .markup()
            .replace("currentColor", &tone().hex())
            .replace("stroke-width=\"2\"", "stroke-width=\"1.75\"");
        ImageSrc::svg_markup(&markup)
    };
    let size = (size * super::tokens::scale()).round();
    svg(src).w(px(size)).h(px(size)).flex_shrink_0()
}

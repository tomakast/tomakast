//! Platform-independent core of tomakast: item model, index and matching.

pub mod core;
pub mod effect;
pub mod engine;
pub mod history;
pub mod home_file;
mod host;
pub mod hotkeys;
pub mod index;
pub mod keys;
pub mod layout;
pub mod links;
pub mod matcher;
pub mod module;
pub mod paths;
pub mod readings;
pub mod script;
pub mod settings;
pub mod settings_edit;
pub mod store;
pub mod strip;
#[cfg(feature = "testing")]
pub mod testing;
mod thread_time;
pub mod translit;
pub mod usage;
pub mod view;
pub mod widget;

pub use core::{Core, CoreUpdate, RELOADED, SearchSnapshot};
pub use effect::{Effect, Toast, ToastKind};
pub use engine::{DYNAMIC_MATCH_SCORE, Hit, PINNED_SCORE, SearchEngine};
pub use history::{HISTORY_CAP, HistoryEntry, MIN_QUERY_CHARS, QueryHistory};
pub use home_file::{
    ADD_TO_HOME, EDIT_HOME, HomeEdit, HomeWriteError, Removed, add_home, edit_home, read_home,
};
pub use host::DYNAMIC_SEARCH_DEADLINE;
pub use index::{
    Action, Icon, Index, Item, ItemId, ItemKind, ShortcutError, is_hidden_char, is_valid_id,
};
pub use layout::LAYOUTS;
pub use links::checked_url;
pub use matcher::{
    CROSS_MIN_RATIO, ItemKeys, LAYOUT_FACTOR, Matcher, Pattern, TRANSLIT_FACTOR, TextKeys,
};
pub use module::{CoreEvent, HomeTile, Module, ModuleContext, RenderedAnswer, RequestId};
pub use paths::Paths;
pub use readings::{
    MAX_READING_QUERY_CHARS, MAX_READINGS, QueryReading, ReadingKind, normalize_layout,
    query_readings,
};
pub use script::{Scripts, letter_count};
pub use settings::{HomeEntry, Hotkey, HotkeyError, LocationSettings, Modifiers, Settings};
pub use translit::{romanize, skeleton, skeleton_alt};
pub use usage::{
    LocalNow, RankWeights, UsageModel, UsageRecord, UsageStats, combined_score, usage_value,
};
pub use view::{
    Accessory, DetailView, FieldKind, FieldValue, FormField, FormView, GridTile, GridView,
    LineAnnotation, LinkPolicy, ListRow, ListView, MenuOption, ScratchpadView, SearchMode,
    ToolbarButton, View, ViewEvent, ViewId,
};
pub use widget::{
    Align, ButtonNode, CanvasNode, ClipRect, Color, ColorStop, ColorToken, DEBUG_DEADLINE_FACTOR,
    Draw, Edges, EllipseCmd, Frame, FrameError, IconNode, ImageCmd, ImageFit, ImageFormat,
    ImageNode, Justify, KeyEvent, LATE_FRAMES_TO_STOP, Layout, LinearGradient, MAX_DRAW_NESTING,
    MAX_FRAME_BYTES, MAX_FRAME_COMMANDS, MAX_FRAME_NODES, MAX_IMAGE_BYTES, MAX_IMAGES,
    MAX_IMAGES_BYTES, MAX_NODE_DEPTH, MAX_PAYLOAD_BYTES, MAX_SVG_BYTES, MAX_SVG_CONVOLVE_ORDER,
    MAX_SVG_ELEMENTS, MAX_SVG_FILTER_APPLICATIONS, MAX_SVG_FILTERS, MAX_SVG_IMAGE_PIXELS,
    MAX_SVG_IMAGES, MAX_SVG_MORPHOLOGY_RADIUS, MAX_SVG_NODES, MAX_SVG_OCTAVES, MAX_TEXT_SIZE,
    MAX_WIDGET_ANSWERS, MAX_WIDGETS, MIN_RENDER_INTERVAL, Node, NodeKind, Paint, Params, PathCmd,
    PathSegment, Point, PointerAt, PointerEvent, PointerKind, RectCmd, SOFT_RENDER_DEADLINE,
    SearchAnswer, ShapeStyle, Slot, TextAlign, TextCmd, TextNode, TextSizes, TextStyle,
    ThemeColors, TileKey, Transform, WidgetAnswer, WidgetContext, WidgetDecl, WidgetEvent,
    WidgetImage, WidgetKey, WidgetSize, WidgetTheme, check_frame, frame_bytes, params,
    soft_render_deadline,
};

//! Desktop route, layout, and sidebar components.

mod layout;
pub use layout::Layout;

mod analyze;
pub use analyze::Analyze;

mod monitor;
pub use monitor::Monitor;

mod settings;
pub use settings::{Settings, SettingsContext, SettingsPageState, SettingsSidebar};

mod sidebar;
pub use sidebar::Sidebar;

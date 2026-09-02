use dioxus::prelude::*;

use crate::views::{Analyze, Layout, Monitor, Settings};

/// Routes rendered inside the shared desktop layout.
#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(Layout)]
        #[route("/")]
        Monitor {},

        #[route("/analyze")]
        Analyze {},

        #[route("/settings")]
        Settings {},
}

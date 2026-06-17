//! Server-rendered UI routing and feature modules.

mod document;
pub mod features;
mod routing;

pub use document::{document, not_found_fragment};
#[allow(unused_imports)]
pub use routing::{
    Authz, AuthzError, AuthzRequest, NoInput, Public, ReuseGranted, Route, RouteContext,
    RouteError, RouteFragment, RouteView, embed, route,
};

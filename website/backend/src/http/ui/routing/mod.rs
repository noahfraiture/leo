//! Typed UI route abstraction over axum extractors and hypertext rendering.

mod authz;
mod compose;
mod context;
mod dispatch;
mod error;
mod input;
mod route;

pub use authz::{Authz, AuthzError, AuthzRequest, Public, ReuseGranted};
pub use compose::{RouteFragment, embed};
pub use context::RouteContext;
pub use dispatch::route;
pub use error::RouteError;
pub use input::NoInput;
pub use route::{Route, RouteView};

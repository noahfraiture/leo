use hypertext::prelude::*;

use crate::http::{
    router::AppState,
    ui::{NoInput, Public, Route, RouteContext, RouteError, RouteView, not_found_fragment},
};

/// Navigation/header feature used as an embedding fragment inside full pages.
pub struct TopBar;

/// View for `TopBar`; only the fragment render is intended for use.
pub struct TopBarView;

#[async_trait::async_trait]
impl Route for TopBar {
    type Input = NoInput;
    type Authz = Public;
    type View = TopBarView;

    async fn handle(
        _context: &RouteContext,
        _granted: (),
        _input: Self::Input,
    ) -> Result<Self::View, RouteError> {
        Ok(TopBarView)
    }
}

impl RouteView for TopBarView {
    fn document(&self, _state: &AppState) -> impl Renderable {
        not_found_fragment()
    }

    fn fragment(&self, _state: &AppState) -> impl Renderable {
        rsx! {
            <header class="navbar rounded-box border border-base-300 bg-base-100 px-4 shadow-sm">
                <div class="flex-1">
                    <a
                        class="btn btn-ghost px-0 text-xl font-semibold normal-case"
                        hx-get="/"
                        hx-target="#body"
                        hx-push-url="/">
                        "Video analysis"
                    </a>
                </div>
            </header>
        }
    }
}

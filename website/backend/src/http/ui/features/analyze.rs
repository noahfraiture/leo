use async_trait::async_trait;
use hypertext::prelude::*;

use crate::http::{
    router::AppState,
    ui::{NoInput, Public, Route, RouteContext, RouteError, RouteView, not_found_fragment},
};

pub struct AnalyzeRoute;

pub struct AnalyzeView {
    response: String,
}

#[async_trait]
impl Route for AnalyzeRoute {
    type Input = NoInput;
    type Authz = Public;
    type View = AnalyzeView;

    async fn handle(
        _context: &RouteContext,
        _granted: (),
        _input: Self::Input,
    ) -> Result<Self::View, RouteError> {
        Ok(AnalyzeView {
            response: "Video analysis is not implemented yet.".to_owned(),
        })
    }
}

impl RouteView for AnalyzeView {
    fn document(&self, _state: &AppState) -> impl Renderable {
        not_found_fragment()
    }

    fn fragment(&self, _state: &AppState) -> impl Renderable {
        rsx! {
            <p id="analysis-result" class="text-sm text-base-content/70">
                (self.response.as_str())
            </p>
        }
    }
}

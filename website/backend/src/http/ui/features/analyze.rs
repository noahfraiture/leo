use async_trait::async_trait;
use axum::{body::Bytes, extract::FromRequest};
use hypertext::prelude::*;
use serde::Deserialize;

use crate::{
    analysis::gemini,
    db,
    http::{
        router::AppState,
        ui::{Public, Route, RouteContext, RouteError, RouteView, not_found_fragment},
    },
};

pub struct AnalyzeRoute;

pub struct AnalyzeView {
    response: String,
}

#[derive(Deserialize)]
pub struct AnalyzeInput {
    #[serde(default)]
    video_keys: Vec<String>,
    #[serde(default)]
    prompt: String,
}

#[async_trait]
impl Route for AnalyzeRoute {
    type Input = AnalyzeInput;
    type Authz = Public;
    type View = AnalyzeView;

    async fn handle(
        context: &RouteContext,
        _granted: (),
        input: Self::Input,
    ) -> Result<Self::View, RouteError> {
        if input.video_keys.is_empty() {
            return Err(RouteError::BadRequest(
                "select at least one video to analyze",
            ));
        }

        if input.video_keys.len() > 10 {
            return Err(RouteError::BadRequest(
                "select no more than 10 videos to analyze",
            ));
        }

        if input.prompt.trim().is_empty() {
            return Err(RouteError::BadRequest("analysis prompt cannot be empty"));
        }

        let mut videos = Vec::with_capacity(input.video_keys.len());
        for key in input.video_keys {
            let Some(video) =
                db::video::Video::read_by_file_key(context.state().db(), &key).await?
            else {
                return Err(RouteError::BadRequest("selected video was not found"));
            };
            videos.push(video);
        }

        let response = gemini::analyze_videos(&videos, input.prompt.trim()).await?;

        Ok(AnalyzeView { response })
    }
}

impl<S> FromRequest<S> for AnalyzeInput
where
    S: Send + Sync,
{
    type Rejection = RouteError;

    async fn from_request(
        request: axum::extract::Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|_| RouteError::BadRequest("invalid analysis form"))?;

        serde_html_form::from_bytes(&bytes)
            .map_err(|_| RouteError::BadRequest("invalid analysis form"))
    }
}

impl RouteView for AnalyzeView {
    fn document(&self, _state: &AppState) -> impl Renderable {
        not_found_fragment()
    }

    fn fragment(&self, _state: &AppState) -> impl Renderable {
        rsx! {
            <div id="analysis-result" class="whitespace-pre-wrap text-sm leading-6 text-base-content/80">
                (self.response.as_str())
            </div>
        }
    }
}

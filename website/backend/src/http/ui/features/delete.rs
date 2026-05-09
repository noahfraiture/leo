use async_trait::async_trait;
use axum::extract::Path;
use hypertext::prelude::*;

use crate::{
    db,
    http::{
        router::AppState,
        ui::{NoInput, Public, Route, RouteContext, RouteError, RouteView, document},
    },
};

use super::home::video_workspace;

pub struct DeleteVideoRoute;

pub struct DeleteVideoView {
    videos: Vec<db::video::Video>,
}

#[async_trait]
impl Route for DeleteVideoRoute {
    type Input = (Path<String>, NoInput);
    type Authz = Public;
    type View = DeleteVideoView;

    async fn handle(
        context: &RouteContext,
        _granted: (),
        (Path(video_key), _): Self::Input,
    ) -> Result<Self::View, RouteError> {
        let video = db::video::Video::find_by_file_key(context.state().db(), &video_key)
            .await?
            .ok_or(RouteError::BadRequest("video does not exist"))?;

        video.delete(context.state().db()).await?;
        let videos = db::video::Video::list(context.state().db()).await?;

        Ok(DeleteVideoView { videos })
    }
}

impl RouteView for DeleteVideoView {
    fn document(&self, state: &AppState) -> impl Renderable {
        document(
            state,
            "Video analysis | Videos",
            rsx! {
                <main class="mx-auto max-w-4xl space-y-8 p-6 lg:py-10">
                    <section class="space-y-6 rounded-box border border-base-300 bg-base-100 p-5 shadow-sm">
                        <h1 class="text-2xl font-semibold text-base-content">"Uploaded videos"</h1>
                        (video_workspace(&self.videos))
                        <a class="btn btn-primary" href="/">"Back to analysis"</a>
                    </section>
                </main>
            },
        )
    }

    fn fragment(&self, _state: &AppState) -> impl Renderable {
        video_workspace(&self.videos)
    }
}

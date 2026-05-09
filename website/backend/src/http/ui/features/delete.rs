use async_trait::async_trait;
use axum::extract::Path;
use hypertext::prelude::*;

use crate::{
    db,
    http::{
        router::AppState,
        ui::{NoInput, Public, Route, RouteContext, RouteError, RouteView, not_found_fragment},
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
    fn document(&self, _state: &AppState) -> impl Renderable {
        not_found_fragment()
    }

    fn fragment(&self, _state: &AppState) -> impl Renderable {
        video_workspace(&self.videos)
    }
}

use async_trait::async_trait;
use axum::extract::Multipart;
use hypertext::prelude::*;

use crate::{
    db,
    http::{
        router::AppState,
        ui::{Public, Route, RouteContext, RouteError, RouteView, document},
    },
};

use super::home::video_selection;

pub struct UploadVideoRoute;

pub struct UploadVideoView {
    videos: Vec<db::video::Video>,
}

#[async_trait]
impl Route for UploadVideoRoute {
    type Input = Multipart;
    type Authz = Public;
    type View = UploadVideoView;

    async fn handle(
        context: &RouteContext,
        _granted: (),
        mut input: Self::Input,
    ) -> Result<Self::View, RouteError> {
        let (filename, bytes) = uploaded_video(&mut input).await?;

        db::video::Video::upload(context.state().db(), filename, bytes).await?;
        let videos = db::video::Video::list(context.state().db()).await?;

        Ok(UploadVideoView { videos })
    }
}

impl RouteView for UploadVideoView {
    fn document(&self, state: &AppState) -> impl Renderable {
        document(
            state,
            "Video analysis | Videos",
            rsx! {
                <main class="mx-auto max-w-4xl space-y-8 p-6 lg:py-10">
                    <section class="space-y-6 rounded-box border border-base-300 bg-base-100 p-5 shadow-sm">
                        <h1 class="text-2xl font-semibold text-base-content">"Uploaded videos"</h1>
                        (video_selection(&self.videos))
                        <a class="btn btn-primary" href="/">"Back to analysis"</a>
                    </section>
                </main>
            },
        )
    }

    fn fragment(&self, _state: &AppState) -> impl Renderable {
        video_selection(&self.videos)
    }
}

async fn uploaded_video(multipart: &mut Multipart) -> Result<(String, Vec<u8>), RouteError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| RouteError::BadRequest("invalid multipart upload"))?
    {
        if field.name() != Some("video") {
            continue;
        }

        let filename = field
            .file_name()
            .map(str::to_owned)
            .ok_or(RouteError::BadRequest("video upload requires a file name"))?;
        let bytes = field
            .bytes()
            .await
            .map_err(|_| RouteError::BadRequest("invalid video upload"))?
            .to_vec();

        if bytes.is_empty() {
            return Err(RouteError::BadRequest("video upload cannot be empty"));
        }

        return Ok((filename, bytes));
    }

    Err(RouteError::BadRequest("missing video upload field"))
}

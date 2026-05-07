use async_trait::async_trait;
use axum::http::StatusCode;
use hypertext::prelude::*;

use super::TopBar;
use crate::http::{
    client::{islands, props},
    router::AppState,
    ui::{
        self, NoInput, Public, Route, RouteContext, RouteError, RouteFragment, RouteView, document,
    },
};

/// Shared page frame used by mounted pages that should keep the same top bar
/// and HTMX swap target. Child pages embed their replaceable content inside
/// `#body`, so regular navigation returns a full document and HTMX navigation
/// swaps only that inner region.
pub struct HomeFrame {
    top_bar: RouteFragment<TopBar>,
}

impl HomeFrame {
    pub async fn new(context: &RouteContext) -> Result<Self, RouteError> {
        let top_bar = ui::embed::<TopBar>()
            .input(NoInput)
            .resolve(context)
            .await?;

        Ok(Self { top_bar })
    }

    pub fn embed(&self, body: impl Renderable) -> impl Renderable {
        rsx! {
            <main class="mx-auto max-w-4xl space-y-8 p-6 lg:py-10">
                (self.top_bar)
                <div id="body">(body)</div>
            </main>
        }
    }
}

/// Public starter page mounted at `/`.
///
/// The current shell keeps the SSR/HTMX/Solid plumbing in place while the
/// upload and analysis workflow is built out.
pub struct HomePage;

pub struct HomePageView {
    home: HomeFrame,
}

#[async_trait]
impl Route for HomePage {
    type Input = NoInput;
    type Authz = Public;
    type View = HomePageView;

    async fn handle(
        context: &RouteContext,
        _granted: (),
        _input: Self::Input,
    ) -> Result<Self::View, RouteError> {
        let home = HomeFrame::new(context).await?;

        Ok(HomePageView { home })
    }
}

impl HomePageView {
    fn body(&self) -> impl Renderable {
        let island_props = props::ExampleIslandProps {
            label: "Analysis status".to_owned(),
            initial_count: 0,
        };
        let island = islands::host_with_props::<islands::ExampleIsland, _>(&island_props);

        rsx! {
            <section class="space-y-6">
                <div class="space-y-2">
                    <h1 class="text-3xl font-semibold text-base-content">"Upload videos"</h1>
                    <p class="max-w-2xl text-base-content/70">
                        "A server-rendered workspace for video uploads and AI analysis with OpenAI and Gemini."
                    </p>
                </div>

                <div class="grid gap-6 lg:grid-cols-[minmax(0,1fr)_20rem]">
                    <section class="rounded-box border border-dashed border-base-300 bg-base-100 p-6 shadow-sm">
                        <div class="space-y-3">
                            <p class="text-sm font-semibold uppercase tracking-[0.22em] text-base-content/60">
                                "Next"
                            </p>
                            <h2 class="text-xl font-semibold text-base-content">"Video intake"</h2>
                            <p class="text-sm leading-6 text-base-content/70">
                                "The upload handler and provider analysis jobs can be added on top of this shell."
                            </p>
                        </div>
                    </section>

                    <aside class="rounded-box border border-base-300 bg-base-100 p-4 shadow-sm">
                        (island)
                    </aside>
                </div>
            </section>
        }
    }
}

impl RouteView for HomePageView {
    fn document(&self, state: &AppState) -> impl Renderable {
        document(state, "Video analysis | Home", self.home.embed(self.body()))
    }

    fn fragment(&self, _state: &AppState) -> impl Renderable {
        self.body()
    }
}

pub async fn healthz() -> (StatusCode, &'static str) {
    (StatusCode::OK, "OK")
}

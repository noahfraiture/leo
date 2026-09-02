use std::rc::Rc;

use dioxus::{
    history::{History, MemoryHistory},
    prelude::*,
    router::components::HistoryProvider,
};

use super::bootstrap::{Bootstrap, InitialSettings, InitialWorkflow, RecorderBootstrap};
use crate::{
    Route, session_task,
    views::{SettingsContext, SettingsPageState},
};

const FAVICON: Asset = asset!("/assets/favicon.ico");
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

/// Describes which operational routes can use their concrete runtime contexts.
#[derive(Clone, PartialEq)]
pub enum RuntimeAvailability {
    Ready { camera_count: usize },
    SetupRequired,
    Failed { message: String },
}

/// Installs shell-wide contexts before selecting the setup or ready router root.
#[component]
pub fn App() -> Element {
    let bootstrap = use_context::<Bootstrap>();
    let InitialSettings {
        store,
        draft,
        initial_route,
    } = use_context::<InitialSettings>();
    let settings =
        use_hook(move || Signal::new_in_scope(SettingsPageState::new(draft), ScopeId::ROOT));
    use_context_provider(move || SettingsContext {
        state: settings,
        store,
    });
    let availability = match &bootstrap {
        Bootstrap::Ready { config, .. } => RuntimeAvailability::Ready {
            camera_count: config.settings.cameras.len(),
        },
        Bootstrap::SetupRequired => RuntimeAvailability::SetupRequired,
        Bootstrap::Failed { message } => RuntimeAvailability::Failed {
            message: message.clone(),
        },
    };
    use_context_provider(move || availability);
    let body = match bootstrap {
        Bootstrap::Ready { .. } => rsx! { ReadyApp { initial_route } },
        Bootstrap::SetupRequired | Bootstrap::Failed { .. } => {
            rsx! { ShellRouter { initial_route } }
        }
    };

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        {body}
    }
}

/// Transfers the one-shot workflow and recorder events into root-scoped UI state.
#[component]
fn ReadyApp(initial_route: Route) -> Element {
    let initial_workflow = use_context::<InitialWorkflow>();
    let recorder = use_context::<RecorderBootstrap>();
    let event_recorder = recorder.clone();
    let mut workflow = use_hook(move || {
        let workflow = initial_workflow
            .0
            .lock()
            .expect("initial workflow mutex should not be poisoned")
            .take()
            .expect("Ready root should take its initialized Workflow exactly once");
        Signal::new_in_scope(workflow, ScopeId::ROOT)
    });
    use_context_provider(|| workflow);

    let _event_task = use_hook(move || {
        let mut events = event_recorder
            .events
            .lock()
            .expect("recorder event receiver mutex should not be poisoned")
            .take()
            .expect("ready root should take recorder events exactly once");
        dioxus::dioxus_core::spawn_forever(async move {
            while let Some(event) = events.recv().await {
                let cleanup = {
                    let mut state = workflow.write();
                    session_task::handle_recorder_event(&mut state, event)
                };
                if let Some(request) = cleanup {
                    session_task::spawn_fault_cleanup(workflow, request);
                }
            }
            tracing::warn!("recorder event channel closed");
            let cleanup = {
                let mut state = workflow.write();
                session_task::handle_recorder_event_channel_closed(&mut state)
            };
            if let Some(request) = cleanup {
                session_task::spawn_fault_cleanup(workflow, request);
            }
        })
    });

    let desktop_e2e: Element = {
        #[cfg(feature = "desktop-e2e")]
        {
            rsx! { crate::desktop_e2e::DesktopE2eDriver {} }
        }
        #[cfg(not(feature = "desktop-e2e"))]
        {
            rsx! {}
        }
    };

    rsx! {
        ShellRouter { initial_route }
        {desktop_e2e}
    }
}

#[component]
fn ShellRouter(initial_route: Route) -> Element {
    let initial_path = initial_route.to_string();
    rsx! {
        HistoryProvider {
            history: move |_| {
                Rc::new(MemoryHistory::with_initial_path(initial_path.clone())) as Rc<dyn History>
            },
            Router::<Route> {}
        }
    }
}

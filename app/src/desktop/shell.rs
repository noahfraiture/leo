//! Root Dioxus shell shared by ready, first-run, and failed desktop startup states.
//!
//! The shell installs application-wide contexts and routing; operational state is added only when
//! desktop startup succeeds.

use std::rc::Rc;

use dioxus::{
    history::{History, MemoryHistory},
    prelude::*,
    router::components::HistoryProvider,
};

use super::bootstrap::{Bootstrap, InitialOperatorState, InitialSettings, RecorderBootstrap};
use crate::{
    Route, operator,
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

/// Transfers one-shot operator state and recorder events into root-scoped UI state.
#[component]
fn ReadyApp(initial_route: Route) -> Element {
    let initial_operator = use_context::<InitialOperatorState>();
    let recorder = use_context::<RecorderBootstrap>();
    let event_recorder = recorder.clone();
    let mut operator_state = use_hook(move || {
        let operator = initial_operator
            .0
            .lock()
            .expect("initial operator-state mutex should not be poisoned")
            .take()
            .expect("ready root should take its initial operator state exactly once");
        Signal::new_in_scope(operator, ScopeId::ROOT)
    });
    use_context_provider(|| operator_state);

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
                    let mut state = operator_state.write();
                    operator::handle_recorder_event(&mut state, event)
                };
                if let Some(request) = cleanup {
                    operator::spawn_fault_cleanup(operator_state, request);
                }
            }
            tracing::warn!("recorder event channel closed");
            let cleanup = {
                let mut state = operator_state.write();
                operator::handle_recorder_event_channel_closed(&mut state)
            };
            if let Some(request) = cleanup {
                operator::spawn_fault_cleanup(operator_state, request);
            }
        })
    });

    let desktop_e2e: Element = {
        #[cfg(feature = "desktop-e2e")]
        {
            rsx! { crate::e2e::DesktopE2eDriver {} }
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

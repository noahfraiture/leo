use std::{
    rc::Rc,
    sync::{Arc, Mutex},
};

use backend::recording::{RecorderEvent, RecorderHandle, RecorderRuntime};
use dioxus::{
    desktop::{Config, tao::event::Event},
    history::{History, MemoryHistory},
    prelude::*,
    router::components::HistoryProvider,
};
use tokio::sync::mpsc::UnboundedReceiver;

use logging::LogGuard;
use preview::{Bridge, CameraSource, PreviewState, bridge};
use settings::{LogLevel, ResolvedSettings, Settings as ApplicationSettings, SettingsStore};
use views::{Analyze, Layout, Monitor, Settings, SettingsContext, SettingsPageState};
use workflow::Workflow;

mod analysis_task;
mod components;
#[cfg(feature = "desktop-e2e")]
mod desktop_e2e;
mod logging;
#[cfg(all(test, feature = "paid-openai-test"))]
mod paid_openai_workflow;
mod preview;
mod session_task;
mod settings;
mod views;
mod workflow;

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(Layout)]
        #[route("/")]
        Monitor {},

        #[route("/analyze")]
        Analyze {},

        #[route("/settings")]
        Settings {},
}

const FAVICON: Asset = asset!("/assets/favicon.ico");
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

#[derive(Clone)]
struct RecorderBootstrap {
    handle: RecorderHandle,
    events: Arc<Mutex<Option<UnboundedReceiver<RecorderEvent>>>>,
}

#[derive(Clone)]
struct InitialWorkflow(Arc<Mutex<Option<Workflow>>>);

/// Describes which operational routes can use their concrete runtime contexts.
#[derive(Clone, PartialEq)]
pub enum RuntimeAvailability {
    Ready { camera_count: usize },
    SetupRequired,
    Failed { message: String },
}

#[cfg(test)]
fn test_openai_config() -> backend::analysis::OpenAiConfig {
    backend::analysis::OpenAiConfig {
        api_key: "test-api-key".into(),
        model: "test-model".into(),
        base_url: Some("http://provider.example/v1".into()),
    }
}

fn initialize_workflow(
    config: &ResolvedSettings,
    recorder: &RecorderBootstrap,
) -> Result<InitialWorkflow, String> {
    Workflow::new(
        config.settings.cameras.clone(),
        config.sessions_root.clone(),
        recorder.handle.clone(),
        config.openai.clone(),
        config.analysis_frame_sets_per_prompt,
        config.analysis_overlap_frame_sets,
    )
    .map(|workflow| InitialWorkflow(Arc::new(Mutex::new(Some(workflow)))))
    .map_err(|_| "Session workflow is unavailable.".into())
}

fn take_initial_workflow(initial: &InitialWorkflow) -> Option<Workflow> {
    initial
        .0
        .lock()
        .expect("initial workflow mutex should not be poisoned")
        .take()
}

fn take_recorder_events(recorder: &RecorderBootstrap) -> Option<UnboundedReceiver<RecorderEvent>> {
    recorder
        .events
        .lock()
        .expect("recorder event receiver mutex should not be poisoned")
        .take()
}

/// Operational startup result kept separate from the always-available shell.
#[derive(Clone)]
enum Bootstrap {
    Ready {
        config: Box<ResolvedSettings>,
        preview: PreviewState,
        recorder: RecorderBootstrap,
        workflow: InitialWorkflow,
    },
    SetupRequired,
    Failed {
        message: String,
    },
}

/// Settings store, editable values, and route used to initialize the shell once.
#[derive(Clone)]
struct InitialSettings {
    store: SettingsStore,
    draft: ApplicationSettings,
    initial_route: Route,
}

/// Validates startup dependencies and launches the desktop operator app.
pub fn launch() {
    let store = SettingsStore::platform().expect("platform settings paths should be available");
    launch_with_store(store);
}

#[cfg(feature = "desktop-e2e")]
#[doc(hidden)]
pub fn launch_desktop_e2e(settings_path: std::path::PathBuf) {
    let default_data_root = settings_path
        .parent()
        .expect("E2E settings path should have a parent")
        .join("default-data");
    launch_with_store(SettingsStore::new(settings_path, default_data_root).unwrap());
}

fn launch_with_store(store: SettingsStore) {
    let mut recorder_owner: Option<RecorderRuntime> = None;
    let mut preview_owner: Option<Bridge> = None;
    let mut log_owner: Option<LogGuard> = None;

    let loaded = store
        .load()
        .unwrap_or_else(|error| panic!("application settings could not be loaded: {error}"));
    let (bootstrap, initial_settings) = match loaded {
        None => {
            let _ = logging::init_stderr(LogLevel::Info);
            tracing::info!("application settings setup is required");
            (
                Bootstrap::SetupRequired,
                InitialSettings {
                    store,
                    draft: ApplicationSettings::default(),
                    initial_route: Route::Settings {},
                },
            )
        }
        Some(config) => {
            let initial_settings = InitialSettings {
                store,
                draft: config.settings.clone(),
                initial_route: Route::Monitor {},
            };
            let bootstrap = match logging::init(&config.logs_root, config.log_level) {
                Err(_error) => {
                    let _ = logging::init_stderr(config.log_level);
                    tracing::error!("application logging initialization failed");
                    Bootstrap::Failed {
                        message: "Application logging is unavailable.".into(),
                    }
                }
                Ok(log_guard) => {
                    log_owner = Some(log_guard);
                    tracing::info!("startup configuration loaded");

                    match RecorderRuntime::spawn(config.recorder_settings) {
                        Err(error) => {
                            tracing::error!(error = %error, "recorder preflight failed");
                            Bootstrap::Failed {
                                message: "Recorder preflight failed.".into(),
                            }
                        }
                        Ok((runtime, handle, events)) => {
                            recorder_owner = Some(runtime);
                            let recorder = RecorderBootstrap {
                                handle,
                                events: Arc::new(Mutex::new(Some(events))),
                            };
                            match initialize_workflow(&config, &recorder) {
                                Err(message) => {
                                    tracing::error!("session workflow initialization failed");
                                    Bootstrap::Failed { message }
                                }
                                Ok(workflow) => {
                                    let sources = config
                                        .settings
                                        .cameras
                                        .iter()
                                        .map(|camera| CameraSource {
                                            id: camera.id,
                                            name: camera.name.clone(),
                                            rtsp_url: camera.rtsp_url.clone(),
                                        })
                                        .collect::<Vec<_>>();
                                    let (preview, bridge) = if sources.is_empty() {
                                        tracing::info!(
                                            "preview skipped because no cameras are configured"
                                        );
                                        (PreviewState::NoCameras, None)
                                    } else {
                                        tracing::info!("preview startup requested");
                                        match bridge::start(sources) {
                                            Ok((state, bridge)) => {
                                                tracing::info!("preview ready");
                                                (state, Some(bridge))
                                            }
                                            Err(error) => {
                                                tracing::warn!(
                                                    error = %error,
                                                    "preview unavailable"
                                                );
                                                (
                                                    PreviewState::Unavailable {
                                                        message: error.to_string(),
                                                    },
                                                    None,
                                                )
                                            }
                                        }
                                    };
                                    preview_owner = bridge;

                                    Bootstrap::Ready {
                                        config: Box::new(config),
                                        preview,
                                        recorder,
                                        workflow,
                                    }
                                }
                            }
                        }
                    }
                }
            };
            (bootstrap, initial_settings)
        }
    };

    let desktop = Config::new().with_custom_event_handler(move |event, _| {
        if !matches!(event, Event::LoopDestroyed) {
            return;
        }

        if let Some(runtime) = recorder_owner.take() {
            tracing::info!("recorder runtime stopping");
            if let Err(error) = runtime.shutdown() {
                tracing::error!(error = %error, "recorder runtime shutdown failed");
            } else {
                tracing::info!("recorder runtime stopped");
            }
        }
        if let Some(bridge) = preview_owner.take() {
            tracing::info!("preview stopping");
            if let Err(error) = bridge.stop() {
                tracing::error!(error = %error, "preview stop failed");
            } else {
                tracing::info!("preview stopped");
            }
        }
        tracing::info!("application logging stopping");
        drop(log_owner.take());
    });

    let launcher = dioxus::LaunchBuilder::desktop();
    let launcher = match &bootstrap {
        Bootstrap::Ready {
            config,
            preview,
            recorder,
            workflow,
        } => launcher
            .with_context(config.as_ref().clone())
            .with_context(preview.clone())
            .with_context(recorder.clone())
            .with_context(workflow.clone()),
        Bootstrap::SetupRequired | Bootstrap::Failed { .. } => launcher,
    };
    launcher
        .with_context(initial_settings)
        .with_context(bootstrap)
        .with_cfg(desktop)
        .launch(App);
}

#[component]
fn App() -> Element {
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

#[component]
fn ReadyApp(initial_route: Route) -> Element {
    let initial_workflow = use_context::<InitialWorkflow>();
    let recorder = use_context::<RecorderBootstrap>();
    let event_recorder = recorder.clone();
    let mut workflow = use_hook(move || {
        Signal::new_in_scope(
            take_initial_workflow(&initial_workflow)
                .expect("Ready root should take its initialized Workflow exactly once"),
            ScopeId::ROOT,
        )
    });
    use_context_provider(|| workflow);

    let _event_task = use_hook(move || {
        let mut events = take_recorder_events(&event_recorder)
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
            rsx! { desktop_e2e::DesktopE2eDriver {} }
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

#[cfg(all(test, unix))]
mod tests {
    use std::{
        fs,
        num::NonZeroUsize,
        os::unix::fs::PermissionsExt,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use backend::{
        recording::{RecorderEvent, RecorderSettings, RecorderStatus, spawn_for_test},
        session::SessionController,
    };

    use super::{
        RecorderBootstrap, initialize_workflow, take_initial_workflow, take_recorder_events,
    };
    use crate::{
        session_task::handle_recorder_event,
        settings::{CameraSettings, Settings, SettingsStore},
        workflow::{SessionRunState, Workflow},
    };

    fn camera_settings() -> Vec<CameraSettings> {
        [1_u32, 2]
            .into_iter()
            .map(|id| CameraSettings {
                id,
                name: format!("Salon {id}"),
                rtsp_url: format!("rtsp://camera-{id}.example/live"),
                initially_included_in_analysis: true,
                sample_every_ms: 1_000,
            })
            .collect()
    }

    fn test_recorder() -> (
        tempfile::TempDir,
        backend::recording::RecorderRuntime,
        RecorderBootstrap,
    ) {
        let temporary = tempfile::tempdir().expect("temporary root should be created");
        let executable = temporary.path().join("successful-preflight");
        fs::write(&executable, "#!/bin/sh\nexit 0\n")
            .expect("fake preflight executable should be written");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
            .expect("fake preflight executable should be executable");
        let (runtime, handle, events) = spawn_for_test(
            RecorderSettings {
                io_timeout: Duration::from_secs(1),
                retry_delay: Duration::from_secs(1),
                stop_timeout: Duration::from_secs(1),
            },
            executable.clone(),
            executable,
        )
        .expect("test recorder runtime should start");
        (
            temporary,
            runtime,
            RecorderBootstrap {
                handle,
                events: Arc::new(Mutex::new(Some(events))),
            },
        )
    }

    #[test]
    fn recorder_event_receiver_is_taken_exactly_once() {
        let (_temporary, runtime, recorder) = test_recorder();

        assert!(take_recorder_events(&recorder).is_some());
        assert!(take_recorder_events(&recorder).is_none());

        runtime.shutdown().expect("runtime should shut down");
    }

    #[test]
    fn workflow_bootstrap_reports_catalogue_io_without_taking_receiver() {
        let (temporary, runtime, recorder) = test_recorder();
        let data_root = temporary.path().join("data");
        fs::create_dir(&data_root).expect("data root should be created");
        let settings = Settings {
            next_camera_id: 3,
            cameras: camera_settings(),
            ..Settings::default()
        };
        let store = SettingsStore::new(temporary.path().join("config/settings.json"), data_root)
            .expect("test settings paths should be valid");
        let config = store
            .resolve(settings)
            .expect("test settings should resolve");
        fs::write(&config.sessions_root, b"not a directory")
            .expect("catalogue root should become invalid after startup validation");

        let Err(error) = initialize_workflow(&config, &recorder) else {
            panic!("invalid catalogue should make Workflow unavailable");
        };

        assert!(error.contains("Session workflow is unavailable"));
        assert!(take_recorder_events(&recorder).is_some());
        runtime.shutdown().expect("runtime should shut down");
    }

    #[test]
    fn workflow_bootstrap_copies_active_analysis_batching_settings() {
        let (temporary, runtime, recorder) = test_recorder();
        let store = SettingsStore::new(
            temporary.path().join("config/settings.json"),
            temporary.path().join("data"),
        )
        .expect("test settings paths should be valid");
        let config = store
            .resolve(Settings {
                analysis_frame_sets_per_prompt: 7,
                analysis_overlap_frame_sets: 2,
                ..Settings::default()
            })
            .expect("test settings should resolve");

        let initial = initialize_workflow(&config, &recorder).expect("workflow should initialize");
        let workflow = take_initial_workflow(&initial).expect("workflow should be retained");

        assert_eq!(workflow.analysis_frame_sets_per_prompt.get(), 7);
        assert_eq!(workflow.analysis_overlap_frame_sets, 2);
        runtime.shutdown().expect("runtime should shut down");
    }

    #[test]
    fn root_event_dispatch_updates_reconnecting_and_claims_one_fatal_cleanup() {
        let (temporary, runtime, recorder) = test_recorder();
        let mut workflow = Workflow::new(
            camera_settings(),
            temporary.path().join("sessions"),
            recorder.handle.clone(),
            Some(crate::test_openai_config()),
            NonZeroUsize::new(5).unwrap(),
            0,
        )
        .expect("workflow should initialize");
        let request = workflow
            .begin_start(1_786_552_800_000)
            .expect("session should begin starting");
        let controller = SessionController::create(request.events_path, request.session_cameras)
            .expect("session controller should start");
        workflow.finish_start(request.directory, controller);

        assert!(
            handle_recorder_event(
                &mut workflow,
                RecorderEvent::Status {
                    camera_id: 2,
                    status: RecorderStatus::Reconnecting,
                    message: Some("camera stream interrupted".into()),
                },
            )
            .is_none()
        );
        assert_eq!(
            workflow.cameras[1].recorder_status,
            RecorderStatus::Reconnecting
        );
        assert!(matches!(workflow.session, SessionRunState::Active { .. }));

        let cleanup = handle_recorder_event(
            &mut workflow,
            RecorderEvent::Faulted {
                camera_id: Some(2),
                message: "recorder storage failed".into(),
            },
        )
        .expect("first fatal event should claim cleanup");
        assert!(cleanup.controller.is_some());
        assert!(matches!(workflow.session, SessionRunState::Faulted { .. }));
        assert!(
            handle_recorder_event(
                &mut workflow,
                RecorderEvent::Faulted {
                    camera_id: Some(1),
                    message: "duplicate fatal event".into(),
                },
            )
            .is_none()
        );

        runtime.shutdown().expect("runtime should shut down");
    }
}

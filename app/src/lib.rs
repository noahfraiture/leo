use std::sync::{Arc, Mutex};

use backend::recording::{RecorderEvent, RecorderHandle, RecorderRuntime};
use dioxus::{
    desktop::{Config, tao::event::Event},
    prelude::*,
};
use tokio::sync::mpsc::UnboundedReceiver;

use camera_config::{StartupConfig, load_startup_config};
use logging::LogGuard;
use preview::{Bridge, CameraSource, PreviewState, bridge};
use views::{Analyze, Layout, Monitor};
use workflow::Workflow;

mod analysis_task;
mod camera_config;
mod components;
#[cfg(feature = "desktop-e2e")]
mod desktop_e2e;
mod logging;
#[cfg(all(test, feature = "paid-openai-test"))]
mod paid_openai_workflow;
mod preview;
mod session_task;
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

const MODEL_CONFIG_ERROR: &str = "Analysis requires OPENAI_API_KEY and ANALYSIS_MODEL.";

fn model_config_error(openai_api_key: bool, analysis_model: bool) -> Option<String> {
    (!openai_api_key || !analysis_model).then(|| MODEL_CONFIG_ERROR.to_owned())
}

fn configuration_value_is_present(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn initialize_workflow(
    config: &StartupConfig,
    recorder: &RecorderBootstrap,
) -> Result<InitialWorkflow, String> {
    let openai_api_key = std::env::var("OPENAI_API_KEY").ok();
    let analysis_model = std::env::var("ANALYSIS_MODEL").ok();
    Workflow::new(
        config.cameras.clone(),
        config.sessions_root.clone(),
        recorder.handle.clone(),
        model_config_error(
            configuration_value_is_present(openai_api_key.as_deref()),
            configuration_value_is_present(analysis_model.as_deref()),
        ),
    )
    .map(|workflow| InitialWorkflow(Arc::new(Mutex::new(Some(workflow)))))
    .map_err(|error| format!("Session workflow is unavailable: {error}"))
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

#[derive(Clone)]
enum Bootstrap {
    Ready {
        config: Box<StartupConfig>,
        preview: PreviewState,
        recorder: RecorderBootstrap,
        workflow: InitialWorkflow,
    },
    Unavailable {
        message: String,
    },
}

/// Validates startup dependencies and launches the desktop operator app.
pub fn launch() {
    let mut recorder_owner: Option<RecorderRuntime> = None;
    let mut preview_owner: Option<Bridge> = None;
    let mut log_owner: Option<LogGuard> = None;

    let bootstrap = match load_startup_config() {
        Err(error) => Bootstrap::Unavailable {
            message: format!("Startup configuration is unavailable: {error}"),
        },
        Ok(config) => match logging::init(&config.logs_root) {
            Err(error) => Bootstrap::Unavailable {
                message: format!("Application logging is unavailable: {error}"),
            },
            Ok(log_guard) => {
                log_owner = Some(log_guard);
                let camera_ids = config
                    .cameras
                    .iter()
                    .map(|camera| camera.id)
                    .collect::<Vec<_>>();
                tracing::info!(
                    camera_count = config.cameras.len(),
                    camera_ids = ?camera_ids,
                    data_root = %config.data_root.display(),
                    sessions_root = %config.sessions_root.display(),
                    "startup configuration loaded"
                );

                match RecorderRuntime::spawn(config.recorder_settings) {
                    Err(error) => {
                        tracing::error!(error = %error, "recorder preflight failed");
                        Bootstrap::Unavailable {
                            message: format!("Recorder preflight failed: {error}"),
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
                                Bootstrap::Unavailable { message }
                            }
                            Ok(workflow) => {
                                let sources = config
                                    .cameras
                                    .iter()
                                    .map(|camera| CameraSource {
                                        id: camera.id,
                                        name: camera.name.clone(),
                                        rtsp_url: camera.rtsp_url.clone(),
                                    })
                                    .collect::<Vec<_>>();
                                tracing::info!(
                                    camera_count = sources.len(),
                                    camera_ids = ?camera_ids,
                                    "preview startup requested"
                                );
                                let preview = match bridge::start(sources) {
                                    Ok((state, bridge)) => {
                                        preview_owner = Some(bridge);
                                        tracing::info!(
                                            camera_count = config.cameras.len(),
                                            camera_ids = ?camera_ids,
                                            "preview ready"
                                        );
                                        state
                                    }
                                    Err(error) => {
                                        tracing::warn!(
                                            error = %error,
                                            camera_count = config.cameras.len(),
                                            camera_ids = ?camera_ids,
                                            "preview unavailable"
                                        );
                                        PreviewState::Unavailable {
                                            message: error.to_string(),
                                        }
                                    }
                                };

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
        },
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
        Bootstrap::Unavailable { .. } => launcher,
    };
    launcher
        .with_context(bootstrap)
        .with_cfg(desktop)
        .launch(App);
}

#[component]
fn App() -> Element {
    let bootstrap = use_context::<Bootstrap>();
    let body = match bootstrap {
        Bootstrap::Ready { .. } => rsx! { ReadyApp {} },
        Bootstrap::Unavailable { message } => rsx! {
            main {
                class: "p-4",
                div {
                    class: "alert alert-error",
                    role: "alert",
                    div {
                        p { "Leo is unavailable: {message}" }
                        p {
                            "Check the camera configuration, data and logging directories, and recorder executable, then restart Leo."
                        }
                    }
                }
            }
        },
    };

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        {body}
    }
}

#[component]
fn ReadyApp() -> Element {
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
        Router::<Route> {}
        {desktop_e2e}
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use backend::{
        recording::{RecorderEvent, RecorderSettings, RecorderStatus, spawn_for_test},
        session::SessionController,
    };

    use super::{
        RecorderBootstrap, configuration_value_is_present, initialize_workflow, model_config_error,
        take_recorder_events,
    };
    use crate::{
        camera_config::{CameraConfig, StartupConfig},
        session_task::handle_recorder_event,
        workflow::{SessionRunState, Workflow},
    };

    fn camera_configs() -> Vec<CameraConfig> {
        [1_u32, 2]
            .into_iter()
            .map(|id| CameraConfig {
                id,
                name: format!("Salon {id}"),
                rtsp_url: format!("rtsp://camera-{id}.example/live"),
                enabled: true,
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
    fn model_configuration_availability_is_sanitized_without_environment_mutation() {
        let unavailable = Some("Analysis requires OPENAI_API_KEY and ANALYSIS_MODEL.".to_owned());

        assert_eq!(model_config_error(false, false), unavailable);
        assert_eq!(model_config_error(false, true), unavailable);
        assert_eq!(model_config_error(true, false), unavailable);
        assert_eq!(model_config_error(true, true), None);
    }

    #[test]
    fn model_configuration_requires_non_blank_values() {
        assert!(!configuration_value_is_present(None));
        assert!(!configuration_value_is_present(Some("")));
        assert!(!configuration_value_is_present(Some("  \t")));
        assert!(configuration_value_is_present(Some("configured")));
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
        let sessions_root = temporary.path().join("sessions");
        fs::write(&sessions_root, b"not a directory")
            .expect("catalogue root should become invalid after startup validation");
        let config = StartupConfig {
            cameras: camera_configs(),
            data_root: temporary.path().to_owned(),
            sessions_root,
            logs_root: temporary.path().join("logs"),
            recorder_settings: RecorderSettings {
                io_timeout: Duration::from_secs(1),
                retry_delay: Duration::from_secs(1),
                stop_timeout: Duration::from_secs(1),
            },
        };

        let Err(error) = initialize_workflow(&config, &recorder) else {
            panic!("invalid catalogue should make Workflow unavailable");
        };

        assert!(error.contains("Session workflow is unavailable"));
        assert!(take_recorder_events(&recorder).is_some());
        runtime.shutdown().expect("runtime should shut down");
    }

    #[test]
    fn root_event_dispatch_updates_reconnecting_and_claims_one_fatal_cleanup() {
        let (temporary, runtime, recorder) = test_recorder();
        let mut workflow = Workflow::new(
            camera_configs(),
            temporary.path().join("sessions"),
            recorder.handle.clone(),
            None,
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

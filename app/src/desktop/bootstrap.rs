use std::sync::{Arc, Mutex};

use backend::recording::{RecorderEvent, RecorderHandle, RecorderRuntime};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    Route,
    logging::{self, LogGuard},
    preview::{Bridge, CameraSource, PreviewState, bridge},
    settings::{LogLevel, ResolvedSettings, Settings as ApplicationSettings, SettingsStore},
    workflow::Workflow,
};

/// Recorder handle and single event receiver handed from desktop startup to the ready UI root.
#[derive(Clone)]
pub struct RecorderBootstrap {
    pub handle: RecorderHandle,
    pub events: Arc<Mutex<Option<UnboundedReceiver<RecorderEvent>>>>,
}

/// Single workflow value transferred into Dioxus's root scope after startup.
#[derive(Clone)]
pub struct InitialWorkflow(pub Arc<Mutex<Option<Workflow>>>);

/// Operational startup result kept separate from the always-available shell.
#[derive(Clone)]
pub enum Bootstrap {
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
pub struct InitialSettings {
    pub store: SettingsStore,
    pub draft: ApplicationSettings,
    pub initial_route: Route,
}

/// Process-owning resources that must outlive the desktop event loop.
pub struct RuntimeOwners {
    recorder: Option<RecorderRuntime>,
    preview: Option<Bridge>,
    log: Option<LogGuard>,
}

impl RuntimeOwners {
    fn new() -> Self {
        Self {
            recorder: None,
            preview: None,
            log: None,
        }
    }

    /// Stops child processes and flushes logging in ownership order.
    pub fn shutdown(&mut self) {
        if let Some(runtime) = self.recorder.take() {
            tracing::info!("recorder runtime stopping");
            if let Err(error) = runtime.shutdown() {
                tracing::error!(error = %error, "recorder runtime shutdown failed");
            } else {
                tracing::info!("recorder runtime stopped");
            }
        }
        if let Some(bridge) = self.preview.take() {
            tracing::info!("preview stopping");
            if let Err(error) = bridge.stop() {
                tracing::error!(error = %error, "preview stop failed");
            } else {
                tracing::info!("preview stopped");
            }
        }
        tracing::info!("application logging stopping");
        drop(self.log.take());
    }
}

/// Prepared shell state and the resources owned by the desktop event loop.
pub struct Startup {
    pub bootstrap: Bootstrap,
    pub settings: InitialSettings,
    pub owners: RuntimeOwners,
}

/// Loads settings and prepares every startup-only runtime dependency.
pub fn prepare(store: SettingsStore) -> Startup {
    let mut owners = RuntimeOwners::new();
    let loaded = store
        .load()
        .unwrap_or_else(|error| panic!("application settings could not be loaded: {error}"));
    let (bootstrap, settings) = match loaded {
        None => missing_settings(store),
        Some(config) => loaded_settings(store, config, &mut owners),
    };

    Startup {
        bootstrap,
        settings,
        owners,
    }
}

fn missing_settings(store: SettingsStore) -> (Bootstrap, InitialSettings) {
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

fn loaded_settings(
    store: SettingsStore,
    config: ResolvedSettings,
    owners: &mut RuntimeOwners,
) -> (Bootstrap, InitialSettings) {
    let settings = InitialSettings {
        store,
        draft: config.settings.clone(),
        initial_route: Route::Monitor {},
    };
    let bootstrap = prepare_runtime(config, owners);
    (bootstrap, settings)
}

fn prepare_runtime(config: ResolvedSettings, owners: &mut RuntimeOwners) -> Bootstrap {
    let log_level = config.settings.log_level;
    let log_guard = match logging::init(&config.logs_root, log_level) {
        Ok(log_guard) => log_guard,
        Err(_error) => {
            let _ = logging::init_stderr(log_level);
            tracing::error!("application logging initialization failed");
            return Bootstrap::Failed {
                message: "Application logging is unavailable.".into(),
            };
        }
    };
    owners.log = Some(log_guard);
    tracing::info!("startup configuration loaded");

    let (runtime, handle, events) = match RecorderRuntime::spawn(config.recorder_settings) {
        Ok(recorder) => recorder,
        Err(error) => {
            tracing::error!(error = %error, "recorder preflight failed");
            return Bootstrap::Failed {
                message: "Recorder preflight failed.".into(),
            };
        }
    };
    owners.recorder = Some(runtime);
    let recorder = RecorderBootstrap {
        handle,
        events: Arc::new(Mutex::new(Some(events))),
    };
    let workflow = match initialize_workflow(&config, &recorder) {
        Ok(workflow) => workflow,
        Err(message) => {
            tracing::error!("session workflow initialization failed");
            return Bootstrap::Failed { message };
        }
    };
    let (preview, bridge) = prepare_preview(&config);
    owners.preview = bridge;

    Bootstrap::Ready {
        config: Box::new(config),
        preview,
        recorder,
        workflow,
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
        config.settings.openai_config(),
        config.analysis_frame_sets_per_prompt,
        config.analysis_overlap_frame_sets,
    )
    .map(|workflow| InitialWorkflow(Arc::new(Mutex::new(Some(workflow)))))
    .map_err(|_| "Session workflow is unavailable.".into())
}

fn prepare_preview(config: &ResolvedSettings) -> (PreviewState, Option<Bridge>) {
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
    if sources.is_empty() {
        tracing::info!("preview skipped because no cameras are configured");
        return (PreviewState::NoCameras, None);
    }

    tracing::info!("preview startup requested");
    match bridge::start(sources) {
        Ok((state, bridge)) => {
            tracing::info!("preview ready");
            (state, Some(bridge))
        }
        Err(error) => {
            tracing::warn!(error = %error, "preview unavailable");
            (
                PreviewState::Unavailable {
                    message: error.to_string(),
                },
                None,
            )
        }
    }
}

#[cfg(all(test, unix))]
pub fn initialize_workflow_for_test(
    config: &ResolvedSettings,
    recorder: &RecorderBootstrap,
) -> Result<InitialWorkflow, String> {
    initialize_workflow(config, recorder)
}

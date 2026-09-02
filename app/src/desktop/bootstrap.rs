use std::sync::{Arc, Mutex};

use backend::recording::{RecorderEvent, RecorderHandle, RecorderRuntime};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    Route,
    logging::{self, LogGuard},
    operator::OperatorState,
    preview::{Bridge, CameraSource, PreviewState, bridge},
    settings::{LogLevel, ResolvedSettings, Settings as ApplicationSettings, SettingsStore},
};

/// Recorder handle and single event receiver handed from desktop startup to the ready UI root.
#[derive(Clone)]
pub struct RecorderBootstrap {
    pub handle: RecorderHandle,
    pub events: Arc<Mutex<Option<UnboundedReceiver<RecorderEvent>>>>,
}

/// Single operator-state value transferred into Dioxus's root scope after startup.
#[derive(Clone)]
pub struct InitialOperatorState(pub Arc<Mutex<Option<OperatorState>>>);

/// Operational startup result kept separate from the always-available shell.
#[derive(Clone)]
pub enum Bootstrap {
    Ready {
        config: Box<ResolvedSettings>,
        preview: PreviewState,
        recorder: RecorderBootstrap,
        operator: InitialOperatorState,
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

/// Process and logging guards retained by the native desktop event loop.
///
/// Dioxus's desktop launcher never returns, so normal stack cleanup is unreachable. The event
/// handler owns these guards and shuts them down explicitly when the native loop is destroyed.
pub struct DesktopRuntime {
    recorder: Option<RecorderRuntime>,
    preview: Option<Bridge>,
    log: Option<LogGuard>,
}

impl DesktopRuntime {
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
    pub runtime: DesktopRuntime,
}

/// Loads settings and prepares every startup-only runtime dependency.
pub fn prepare(store: SettingsStore) -> Startup {
    let mut runtime = DesktopRuntime::new();
    let loaded = store
        .load()
        .unwrap_or_else(|error| panic!("application settings could not be loaded: {error}"));
    let (bootstrap, settings) = match loaded {
        None => missing_settings(store),
        Some(config) => loaded_settings(store, config, &mut runtime),
    };

    Startup {
        bootstrap,
        settings,
        runtime,
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
    runtime: &mut DesktopRuntime,
) -> (Bootstrap, InitialSettings) {
    let settings = InitialSettings {
        store,
        draft: config.settings.clone(),
        initial_route: Route::Monitor {},
    };
    let bootstrap = prepare_runtime(config, runtime);
    (bootstrap, settings)
}

fn prepare_runtime(config: ResolvedSettings, runtime: &mut DesktopRuntime) -> Bootstrap {
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
    runtime.log = Some(log_guard);
    tracing::info!("startup configuration loaded");

    let (runtime_owner, handle, events) = match RecorderRuntime::spawn(config.recorder_settings) {
        Ok(recorder) => recorder,
        Err(error) => {
            tracing::error!(error = %error, "recorder preflight failed");
            return Bootstrap::Failed {
                message: "Recorder preflight failed.".into(),
            };
        }
    };
    runtime.recorder = Some(runtime_owner);
    let recorder = RecorderBootstrap {
        handle,
        events: Arc::new(Mutex::new(Some(events))),
    };
    let operator = match initialize_operator(&config, &recorder) {
        Ok(operator) => operator,
        Err(message) => {
            tracing::error!("operator state initialization failed");
            return Bootstrap::Failed { message };
        }
    };
    let (preview, bridge) = prepare_preview(&config);
    runtime.preview = bridge;

    Bootstrap::Ready {
        config: Box::new(config),
        preview,
        recorder,
        operator,
    }
}

fn initialize_operator(
    config: &ResolvedSettings,
    recorder: &RecorderBootstrap,
) -> Result<InitialOperatorState, String> {
    OperatorState::new(
        config.settings.cameras.clone(),
        config.sessions_root.clone(),
        recorder.handle.clone(),
        config.settings.openai_config(),
        config.analysis_frame_sets_per_prompt,
        config.analysis_overlap_frame_sets,
    )
    .map(|operator| InitialOperatorState(Arc::new(Mutex::new(Some(operator)))))
    .map_err(|_| "Operator state is unavailable.".into())
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

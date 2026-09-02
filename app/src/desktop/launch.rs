use dioxus::desktop::{Config, tao::event::Event};

use super::{
    bootstrap::{Bootstrap, DesktopRuntime, Startup, prepare},
    shell::App,
};
use crate::settings::SettingsStore;

/// Launches the production desktop app from the platform settings location.
pub fn launch() {
    launch_with_store(SettingsStore::platform());
}

/// Launches the desktop shell with an explicit settings store.
pub fn launch_with_store(store: SettingsStore) {
    let Startup {
        bootstrap,
        settings,
        runtime,
    } = prepare(store);
    let desktop = desktop_config(runtime);
    let launcher = dioxus::LaunchBuilder::desktop();
    let launcher = match &bootstrap {
        Bootstrap::Ready {
            config,
            preview,
            recorder,
            operator,
        } => launcher
            .with_context(config.as_ref().clone())
            .with_context(preview.clone())
            .with_context(recorder.clone())
            .with_context(operator.clone()),
        Bootstrap::SetupRequired | Bootstrap::Failed { .. } => launcher,
    };
    launcher
        .with_context(settings)
        .with_context(bootstrap)
        .with_cfg(desktop)
        .launch(App);
}

fn desktop_config(mut runtime: DesktopRuntime) -> Config {
    Config::new().with_custom_event_handler(move |event, _| {
        if matches!(event, Event::LoopDestroyed) {
            runtime.shutdown();
        }
    })
}

//! Isolated real-WKWebView playback acceptance, without model calls or production settings.
use backend::analysis::{PlaybackClip, build_evidence, prepare_playback};
use base64::{Engine, engine::general_purpose::STANDARD};
use dioxus::prelude::*;
use std::path::PathBuf;

#[derive(Clone)]
struct ProbeInput {
    clip: Option<PlaybackClip>,
    path: PathBuf,
    initial_seek_ms: u64,
    result: PathBuf,
}

/// Opens a real desktop video element, verifies decoded playback and a seek, then writes JSON.
pub fn launch_playback_probe(directory: PathBuf, result: PathBuf) {
    let session =
        backend::session::Session::load(&directory.join("events.jsonl")).expect("fixture session");
    let ids = session.cameras.iter().map(|c| c.id).collect::<Vec<_>>();
    let segments = backend::recording::list_segments(&directory.join("recordings"), &ids)
        .expect("fixture segments");
    let evidence = build_evidence(&directory, &session, &segments).expect("fixture evidence");
    let frame = evidence
        .frames
        .iter()
        .find(|f| f.session_offset_ms >= 2000)
        .unwrap_or(&evidence.frames[0]);
    let cache = result
        .parent()
        .expect("result parent")
        .join("playback-cache");
    let clip = prepare_playback(&directory, &evidence, &frame.id, 2000, 8000, &cache)
        .expect("playback conversion");
    launch_probe(ProbeInput {
        path: clip.path.clone(),
        initial_seek_ms: clip.citation_offset_ms,
        clip: Some(clip),
        result,
    });
}
/// Checks one already generated playback derivative, without re-encoding or changing its bytes.
pub fn launch_clip_playback_probe(path: PathBuf, result: PathBuf, initial_seek_ms: u64) {
    launch_probe(ProbeInput {
        clip: None,
        path,
        initial_seek_ms,
        result,
    });
}

fn launch_probe(input: ProbeInput) {
    dioxus::LaunchBuilder::desktop()
        .with_context(input)
        .launch(Probe);
}

#[component]
fn Probe() -> Element {
    super::use_media_registry();
    rsx! {
        ProbePlayer {}
    }
}
#[component]
fn ProbePlayer() -> Element {
    let config = use_context::<ProbeInput>();
    let registry = use_context::<super::MediaRegistry>();
    let url = use_hook(|| {
        registry
            .register(&config.path)
            .expect("registered playback")
    });
    let result = config.result.clone();
    let clip = config.clip.clone();
    let path = config.path.clone();
    let initial_seek_ms = config.initial_seek_ms;
    use_effect(move || {
        let result = result.clone();
        let clip = clip.clone();
        let path = path.clone();
        let desktop = dioxus::desktop::window();
        spawn(async move {
            let script = format!(
                "{}\n{}",
                include_str!("pixel_probe.js"),
                include_str!("probe_driver.js")
            )
            .replace(
                "__INITIAL_SEEK_SECONDS__",
                &(initial_seek_ms as f64 / 1000.0).to_string(),
            );
            let mut eval = document::eval(&script);
            let mut outcome = eval
                .recv::<serde_json::Value>()
                .await
                .unwrap_or_else(|error| serde_json::json!({"ok":false,"error":error.to_string()}));
            for stage in ["seeked", "failure"] {
                let Some(observation) = outcome["observations"][stage].as_object_mut() else {
                    continue;
                };
                if let Some(png) = observation.remove("framePng") {
                    let image_path = result.with_extension(format!("{stage}.png"));
                    let bytes = png
                        .as_str()
                        .and_then(|value| value.strip_prefix("data:image/png;base64,"))
                        .and_then(|value| STANDARD.decode(value).ok());
                    if let Some(bytes) = bytes {
                        if std::fs::write(&image_path, bytes).is_ok() {
                            observation.insert(
                                "decodedFrameArtifact".into(),
                                serde_json::json!(image_path),
                            );
                        }
                    }
                }
            }
            let report = serde_json::json!({
                "webview":outcome,"clip":clip,"sourcePath":path,"initialSeekMs":initial_seek_ms,
                "scope":"Decoded pixels through production MediaRegistry in an isolated WKWebView; does not prove on-screen composition or screenshot capture."
            });
            std::fs::write(&result, serde_json::to_vec_pretty(&report).unwrap())
                .expect("write playback diagnostic");
            desktop.close();
        });
    });
    rsx! {
        main {
            h1 { "Recording playback check" }
            video {
                id: "probe-video",
                src: url,
                controls: true,
                preload: "metadata",
                style: "width:100%;max-height:384px;background:black",
                onloadedmetadata: move |_| {
                    let script = format!(
                        "document.getElementById('probe-video').currentTime = {};",
                        initial_seek_ms as f64 / 1000.0,
                    );
                    spawn(async move {
                        let _ = document::eval(&script).await;
                    });
                },
            }
        }
    }
}

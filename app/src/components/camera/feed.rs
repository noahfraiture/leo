use backend::recording::RecorderStatus;
use dioxus::prelude::*;

use crate::preview::PreviewFeed;

const READER_PROGRAM: &str = r#"
const config = await dioxus.recv();
const deadline = Date.now() + 5000;
while (!window.MediaMTXWebRTCReader && Date.now() < deadline) {
  await new Promise((resolve) => setTimeout(resolve, 50));
}
if (!window.MediaMTXWebRTCReader) {
  dioxus.send("MediaMTX reader failed to load");
  return;
}
const video = document.getElementById(config.video_id);
const reader = new MediaMTXWebRTCReader({
  url: config.whep_url,
  onError: (error) => dioxus.send(error),
  onTrack: (event) => {
    video.srcObject = event.streams[0];
    dioxus.send(null);
  },
});
await dioxus.recv();
reader.close();
video.srcObject = null;
"#;

/// Renders one stable live preview with independent analysis and recorder status.
#[component]
pub fn CameraFeed(
    feed: PreviewFeed,
    selected: bool,
    participating: bool,
    recorder_status: RecorderStatus,
    on_select: EventHandler<u32>,
) -> Element {
    let mut error = use_signal(|| None::<String>);
    let mut player_channel = use_signal(|| None::<document::Eval>);
    let config = serde_json::json!({
        "video_id": feed.video_id.clone(),
        "whep_url": feed.whep_url.clone(),
    });

    use_effect(move || {
        let mut reader = document::eval(READER_PROGRAM);
        if let Err(send_error) = reader.send(&config) {
            error.set(Some(format!("Failed to start live preview: {send_error}")));
            return;
        }
        player_channel.set(Some(reader));
        spawn(async move {
            while let Ok(status) = reader.recv::<Option<String>>().await {
                error.set(status);
            }
        });
    });

    use_drop(move || {
        if let Some(eval) = player_channel.peek().as_ref() {
            let _ = eval.send(());
        }
    });

    let participation = if participating {
        "Included"
    } else {
        "Excluded"
    };
    let recorder_status = recorder_status_label(recorder_status);
    let selection_label = format!(
        "{} {}",
        if selected { "Selected" } else { "Select" },
        feed.name
    );
    let camera_id = feed.camera_id;

    rsx! {
        article {
            class: if selected {
                "card w-full border border-primary bg-base-100"
            } else {
                "card w-full border border-base-300 bg-base-100"
            },

            figure {
                class: "relative aspect-video bg-base-200",

                video {
                    id: feed.video_id.clone(),
                    class: "h-full w-full object-cover",
                    autoplay: true,
                    muted: true,
                    playsinline: true,
                }

                if let Some(message) = error() {
                    p {
                        class: "absolute inset-0 flex items-center justify-center bg-base-300/90 p-4 text-center",
                        role: "alert",
                        aria_live: "assertive",
                        "{message}"
                    }
                }
            }

            div {
                class: "card-body gap-3 p-4",

                div {
                    class: "flex flex-wrap items-center justify-between gap-2",

                    h2 {
                        class: "card-title",
                        "{feed.name}"
                    }

                    button {
                        class: "btn btn-sm",
                        r#type: "button",
                        aria_label: selection_label,
                        aria_pressed: selected,
                        onclick: move |_| on_select.call(camera_id),
                        if selected { "Selected" } else { "Select" }
                    }
                }

                div {
                    class: "flex flex-wrap gap-2",
                    span {
                        class: "badge badge-outline",
                        aria_label: "Analysis participation: {participation}",
                        "{participation}"
                    }
                    span {
                        class: "badge badge-outline",
                        aria_label: "Recorder status: {recorder_status}",
                        role: "status",
                        aria_live: "polite",
                        "{recorder_status}"
                    }
                }
            }
        }
    }
}

fn recorder_status_label(status: RecorderStatus) -> &'static str {
    match status {
        RecorderStatus::Starting => "Starting",
        RecorderStatus::Recording => "Recording",
        RecorderStatus::Reconnecting => "Reconnecting",
        RecorderStatus::Stopped => "Idle",
    }
}

#[cfg(test)]
#[path = "tests/feed.rs"]
mod tests;

use dioxus::prelude::*;

use crate::preview::{PreviewFeed, ReaderConfig};

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
  user: config.user,
  pass: config.password,
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

#[component]
pub fn CameraFeed(feed: PreviewFeed, reader: ReaderConfig) -> Element {
    let mut error = use_signal(|| None::<String>);
    let mut player_channel = use_signal(|| None::<document::Eval>);
    let script_url = reader.script_url.clone();
    let config = serde_json::json!({
        "video_id": feed.video_id.clone(),
        "whep_url": feed.whep_url.clone(),
        "user": reader.user,
        "password": reader.password,
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

    rsx! {
        document::Script { src: script_url }

        div {
            class: "card card-border bg-base-100 w-full",

            figure {
                class: "relative aspect-video bg-base-200",

                video {
                    id: feed.video_id.clone(),
                    class: "h-full w-full object-cover",
                    autoplay: true,
                    muted: true,
                    playsinline: true,
                }

                div {
                    class: "absolute inset-4 flex flex-col justify-between",

                    div {
                        class: "flex justify-between",

                        div {
                            class: "badge badge-outline",
                            span { class: "status status-success" }
                            "LIVE"
                        }

                        span { "14:42:18" }
                    }

                    div {
                        class: "badge badge-primary badge-outline",
                        "Selected"
                    }
                }

                if let Some(message) = error() {
                    p {
                        class: "absolute inset-0 flex items-center justify-center bg-base-300/90 p-4 text-center",
                        role: "status",
                        "{message}"
                    }
                }
            }

            div {
                class: "card-body",

                div {
                    class: "flex items-center justify-between",

                    h2 {
                        class: "card-title",
                        span { class: "status status-success" }
                        "{feed.name}"
                    }

                    div {
                        class: "card-actions",
                        button {
                            class: "btn btn-ghost btn-circle btn-sm",
                            aria_label: "Camera options",
                            "..."
                        }
                    }
                }

                p { "CAM 04 - Selected camera" }
            }
        }
    }
}

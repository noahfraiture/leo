use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq)]
pub(crate) struct CameraSource {
    pub name: String,
    pub rtsp_url: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PreviewFeed {
    pub name: String,
    pub video_id: String,
    pub whep_url: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReaderConfig {
    pub script_url: String,
    pub user: String,
    pub password: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum PreviewState {
    Ready {
        feeds: Vec<PreviewFeed>,
        reader: ReaderConfig,
    },
    Unavailable {
        message: String,
    },
}

pub(crate) fn preview_metadata(sources: &[CameraSource], password: String) -> PreviewState {
    let feeds = sources
        .iter()
        .enumerate()
        .map(|(index, source)| PreviewFeed {
            name: source.name.clone(),
            video_id: format!("camera-{index}-video"),
            whep_url: format!("http://127.0.0.1:8889/camera-{index}/whep"),
        })
        .collect();
    let reader = ReaderConfig {
        script_url: "http://127.0.0.1:8889/reader.js".into(),
        user: "app-preview".into(),
        password,
    };

    PreviewState::Ready { feeds, reader }
}

#[cfg(test)]
mod tests {
    use crate::preview::{CameraSource, preview_metadata};

    #[test]
    fn metadata_does_not_expose_camera_credentials() {
        let source = CameraSource {
            name: "Workshop".into(),
            rtsp_url: "rtsp://camera-user:camera-pass@127.0.0.1/live".into(),
        };
        let preview = preview_metadata(&[source], "local-password".into());
        let serialized = serde_json::to_string(&preview).unwrap();

        assert!(serialized.contains("camera-0-video"));
        assert!(serialized.contains("http://127.0.0.1:8889/camera-0/whep"));
        assert!(!serialized.contains("camera-user"));
        assert!(!serialized.contains("camera-pass"));
        assert!(!serialized.contains("rtsp://"));
    }
}

use super::MediaRegistry;
use dioxus::desktop::wry::http::{Request, Response, StatusCode};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

/// Serves a registered MP4, honoring byte ranges used by WebKit for metadata reads and seeking.
pub fn respond(registry: &MediaRegistry, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let id = request
        .uri()
        .path()
        .strip_prefix("/leo-media/")
        .unwrap_or("");
    let path = registry.0.lock().ok().and_then(|r| r.get(id).cloned());
    let Some(path) = path else {
        return empty(StatusCode::NOT_FOUND);
    };
    if request.method() != "GET" && request.method() != "HEAD" {
        return empty(StatusCode::METHOD_NOT_ALLOWED);
    }
    let Ok(mut file) = File::open(path) else {
        return empty(StatusCode::NOT_FOUND);
    };
    let Ok(metadata) = file.metadata() else {
        return empty(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let len = metadata.len();
    if len == 0 || len > 64 * 1024 * 1024 {
        return empty(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let range = request.headers().get("range").and_then(|v| v.to_str().ok());
    let (start, end) = if let Some(range) = range {
        match parse_range(range, len) {
            Some(pair) => pair,
            None => {
                return Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header("Content-Range", format!("bytes */{len}"))
                    .body(vec![])
                    .unwrap();
            }
        }
    } else {
        (0, len - 1)
    };
    let mut builder = Response::builder()
        .status(if range.is_some() {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header("Content-Type", "video/mp4")
        .header("Accept-Ranges", "bytes")
        .header("Cache-Control", "private, max-age=3600")
        .header("Content-Length", end - start + 1)
        .header("Access-Control-Allow-Origin", "*");
    if range.is_some() {
        builder = builder.header("Content-Range", format!("bytes {start}-{end}/{len}"));
    }
    let mut bytes = Vec::new();
    if request.method() != "HEAD" {
        if file.seek(SeekFrom::Start(start)).is_err()
            || file.take(end - start + 1).read_to_end(&mut bytes).is_err()
        {
            return empty(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
    builder.body(bytes).unwrap()
}
fn empty(status: StatusCode) -> Response<Vec<u8>> {
    Response::builder().status(status).body(vec![]).unwrap()
}
fn parse_range(value: &str, len: u64) -> Option<(u64, u64)> {
    let value = value.strip_prefix("bytes=")?;
    if value.contains(',') {
        return None;
    }
    let (start, end) = value.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?;
        return (suffix > 0).then_some((len.saturating_sub(suffix), len - 1));
    }
    let start = start.parse::<u64>().ok()?;
    let end = if end.is_empty() {
        len - 1
    } else {
        end.parse::<u64>().ok()?.min(len - 1)
    };
    (start < len && start <= end).then_some((start, end))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn media() -> (tempfile::TempDir, MediaRegistry, String) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("clip.mp4");
        std::fs::write(&file, b"0123456789").unwrap();
        let registry = MediaRegistry::default();
        let url = registry.register(&file).unwrap();
        (dir, registry, url)
    }
    #[test]
    fn serves_requested_bytes_of_registered_media() {
        let (_dir, registry, url) = media();
        let response = respond(
            &registry,
            Request::builder()
                .uri(&url)
                .header("Range", "bytes=4-7")
                .body(vec![])
                .unwrap(),
        );
        assert_eq!(response.status(), 206);
        assert_eq!(response.headers()["content-range"], "bytes 4-7/10");
        assert_eq!(response.body(), b"4567");
    }
    #[test]
    fn unknown_ids_cannot_open_arbitrary_paths() {
        let response = respond(
            &MediaRegistry::default(),
            Request::builder()
                .uri("/leo-media/../../settings.json")
                .body(vec![])
                .unwrap(),
        );
        assert_eq!(response.status(), 404);
    }
    #[test]
    fn head_returns_metadata_without_a_body() {
        let (_dir, registry, url) = media();
        let response = respond(
            &registry,
            Request::builder()
                .method("HEAD")
                .uri(&url)
                .body(vec![])
                .unwrap(),
        );
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-length"], "10");
        assert_eq!(response.headers()["accept-ranges"], "bytes");
        assert!(response.body().is_empty());
    }
    #[test]
    fn range_boundaries_and_suffixes() {
        for (input, expected) in [
            ("bytes=-2", Some((8, 9))),
            ("bytes=10-", None),
            ("bytes=0-2,5-7", None),
            ("bytes=7-", Some((7, 9))),
            ("bytes=7-100", Some((7, 9))),
            ("bytes=-0", None),
            ("bytes=8-7", None),
        ] {
            assert_eq!(parse_range(input, 10), expected, "{input}");
        }
    }
}

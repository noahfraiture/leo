use std::{
    collections::HashSet,
    ffi::OsStr,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::camera::Camera;

/// A fixture-backed recording exposed by the simulated Recording API.
#[derive(Clone)]
pub(crate) struct Recording {
    /// Stable recording identifier used by List and Download.
    pub id: u32,
    /// Configured camera that owns this recording.
    pub camera_id: u32,
    /// Synology server identifier exposed by the API.
    pub ds_id: u32,
    /// Mounted archive identifier exposed by the API.
    pub mount_id: u32,
    /// Inclusive recording start as UTC Unix seconds.
    pub start_time: u64,
    /// Exclusive recording end as UTC Unix seconds.
    pub stop_time: u64,
    /// Logical Synology path returned to clients.
    pub file_path: String,
    /// Private local fixture path used to serve media.
    pub video_path: PathBuf,
    /// Numeric video codec identifier from the documented v6 schema.
    pub video_codec: u8,
    /// Numeric audio codec identifier from the documented v6 schema.
    pub audio_codec: u8,
    /// Encoded video width in pixels.
    pub width: u32,
    /// Encoded video height in pixels.
    pub height: u32,
    /// Fixture media size in bytes.
    pub size_byte: u64,
    /// Whether the simulated recording is locked against deletion.
    pub locked: bool,
}

/// One unvalidated row from the simulator's recording catalogue file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FixtureRecording {
    id: u32,
    camera_id: u32,
    ds_id: u32,
    mount_id: u32,
    start_time: u64,
    stop_time: u64,
    file_path: String,
    video: PathBuf,
    video_codec: u8,
    audio_codec: u8,
    width: u32,
    height: u32,
    locked: bool,
}

/// Failures that prevent a fixture catalogue from becoming runtime state.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("failed to read recording catalogue {path:?}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse recording catalogue {path:?}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("recording ID must be non-zero")]
    ZeroId,
    #[error("duplicate recording ID {0}")]
    DuplicateId(u32),
    #[error("recording {id} references unknown camera {camera_id}")]
    UnknownCamera { id: u32, camera_id: u32 },
    #[error("recording {id} stop time must be after its start time")]
    InvalidTimeRange { id: u32 },
    #[error("recording {id} width and height must be non-zero")]
    ZeroDimensions { id: u32 },
    #[error("recording {id} has invalid video codec {codec}")]
    InvalidVideoCodec { id: u32, codec: u8 },
    #[error("recording {id} has invalid audio codec {codec}")]
    InvalidAudioCodec { id: u32, codec: u8 },
    #[error("failed to resolve recording {id} media {path:?}")]
    Media {
        id: u32,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("recording {id} media is not a regular file: {path:?}")]
    MediaNotFile { id: u32, path: PathBuf },
    #[error("recording {id} media is not an MP4 file: {path:?}")]
    MediaNotMp4 { id: u32, path: PathBuf },
}

type Result<T> = std::result::Result<T, Error>;

/// Loads and validates fixture recordings before attaching them to their cameras.
pub(crate) fn load_catalogue(path: &Path, cameras: &mut [Camera]) -> Result<()> {
    let contents = fs::read(path).map_err(|source| Error::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let rows: Vec<FixtureRecording> =
        serde_json::from_slice(&contents).map_err(|source| Error::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    let directory = path.parent().unwrap_or(Path::new("."));
    let mut ids = HashSet::with_capacity(rows.len());
    let mut recordings = Vec::with_capacity(rows.len());

    for row in rows {
        if row.id == 0 {
            return Err(Error::ZeroId);
        }
        if !ids.insert(row.id) {
            return Err(Error::DuplicateId(row.id));
        }
        if !cameras.iter().any(|camera| camera.id == row.camera_id) {
            return Err(Error::UnknownCamera {
                id: row.id,
                camera_id: row.camera_id,
            });
        }
        if row.stop_time <= row.start_time {
            return Err(Error::InvalidTimeRange { id: row.id });
        }
        if row.width == 0 || row.height == 0 {
            return Err(Error::ZeroDimensions { id: row.id });
        }
        if !matches!(row.video_codec, 0 | 1 | 2 | 3 | 5 | 6 | 7) {
            return Err(Error::InvalidVideoCodec {
                id: row.id,
                codec: row.video_codec,
            });
        }
        if !matches!(row.audio_codec, 0..=6) {
            return Err(Error::InvalidAudioCodec {
                id: row.id,
                codec: row.audio_codec,
            });
        }

        let unresolved_video = directory.join(&row.video);
        let video_path = unresolved_video
            .canonicalize()
            .map_err(|source| Error::Media {
                id: row.id,
                path: unresolved_video.clone(),
                source,
            })?;
        let metadata = video_path.metadata().map_err(|source| Error::Media {
            id: row.id,
            path: video_path.clone(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(Error::MediaNotFile {
                id: row.id,
                path: video_path,
            });
        }
        if unresolved_video.extension() != Some(OsStr::new("mp4")) {
            return Err(Error::MediaNotMp4 {
                id: row.id,
                path: unresolved_video,
            });
        }
        let mut header = Vec::with_capacity(8);
        fs::File::open(&video_path)
            .and_then(|file| file.take(8).read_to_end(&mut header))
            .map_err(|source| Error::Media {
                id: row.id,
                path: video_path.clone(),
                source,
            })?;
        if header.get(4..8) != Some(&b"ftyp"[..]) {
            return Err(Error::MediaNotMp4 {
                id: row.id,
                path: unresolved_video,
            });
        }

        recordings.push(Recording {
            id: row.id,
            camera_id: row.camera_id,
            ds_id: row.ds_id,
            mount_id: row.mount_id,
            start_time: row.start_time,
            stop_time: row.stop_time,
            file_path: row.file_path,
            video_path,
            video_codec: row.video_codec,
            audio_codec: row.audio_codec,
            width: row.width,
            height: row.height,
            size_byte: metadata.len(),
            locked: row.locked,
        });
    }

    recordings.sort_by_key(|recording| (recording.start_time, recording.id));
    for recording in recordings {
        cameras
            .iter_mut()
            .find(|camera| camera.id == recording.camera_id)
            .expect("camera IDs were validated")
            .recordings
            .push(recording);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, net::SocketAddr, path::Path};

    use serde_json::{Value, json};

    use super::{Error, load_catalogue};
    use crate::camera::Camera;

    fn camera(index: usize) -> Camera {
        Camera::new(
            index,
            SocketAddr::from(([127, 0, 0, 1], 8001 + index as u16)),
        )
    }

    fn valid_row(video: &str) -> Value {
        json!({
            "id": 1,
            "cameraId": 1,
            "dsId": 0,
            "mountId": 0,
            "startTime": 1786147200_u64,
            "stopTime": 1786147205_u64,
            "filePath": "20260808AM/camera-1-1786147200.mp4",
            "video": video,
            "videoCodec": 3,
            "audioCodec": 0,
            "width": 1280,
            "height": 720,
            "locked": false
        })
    }

    fn write_catalogue(directory: &Path, rows: &[Value]) -> std::path::PathBuf {
        fs::create_dir_all(directory).unwrap();
        let path = directory.join("recordings.json");
        fs::write(&path, serde_json::to_vec(rows).unwrap()).unwrap();
        path
    }

    fn write_mp4(path: &Path) {
        fs::write(path, b"\0\0\0\x08ftyp").unwrap();
    }

    #[test]
    fn loads_recording_and_resolves_video_relative_to_catalogue() {
        let directory = tempfile::tempdir().unwrap();
        let catalogue_directory = directory.path().join("catalogue");
        let video = directory.path().join("media/video.mp4");
        fs::create_dir_all(video.parent().unwrap()).unwrap();
        write_mp4(&video);
        let path = write_catalogue(&catalogue_directory, &[valid_row("../media/video.mp4")]);
        let mut cameras = [camera(0)];

        load_catalogue(&path, &mut cameras).unwrap();

        let recording = &cameras[0].recordings[0];
        assert_eq!(recording.id, 1);
        assert_eq!(recording.camera_id, 1);
        assert_eq!(recording.ds_id, 0);
        assert_eq!(recording.mount_id, 0);
        assert_eq!(recording.start_time, 1786147200);
        assert_eq!(recording.stop_time, 1786147205);
        assert_eq!(recording.file_path, "20260808AM/camera-1-1786147200.mp4");
        assert_eq!(recording.video_path, video.canonicalize().unwrap());
        assert_eq!(recording.video_codec, 3);
        assert_eq!(recording.audio_codec, 0);
        assert_eq!(recording.width, 1280);
        assert_eq!(recording.height, 720);
        assert!(!recording.locked);
    }

    #[test]
    fn rejects_unknown_json_fields() {
        let directory = tempfile::tempdir().unwrap();
        write_mp4(&directory.path().join("video.mp4"));
        let mut row = valid_row("video.mp4");
        row["unexpected"] = json!(true);
        let path = write_catalogue(directory.path(), &[row]);

        assert!(load_catalogue(&path, &mut [camera(0)]).is_err());
    }

    #[test]
    fn rejects_zero_recording_id() {
        let directory = tempfile::tempdir().unwrap();
        write_mp4(&directory.path().join("video.mp4"));
        let mut row = valid_row("video.mp4");
        row["id"] = json!(0);
        let path = write_catalogue(directory.path(), &[row]);

        assert!(load_catalogue(&path, &mut [camera(0)]).is_err());
    }

    #[test]
    fn rejects_duplicate_recording_ids() {
        let directory = tempfile::tempdir().unwrap();
        write_mp4(&directory.path().join("video.mp4"));
        let row = valid_row("video.mp4");
        let path = write_catalogue(directory.path(), &[row.clone(), row]);

        assert!(load_catalogue(&path, &mut [camera(0)]).is_err());
    }

    #[test]
    fn rejects_unknown_camera_id() {
        let directory = tempfile::tempdir().unwrap();
        write_mp4(&directory.path().join("video.mp4"));
        let mut row = valid_row("video.mp4");
        row["cameraId"] = json!(2);
        let path = write_catalogue(directory.path(), &[row]);

        assert!(load_catalogue(&path, &mut [camera(0)]).is_err());
    }

    #[test]
    fn rejects_invalid_time_ranges() {
        let directory = tempfile::tempdir().unwrap();
        write_mp4(&directory.path().join("video.mp4"));

        for (start, stop) in [(10, 10), (11, 10)] {
            let mut row = valid_row("video.mp4");
            row["startTime"] = json!(start);
            row["stopTime"] = json!(stop);
            let path = write_catalogue(directory.path(), &[row]);

            assert!(
                load_catalogue(&path, &mut [camera(0)]).is_err(),
                "accepted {start}..{stop}"
            );
        }
    }

    #[test]
    fn rejects_missing_media() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_catalogue(directory.path(), &[valid_row("missing.mp4")]);

        assert!(load_catalogue(&path, &mut [camera(0)]).is_err());
    }

    #[test]
    fn rejects_non_file_media() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("video.mp4")).unwrap();
        let path = write_catalogue(directory.path(), &[valid_row("video.mp4")]);

        assert!(load_catalogue(&path, &mut [camera(0)]).is_err());
    }

    #[test]
    fn rejects_non_mp4_media() {
        let directory = tempfile::tempdir().unwrap();
        write_mp4(&directory.path().join("video.mov"));
        let path = write_catalogue(directory.path(), &[valid_row("video.mov")]);

        assert!(load_catalogue(&path, &mut [camera(0)]).is_err());
    }

    #[test]
    fn rejects_empty_mp4_media() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("video.mp4"), []).unwrap();
        let path = write_catalogue(directory.path(), &[valid_row("video.mp4")]);

        assert!(matches!(
            load_catalogue(&path, &mut [camera(0)]),
            Err(Error::MediaNotMp4 { id: 1, .. })
        ));
    }

    #[test]
    fn rejects_mp4_without_ftyp_box() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("video.mp4"), b"not an mp4").unwrap();
        let path = write_catalogue(directory.path(), &[valid_row("video.mp4")]);

        assert!(matches!(
            load_catalogue(&path, &mut [camera(0)]),
            Err(Error::MediaNotMp4 { id: 1, .. })
        ));
    }

    #[test]
    fn rejects_zero_dimensions() {
        let directory = tempfile::tempdir().unwrap();
        write_mp4(&directory.path().join("video.mp4"));

        for field in ["width", "height"] {
            let mut row = valid_row("video.mp4");
            row[field] = json!(0);
            let path = write_catalogue(directory.path(), &[row]);

            assert!(
                load_catalogue(&path, &mut [camera(0)]).is_err(),
                "accepted zero {field}"
            );
        }
    }

    #[test]
    fn rejects_invalid_video_codec() {
        let directory = tempfile::tempdir().unwrap();
        write_mp4(&directory.path().join("video.mp4"));
        let mut row = valid_row("video.mp4");
        row["videoCodec"] = json!(4);
        let path = write_catalogue(directory.path(), &[row]);

        assert!(matches!(
            load_catalogue(&path, &mut [camera(0)]),
            Err(Error::InvalidVideoCodec { id: 1, codec: 4 })
        ));
    }

    #[test]
    fn rejects_invalid_audio_codec() {
        let directory = tempfile::tempdir().unwrap();
        write_mp4(&directory.path().join("video.mp4"));
        let mut row = valid_row("video.mp4");
        row["audioCodec"] = json!(7);
        let path = write_catalogue(directory.path(), &[row]);

        assert!(matches!(
            load_catalogue(&path, &mut [camera(0)]),
            Err(Error::InvalidAudioCodec { id: 1, codec: 7 })
        ));
    }

    #[test]
    fn uses_actual_media_size() {
        let directory = tempfile::tempdir().unwrap();
        write_mp4(&directory.path().join("video.mp4"));
        let path = write_catalogue(directory.path(), &[valid_row("video.mp4")]);
        let mut cameras = [camera(0)];

        load_catalogue(&path, &mut cameras).unwrap();

        assert_eq!(cameras[0].recordings[0].size_byte, 8);
    }

    #[test]
    fn sorts_recordings_by_start_time_then_id() {
        let directory = tempfile::tempdir().unwrap();
        write_mp4(&directory.path().join("video.mp4"));
        let mut first = valid_row("video.mp4");
        first["id"] = json!(1);
        first["startTime"] = json!(20);
        first["stopTime"] = json!(21);
        let mut second = valid_row("video.mp4");
        second["id"] = json!(3);
        second["startTime"] = json!(10);
        second["stopTime"] = json!(11);
        let mut third = valid_row("video.mp4");
        third["id"] = json!(2);
        third["startTime"] = json!(10);
        third["stopTime"] = json!(11);
        let path = write_catalogue(directory.path(), &[first, second, third]);
        let mut cameras = [camera(0)];

        load_catalogue(&path, &mut cameras).unwrap();

        assert_eq!(
            cameras[0]
                .recordings
                .iter()
                .map(|recording| recording.id)
                .collect::<Vec<_>>(),
            [2, 3, 1]
        );
    }
}

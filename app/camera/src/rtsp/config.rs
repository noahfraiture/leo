use std::{io::Write, net::SocketAddr, path::Path};

use tempfile::NamedTempFile;

use crate::rtsp::Error;

#[derive(Debug)]
pub(super) struct ConfigFile(NamedTempFile);

impl ConfigFile {
    pub(super) fn create(address: SocketAddr, video: &Path) -> Result<Self, Error> {
        if video.to_str().is_none() {
            return Err(Error::FixtureNotUtf8(video.to_path_buf()));
        }
        let canonical = video.canonicalize().map_err(|source| Error::Fixture {
            path: video.to_path_buf(),
            source,
        })?;
        if !canonical.is_file() {
            return Err(Error::FixtureNotFile(canonical));
        }
        let video = canonical
            .to_str()
            .ok_or_else(|| Error::FixtureNotUtf8(canonical.clone()))?;
        let contents = render(address, video)?;
        let mut file = NamedTempFile::new().map_err(Error::CreateConfig)?;
        file.write_all(contents.as_bytes())
            .and_then(|()| file.flush())
            .map_err(Error::WriteConfig)?;
        Ok(Self(file))
    }

    pub(super) fn path(&self) -> &Path {
        self.0.path()
    }
}

fn render(address: SocketAddr, video: &str) -> Result<String, Error> {
    let address = serde_json::to_string(&address.to_string()).map_err(Error::SerializeConfig)?;
    let video = serde_json::to_string(video).map_err(Error::SerializeConfig)?;

    Ok(format!(
        concat!(
            "logDestinations: [stdout]\n",
            "api: false\n",
            "metrics: false\n",
            "pprof: false\n",
            "playback: false\n",
            "rtsp: true\n",
            "rtspAddress: {address}\n",
            "rtspTransports: [tcp]\n",
            "rtmp: false\n",
            "hls: false\n",
            "webrtc: false\n",
            "srt: false\n",
            "authInternalUsers:\n",
            "  - user: any\n",
            "    pass:\n",
            "    ips: []\n",
            "    permissions:\n",
            "      - action: read\n",
            "paths:\n",
            "  axis-media/media.amp:\n",
            "    alwaysAvailable: true\n",
            "    alwaysAvailableFile: {video}\n",
        ),
        address = address,
        video = video,
    ))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        net::SocketAddr,
        path::{Path, PathBuf},
    };

    use super::ConfigFile;
    use crate::rtsp::Error;

    fn address() -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 8554))
    }

    fn create_video(path: &Path) {
        fs::write(path, b"video").unwrap();
    }

    #[test]
    fn rejects_missing_fixture() {
        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join("missing.mp4");

        let error = ConfigFile::create(address(), &video).unwrap_err();

        match error {
            Error::Fixture { path, source } => {
                assert_eq!(path, video);
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
            error => panic!("unexpected error: {error:?}"),
        }
    }

    #[test]
    fn rejects_directory_fixture() {
        let directory = tempfile::tempdir().unwrap();
        let canonical = directory.path().canonicalize().unwrap();

        let error = ConfigFile::create(address(), directory.path()).unwrap_err();

        match error {
            Error::FixtureNotFile(path) => assert_eq!(path, canonical),
            error => panic!("unexpected error: {error:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_fixture_path() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let directory = tempfile::tempdir().unwrap();
        let video = directory
            .path()
            .join(OsString::from_vec(b"video-\xff.mp4".to_vec()));

        let error = ConfigFile::create(address(), &video).unwrap_err();

        match error {
            Error::FixtureNotUtf8(path) => assert_eq!(path, video),
            error => panic!("unexpected error: {error:?}"),
        }
    }

    #[test]
    fn renders_exact_rtsp_only_configuration_with_quoted_values() {
        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join("video # \"quoted\" \\.mp4");
        create_video(&video);
        let canonical = video.canonicalize().unwrap();
        let address = "[::1]:8554".parse().unwrap();

        let config = ConfigFile::create(address, &video).unwrap();

        let expected = format!(
            concat!(
                "logDestinations: [stdout]\n",
                "api: false\n",
                "metrics: false\n",
                "pprof: false\n",
                "playback: false\n",
                "rtsp: true\n",
                "rtspAddress: {}\n",
                "rtspTransports: [tcp]\n",
                "rtmp: false\n",
                "hls: false\n",
                "webrtc: false\n",
                "srt: false\n",
                "authInternalUsers:\n",
                "  - user: any\n",
                "    pass:\n",
                "    ips: []\n",
                "    permissions:\n",
                "      - action: read\n",
                "paths:\n",
                "  axis-media/media.amp:\n",
                "    alwaysAvailable: true\n",
                "    alwaysAvailableFile: {}\n",
            ),
            serde_json::to_string(&address.to_string()).unwrap(),
            serde_json::to_string(canonical.to_str().unwrap()).unwrap()
        );
        assert_eq!(fs::read_to_string(config.path()).unwrap(), expected);
    }

    #[test]
    fn owns_temporary_file_until_drop() {
        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join("video.mp4");
        create_video(&video);

        let config = ConfigFile::create(address(), &video).unwrap();
        let path = PathBuf::from(config.path());
        assert!(path.exists());

        drop(config);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn temporary_file_mode_is_0600() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join("video.mp4");
        create_video(&video);

        let config = ConfigFile::create(address(), &video).unwrap();
        let mode = fs::metadata(config.path()).unwrap().permissions().mode();

        assert_eq!(mode & 0o777, 0o600);
    }
}

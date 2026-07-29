use std::{fmt::Write as _, io::Write as _, path::Path};

use tempfile::NamedTempFile;

use super::{CameraSource, Error};

pub(crate) struct ConfigFile(NamedTempFile);

impl ConfigFile {
    pub(crate) fn create(sources: &[CameraSource], password: &str) -> Result<Self, Error> {
        let contents = render(sources, password)?;
        let mut file = NamedTempFile::new().map_err(Error::CreateConfig)?;
        file.write_all(contents.as_bytes())
            .and_then(|()| file.flush())
            .map_err(Error::WriteConfig)?;
        Ok(Self(file))
    }

    pub(crate) fn path(&self) -> &Path {
        self.0.path()
    }
}

fn render(sources: &[CameraSource], password: &str) -> Result<String, Error> {
    let password = serde_json::to_string(password)?;
    let mut contents = format!(
        concat!(
            "logDestinations: [stdout]\n",
            "api: false\n",
            "metrics: false\n",
            "pprof: false\n",
            "playback: false\n",
            "rtsp: false\n",
            "rtmp: false\n",
            "hls: false\n",
            "webrtc: true\n",
            "webrtcAddress: 127.0.0.1:8889\n",
            "webrtcAllowOrigins: ['*']\n",
            "webrtcLocalUDPAddress: 127.0.0.1:8189\n",
            "webrtcLocalTCPAddress: ''\n",
            "webrtcIPsFromInterfaces: false\n",
            "webrtcAdditionalHosts: [127.0.0.1]\n",
            "srt: false\n",
            "authInternalUsers:\n",
            "  - user: app-preview\n",
            "    pass: {password}\n",
            "    ips: [127.0.0.1, '::1']\n",
            "    permissions:\n",
        ),
        password = password,
    );

    for index in 0..sources.len() {
        write!(
            contents,
            "      - action: read\n        path: camera-{index}\n"
        )
        .expect("writing to a String cannot fail");
    }

    contents.push_str("paths:\n");
    for (index, source) in sources.iter().enumerate() {
        let source = serde_json::to_string(&source.rtsp_url)?;
        write!(
            contents,
            concat!(
                "  camera-{index}:\n",
                "    source: {source}\n",
                "    sourceOnDemand: true\n",
                "    sourceOnDemandStartTimeout: 10s\n",
                "    sourceOnDemandCloseAfter: 10s\n",
                "    rtspTransport: tcp\n",
                "    record: false\n",
            ),
            index = index,
            source = source,
        )
        .expect("writing to a String cannot fail");
    }

    Ok(contents)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use crate::preview::{CameraSource, ConfigFile};

    fn sources() -> Vec<CameraSource> {
        vec![
            CameraSource {
                name: "Workshop".into(),
                rtsp_url: "rtsp://camera-0/live".into(),
            },
            CameraSource {
                name: "Assembly".into(),
                rtsp_url: "rtsp://user:p@ss@camera-1/live?profile=low#preview".into(),
            },
        ]
    }

    #[test]
    fn renders_web_rtc_preview_configuration() {
        let sources = sources();
        let config = ConfigFile::create(&sources, "local-password").unwrap();
        let contents = fs::read_to_string(config.path()).unwrap();

        assert_eq!(
            contents,
            format!(
                concat!(
                    "logDestinations: [stdout]\n",
                    "api: false\n",
                    "metrics: false\n",
                    "pprof: false\n",
                    "playback: false\n",
                    "rtsp: false\n",
                    "rtmp: false\n",
                    "hls: false\n",
                    "webrtc: true\n",
                    "webrtcAddress: 127.0.0.1:8889\n",
                    "webrtcAllowOrigins: ['*']\n",
                    "webrtcLocalUDPAddress: 127.0.0.1:8189\n",
                    "webrtcLocalTCPAddress: ''\n",
                    "webrtcIPsFromInterfaces: false\n",
                    "webrtcAdditionalHosts: [127.0.0.1]\n",
                    "srt: false\n",
                    "authInternalUsers:\n",
                    "  - user: app-preview\n",
                    "    pass: \"local-password\"\n",
                    "    ips: [127.0.0.1, '::1']\n",
                    "    permissions:\n",
                    "      - action: read\n",
                    "        path: camera-0\n",
                    "      - action: read\n",
                    "        path: camera-1\n",
                    "paths:\n",
                    "  camera-0:\n",
                    "    source: {source_0}\n",
                    "    sourceOnDemand: true\n",
                    "    sourceOnDemandStartTimeout: 10s\n",
                    "    sourceOnDemandCloseAfter: 10s\n",
                    "    rtspTransport: tcp\n",
                    "    record: false\n",
                    "  camera-1:\n",
                    "    source: {source_1}\n",
                    "    sourceOnDemand: true\n",
                    "    sourceOnDemandStartTimeout: 10s\n",
                    "    sourceOnDemandCloseAfter: 10s\n",
                    "    rtspTransport: tcp\n",
                    "    record: false\n",
                ),
                source_0 = serde_json::to_string(&sources[0].rtsp_url).unwrap(),
                source_1 = serde_json::to_string(&sources[1].rtsp_url).unwrap(),
            )
        );
    }

    #[test]
    fn owns_temporary_file_until_drop() {
        let config = ConfigFile::create(&sources(), "local-password").unwrap();
        let path = PathBuf::from(config.path());
        assert!(path.exists());

        drop(config);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn temporary_file_mode_is_0600() {
        use std::os::unix::fs::PermissionsExt;

        let config = ConfigFile::create(&sources(), "local-password").unwrap();
        let mode = fs::metadata(config.path()).unwrap().permissions().mode();

        assert_eq!(mode & 0o777, 0o600);
    }
}

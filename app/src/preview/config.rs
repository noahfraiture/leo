use std::{fmt::Write as _, io::Write as _, path::Path};

use tempfile::NamedTempFile;

use super::{CameraSource, Error};

pub(crate) struct ConfigFile(NamedTempFile);

impl ConfigFile {
    pub(crate) fn create(sources: &[CameraSource]) -> Result<Self, Error> {
        let contents = render(sources)?;
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

fn render(sources: &[CameraSource]) -> Result<String, Error> {
    let mut contents = concat!(
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
        "  - user: any\n",
        "    pass:\n",
        "    ips: [127.0.0.1, '::1']\n",
        "    permissions:\n",
    )
    .to_owned();

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
#[path = "tests/config.rs"]
mod tests;

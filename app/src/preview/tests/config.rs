use std::{fs, path::PathBuf};

use crate::preview::{CameraSource, ConfigFile};

fn sources() -> Vec<CameraSource> {
    vec![
        CameraSource {
            id: 26,
            name: "Workshop".into(),
            rtsp_url: "rtsp://camera-0/live".into(),
        },
        CameraSource {
            id: 41,
            name: "Assembly".into(),
            rtsp_url: "rtsp://user:p@ss@camera-1/live?profile=low#preview".into(),
        },
    ]
}

#[test]
fn renders_web_rtc_preview_configuration() {
    let sources = sources();
    let config = ConfigFile::create(&sources).unwrap();
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
                "  - user: any\n",
                "    pass:\n",
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
    let config = ConfigFile::create(&sources()).unwrap();
    let path = PathBuf::from(config.path());
    assert!(path.exists());

    drop(config);
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn temporary_file_mode_is_0600() {
    use std::os::unix::fs::PermissionsExt;

    let config = ConfigFile::create(&sources()).unwrap();
    let mode = fs::metadata(config.path()).unwrap().permissions().mode();

    assert_eq!(mode & 0o777, 0o600);
}

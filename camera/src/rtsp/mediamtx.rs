use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};

use tokio::{
    net::TcpStream,
    process::{Child, Command},
    time::sleep,
};

use crate::rtsp::{Error, config::ConfigFile};

const READINESS_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) struct Server {
    child: Child,
    _config: ConfigFile,
}

impl Server {
    pub(crate) async fn start(address: SocketAddr, video: PathBuf) -> Result<Self, Error> {
        let config = ConfigFile::create(address, &video)?;
        let mut command = Command::new("mediamtx");
        command
            .arg(config.path())
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                Error::MediaMtxNotFound(source)
            } else {
                Error::Spawn(source)
            }
        })?;

        if let Err(error) = wait_until_ready(&mut child, address, READINESS_TIMEOUT).await {
            return match child.kill().await {
                Ok(()) => Err(error),
                Err(source) => Err(Error::Stop(source)),
            };
        }

        Ok(Self {
            child,
            _config: config,
        })
    }

    pub(crate) async fn wait(&mut self) -> Result<(), Error> {
        let status = self.child.wait().await.map_err(Error::Wait)?;
        Err(Error::UnexpectedExit(status))
    }

    pub(crate) async fn stop(mut self) -> Result<(), Error> {
        self.child.kill().await.map_err(Error::Stop)
    }
}

async fn wait_until_ready(
    child: &mut Child,
    address: SocketAddr,
    timeout: Duration,
) -> Result<(), Error> {
    let result = tokio::time::timeout(timeout, async {
        tokio::select! {
            result = child.wait() => {
                let status = result.map_err(Error::Wait)?;
                Err(Error::ExitedBeforeReady(status))
            }
            () = async {
                let address = probe_address(address);
                loop {
                    sleep(PROBE_INTERVAL).await;
                    if TcpStream::connect(address).await.is_ok() {
                        return;
                    }
                }
            } => Ok(()),
        }
    })
    .await;

    match result {
        Ok(result) => result,
        Err(_) => Err(Error::ReadinessTimeout { address, timeout }),
    }
}

fn probe_address(mut address: SocketAddr) -> SocketAddr {
    if address.ip().is_unspecified() {
        address.set_ip(match address.ip() {
            IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
        });
    }
    address
}

#[cfg(test)]
mod tests {
    use std::{env, fs, net::SocketAddr, process::Stdio, time::Duration};

    use tokio::{
        net::TcpListener,
        process::{Child, Command},
        time::timeout,
    };

    use super::{READINESS_TIMEOUT, Server, probe_address, wait_until_ready};
    use crate::rtsp::{Error, config::ConfigFile};

    const LIVE_CHILD_ENV: &str = "CAMERA_RTSP_LIVE_CHILD";
    const LIVE_CHILD_TEST: &str = "rtsp::mediamtx::tests::live_child_process";

    fn child(args: &[&str]) -> Child {
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command.spawn().unwrap()
    }

    fn live_child() -> Child {
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(["--exact", LIVE_CHILD_TEST])
            .env(LIVE_CHILD_ENV, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command.spawn().unwrap()
    }

    fn unused_address() -> SocketAddr {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    }

    fn config() -> ConfigFile {
        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join("video.mp4");
        fs::write(&video, b"video").unwrap();
        ConfigFile::create("127.0.0.1:8554".parse().unwrap(), &video).unwrap()
    }

    #[test]
    fn live_child_process() {
        if env::var_os(LIVE_CHILD_ENV).is_some() {
            std::thread::sleep(Duration::from_secs(30));
        }
    }

    #[tokio::test]
    async fn readiness_succeeds_when_listener_accepts_connections() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut child = live_child();

        wait_until_ready(&mut child, address, Duration::from_secs(1))
            .await
            .unwrap();

        child.kill().await.unwrap();
    }

    #[tokio::test]
    async fn readiness_reports_child_exit() {
        let address = unused_address();
        let mut child = child(&["--list"]);

        let error = wait_until_ready(&mut child, address, Duration::from_secs(1))
            .await
            .unwrap_err();

        assert!(matches!(error, Error::ExitedBeforeReady(status) if status.success()));
    }

    #[tokio::test]
    async fn readiness_timeout_is_bounded() {
        let address = unused_address();
        let mut child = live_child();
        let readiness_timeout = Duration::from_millis(100);

        let error = timeout(
            Duration::from_secs(1),
            wait_until_ready(&mut child, address, readiness_timeout),
        )
        .await
        .expect("readiness exceeded its bound")
        .unwrap_err();

        assert!(matches!(
            error,
            Error::ReadinessTimeout {
                address: timed_out,
                timeout,
            } if timed_out == address && timeout == readiness_timeout
        ));
        assert_eq!(READINESS_TIMEOUT, Duration::from_secs(5));
        child.kill().await.unwrap();
    }

    #[test]
    fn wildcard_listeners_are_probed_via_loopback() {
        assert_eq!(
            probe_address("0.0.0.0:8554".parse().unwrap()),
            "127.0.0.1:8554".parse().unwrap()
        );
        assert_eq!(
            probe_address("[::]:8554".parse().unwrap()),
            "[::1]:8554".parse().unwrap()
        );
    }

    #[tokio::test]
    async fn wait_reports_successful_and_failed_exits_as_unexpected() {
        for (args, expected_success) in [
            (&["--list"][..], true),
            (&["--invalid-test-harness-option"][..], false),
        ] {
            let mut server = Server {
                child: child(args),
                _config: config(),
            };

            let error = server.wait().await.unwrap_err();

            assert!(matches!(
                error,
                Error::UnexpectedExit(status) if status.success() == expected_success
            ));
            server.stop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn stop_completes_for_live_and_reaped_children() {
        Server {
            child: live_child(),
            _config: config(),
        }
        .stop()
        .await
        .unwrap();

        let mut server = Server {
            child: child(&["--list"]),
            _config: config(),
        };
        assert!(server.child.wait().await.unwrap().success());
        server.stop().await.unwrap();
    }
}

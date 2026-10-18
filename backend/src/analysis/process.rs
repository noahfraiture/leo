//! Bounded media conversion processes. Output lives in caller-owned files, not pipe buffers.
use std::{
    io,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, LazyLock, Mutex, PoisonError},
    thread,
    time::{Duration, Instant},
};

const POLL_INTERVAL: Duration = Duration::from_millis(20);
static PROCESSES: LazyLock<ProcessRegistry> = LazyLock::new(ProcessRegistry::default);

/// Run a file-producing media command without interactive input, killing and reaping on timeout.
/// Diagnostics are suppressed because media paths may contain private operator information.
pub fn run(command: &mut Command, timeout: Duration) -> io::Result<ExitStatus> {
    let child = PROCESSES.spawn(command)?;
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .child
            .lock()
            .map_err(|_| io::Error::other("Media process ownership is unavailable"))?
            .try_wait()?
        {
            return Ok(status);
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Media conversion exceeded its time limit",
            ));
        }
        thread::sleep(POLL_INTERVAL.min(remaining));
    }
}

/// Permanently stop and reap analysis conversions before the host exits its native event loop.
/// New conversions are rejected after shutdown, including those from already-running workers.
pub fn stop_media_processes() -> io::Result<()> {
    PROCESSES.shutdown()
}

#[derive(Default)]
struct ProcessRegistry {
    state: Mutex<ProcessState>,
}

#[derive(Default)]
struct ProcessState {
    stopping: bool,
    children: Vec<Arc<Mutex<Child>>>,
}

impl ProcessRegistry {
    fn spawn(&self, command: &mut Command) -> io::Result<OwnedChild<'_>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("Media process registry is unavailable"))?;
        if state.stopping {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Media processing is shutting down",
            ));
        }
        // Admission and registration share the shutdown lock, so no child can escape its sweep.
        let child = Arc::new(Mutex::new(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        ));
        state.children.push(child.clone());
        Ok(OwnedChild {
            registry: self,
            child,
        })
    }

    fn shutdown(&self) -> io::Result<()> {
        // Keep shutdown calls serialized until every owned child has been stopped and reaped.
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.stopping = true;
        let mut result = Ok(());
        for child in state.children.drain(..) {
            if let Err(error) = stop_child(&child)
                && result.is_ok()
            {
                result = Err(error);
            }
        }
        result
    }
}

/// Every error path retains cleanup ownership until the child has been stopped and reaped.
struct OwnedChild<'a> {
    registry: &'a ProcessRegistry,
    child: Arc<Mutex<Child>>,
}

impl Drop for OwnedChild<'_> {
    fn drop(&mut self) {
        if let Err(error) = stop_child(&self.child) {
            tracing::error!(%error, "media conversion process could not be stopped and reaped");
        }
        self.registry
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .children
            .retain(|child| !Arc::ptr_eq(child, &self.child));
    }
}

fn stop_child(child: &Mutex<Child>) -> io::Result<()> {
    let mut child = child.lock().unwrap_or_else(PoisonError::into_inner);
    if child.try_wait().is_ok_and(|status| status.is_some()) {
        return Ok(());
    }
    let kill_error = child.kill().err();
    // Even a raced exit or failed kill must still attempt to reap the owned process.
    child
        .wait()
        .map(|_| ())
        .map_err(|error| kill_error.unwrap_or(error))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn completed_commands_retain_their_exit_status() {
        for code in [0, 7] {
            let status = run(
                Command::new("sh").args(["-c", &format!("exit {code}")]),
                Duration::from_secs(2),
            )
            .unwrap();
            assert_eq!(status.code(), Some(code));
        }
    }

    #[test]
    fn a_stalled_conversion_times_out_without_leaving_a_live_child() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("child.pid");
        let started = Instant::now();
        let error = run(
            Command::new("sh")
                .args([
                    "-c",
                    "printf '%s' \"$$\" > \"$LEO_TEST_CHILD_PID\"; exec sleep 30",
                ])
                .env("LEO_TEST_CHILD_PID", &pid_file),
            Duration::from_secs(1),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5));
        let pid = std::fs::read_to_string(pid_file).unwrap();
        assert!(
            !Command::new("kill")
                .args(["-0", &pid])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn shutdown_stops_all_children_and_rejects_new_conversions() {
        let registry = ProcessRegistry::default();
        let children = [
            registry.spawn(Command::new("sleep").arg("30")).unwrap(),
            registry.spawn(Command::new("sleep").arg("30")).unwrap(),
        ];
        let pids = children
            .each_ref()
            .map(|child| child.child.lock().unwrap().id());

        registry.shutdown().unwrap();
        for pid in pids {
            assert!(
                !Command::new("kill")
                    .args(["-0", &pid.to_string()])
                    .stderr(Stdio::null())
                    .status()
                    .unwrap()
                    .success()
            );
        }
        assert_eq!(
            registry
                .spawn(Command::new("sleep").arg("30"))
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::Interrupted
        );
        registry.shutdown().unwrap();
    }
}

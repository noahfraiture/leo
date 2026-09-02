//! Owns the recorder actor, per-camera supervisors, and each FFmpeg attempt's cleanup.
//! These layers share stop and fault ownership, so they remain one state machine.

use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{ChildStderr, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use ffmpeg_sidecar::{
    child::FfmpegChild,
    command::FfmpegCommand,
    event::{FfmpegEvent, FfmpegProgress},
    ffmpeg_time_duration::FfmpegTimeDuration,
    log_parser::FfmpegLogParser,
    paths::ffmpeg_path,
};
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot,
};
use url::Url;

use super::{
    error::{Error, Result},
    segment::{RecordingSegment, probe_media},
};

const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// One camera source supervised by the host recorder.
#[derive(Clone, PartialEq, Eq)]
pub struct RecordingCamera {
    /// Stable camera ID used by recorder events and the output directory name.
    pub id: u32,
    /// The credential-bearing source URL; recorder diagnostics never expose it.
    pub rtsp_url: String,
}

/// Time bounds controlling recorder I/O, reconnects, and graceful stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecorderSettings {
    /// Bound for executable preflight, startup readiness, and recorder network I/O.
    pub io_timeout: Duration,
    /// Delay before restarting a camera after an interrupted attempt.
    pub retry_delay: Duration,
    /// Grace period before an FFmpeg child is killed during Stop.
    pub stop_timeout: Duration,
}

/// Current per-camera recorder lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderStatus {
    Starting,
    Recording,
    Reconnecting,
    Stopped,
}

/// Sanitized recorder state and fatal-error notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecorderEvent {
    Status {
        camera_id: u32,
        status: RecorderStatus,
        message: Option<String>,
    },
    Faulted {
        camera_id: Option<u32>,
        message: String,
    },
}

enum RecorderCommand {
    Start {
        cameras: Vec<RecordingCamera>,
        recordings_root: PathBuf,
        reply: oneshot::Sender<Result<()>>,
    },
    Stop {
        reply: oneshot::Sender<Result<Vec<RecordingSegment>>>,
    },
    Shutdown,
}

/// Cloneable asynchronous command boundary for one recorder runtime.
#[derive(Clone)]
pub struct RecorderHandle {
    commands: Sender<RecorderCommand>,
    shutdown: Arc<AtomicBool>,
}

/// Owner of the recorder management thread and all process cleanup.
pub struct RecorderRuntime {
    commands: Sender<RecorderCommand>,
    shutdown: Arc<AtomicBool>,
    management: Option<JoinHandle<Result<()>>>,
}

struct RecorderSet {
    stop: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    startup: Arc<Mutex<StartupState>>,
    fault_emitted: Arc<AtomicBool>,
    events: UnboundedSender<RecorderEvent>,
    supervisors: Vec<CameraSupervisor>,
}

struct CameraSupervisor {
    thread: JoinHandle<Result<Vec<RecordingSegment>>>,
}

enum StartupEvent {
    Ready { camera_id: u32 },
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StartupState {
    Pending,
    Failed,
    Published,
    Cancelled,
}

enum StartupPublication {
    Published,
    Failed(oneshot::Sender<Result<()>>),
    ReplyDropped,
}

impl RecorderRuntime {
    /// Preflights host executables and starts the recorder management thread.
    pub fn spawn(
        settings: RecorderSettings,
    ) -> Result<(Self, RecorderHandle, UnboundedReceiver<RecorderEvent>)> {
        spawn_with_executables(
            settings,
            ffmpeg_path(),
            ffmpeg_sidecar::ffprobe::ffprobe_path(),
        )
    }

    /// Stops every active recorder process and joins the management thread.
    pub fn shutdown(mut self) -> Result<()> {
        self.shutdown_inner()
    }

    fn shutdown_inner(&mut self) -> Result<()> {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = self.commands.send(RecorderCommand::Shutdown);
        let Some(management) = self.management.take() else {
            return Ok(());
        };
        management.join().map_err(|_| Error::RecorderThread)?
    }
}

impl Drop for RecorderRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown_inner();
    }
}

impl RecorderHandle {
    /// Starts all requested cameras and resolves only after they are ready.
    pub async fn start(
        &self,
        cameras: Vec<RecordingCamera>,
        recordings_root: PathBuf,
    ) -> Result<()> {
        if self.shutdown.load(Ordering::Relaxed) {
            return Err(Error::Shutdown);
        }
        validate_start(&cameras, &recordings_root)?;
        let (reply, response) = oneshot::channel();
        self.commands
            .send(RecorderCommand::Start {
                cameras,
                recordings_root,
                reply,
            })
            .map_err(|_| Error::RecorderCommandClosed)?;
        response.await.map_err(|_| Error::RecorderReplyDropped)?
    }

    /// Stops every active camera and returns all finalized segments.
    pub async fn stop(&self) -> Result<Vec<RecordingSegment>> {
        if self.shutdown.load(Ordering::Relaxed) {
            return Err(Error::Shutdown);
        }
        let (reply, response) = oneshot::channel();
        self.commands
            .send(RecorderCommand::Stop { reply })
            .map_err(|_| Error::RecorderCommandClosed)?;
        response.await.map_err(|_| Error::RecorderReplyDropped)?
    }
}

fn spawn_with_executables(
    settings: RecorderSettings,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
) -> Result<(
    RecorderRuntime,
    RecorderHandle,
    UnboundedReceiver<RecorderEvent>,
)> {
    validate_settings(settings)?;
    let shutdown = Arc::new(AtomicBool::new(false));
    preflight(&ffmpeg, settings.io_timeout)?;
    preflight(&ffprobe, settings.io_timeout)?;

    spawn_management_thread(settings, ffmpeg, ffprobe, shutdown)
}

fn spawn_management_thread(
    settings: RecorderSettings,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    shutdown: Arc<AtomicBool>,
) -> Result<(
    RecorderRuntime,
    RecorderHandle,
    UnboundedReceiver<RecorderEvent>,
)> {
    let (commands, command_receiver) = mpsc::channel();
    let (events, event_receiver) = tokio::sync::mpsc::unbounded_channel();
    let thread_shutdown = Arc::clone(&shutdown);
    let management = thread::Builder::new()
        .name("recorder-management".into())
        .spawn(move || {
            management_loop(
                command_receiver,
                settings,
                ffmpeg,
                ffprobe,
                events,
                thread_shutdown,
            )
        })?;
    let handle = RecorderHandle {
        commands: commands.clone(),
        shutdown: Arc::clone(&shutdown),
    };
    Ok((
        RecorderRuntime {
            commands,
            shutdown,
            management: Some(management),
        },
        handle,
        event_receiver,
    ))
}

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub fn spawn_for_test(
    settings: RecorderSettings,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
) -> Result<(
    RecorderRuntime,
    RecorderHandle,
    UnboundedReceiver<RecorderEvent>,
)> {
    validate_settings(settings)?;
    spawn_management_thread(settings, ffmpeg, ffprobe, Arc::new(AtomicBool::new(false)))
}

fn management_loop(
    commands: Receiver<RecorderCommand>,
    settings: RecorderSettings,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    events: UnboundedSender<RecorderEvent>,
    shutdown: Arc<AtomicBool>,
) -> Result<()> {
    let mut active: Option<RecorderSet> = None;
    loop {
        if shutdown.load(Ordering::Relaxed) {
            return match active.take() {
                Some(recorders) => stop_for_shutdown(recorders),
                None => Ok(()),
            };
        }
        match commands.recv_timeout(CHILD_POLL_INTERVAL) {
            Ok(RecorderCommand::Start {
                cameras,
                recordings_root,
                reply,
            }) => {
                if active.is_some() {
                    let _ = reply.send(Err(Error::RecorderAlreadyActive));
                    continue;
                }
                match start_recorder_set(
                    settings,
                    &ffmpeg,
                    &ffprobe,
                    cameras,
                    recordings_root,
                    Arc::clone(&shutdown),
                    events.clone(),
                ) {
                    Ok(recorders) if shutdown.load(Ordering::Relaxed) => {
                        recorders.cancel_startup();
                        let cleanup = stop_for_shutdown(recorders);
                        let _ = reply.send(Err(Error::Shutdown));
                        cleanup?;
                        return Ok(());
                    }
                    Ok(recorders) => match recorders.publish_start(reply) {
                        StartupPublication::Published => {
                            active = Some(recorders);
                        }
                        StartupPublication::Failed(reply) => {
                            let error =
                                cleanup_failed_start(recorders, Error::RecorderStartupFailed);
                            let _ = reply.send(Err(error));
                        }
                        StartupPublication::ReplyDropped => {
                            recorders.stop()?;
                        }
                    },
                    Err(error) => {
                        let _ = reply.send(Err(error));
                    }
                }
            }
            Ok(RecorderCommand::Stop { reply }) => {
                let result = match active.take() {
                    Some(recorders) => recorders.stop(),
                    None => Err(Error::RecorderNotActive),
                };
                let _ = reply.send(result);
            }
            Ok(RecorderCommand::Shutdown) => {
                return match active.take() {
                    Some(recorders) => stop_for_shutdown(recorders),
                    None => Ok(()),
                };
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return match active.take() {
                    Some(recorders) => stop_for_shutdown(recorders),
                    None => Ok(()),
                };
            }
        }
    }
}

fn start_recorder_set(
    settings: RecorderSettings,
    ffmpeg: &Path,
    ffprobe: &Path,
    cameras: Vec<RecordingCamera>,
    recordings_root: PathBuf,
    shutdown: Arc<AtomicBool>,
    events: UnboundedSender<RecorderEvent>,
) -> Result<RecorderSet> {
    let deadline = Instant::now()
        .checked_add(settings.io_timeout)
        .ok_or(Error::InvalidRecorderTimeout)?;
    let stop = Arc::new(AtomicBool::new(false));
    let startup = Arc::new(Mutex::new(StartupState::Pending));
    let fault_emitted = Arc::new(AtomicBool::new(false));
    let (startup_sender, startup_events) = mpsc::channel();
    let mut recorders = RecorderSet {
        stop,
        shutdown: Arc::clone(&shutdown),
        startup,
        fault_emitted,
        events,
        supervisors: Vec::with_capacity(cameras.len()),
    };

    for camera in cameras {
        let camera_id = camera.id;
        let camera_directory = recordings_root.join(format!("camera-{camera_id}"));
        let ffmpeg = ffmpeg.to_path_buf();
        let ffprobe = ffprobe.to_path_buf();
        let startup_sender = startup_sender.clone();
        let stop = Arc::clone(&recorders.stop);
        let startup = Arc::clone(&recorders.startup);
        let fault_emitted = Arc::clone(&recorders.fault_emitted);
        let events = recorders.events.clone();
        let thread_shutdown = Arc::clone(&shutdown);
        let thread = thread::Builder::new()
            .name(format!("recorder-camera-{camera_id}"))
            .spawn(move || {
                supervise_camera(
                    settings,
                    ffmpeg,
                    ffprobe,
                    camera,
                    camera_directory,
                    deadline,
                    stop,
                    thread_shutdown,
                    startup,
                    fault_emitted,
                    startup_sender,
                    events,
                )
            });
        match thread {
            Ok(thread) => recorders.supervisors.push(CameraSupervisor { thread }),
            Err(error) => {
                return Err(cleanup_failed_start(recorders, Error::Io(error)));
            }
        }
    }
    drop(startup_sender);

    let mut ready = HashSet::with_capacity(recorders.supervisors.len());
    while ready.len() < recorders.supervisors.len() {
        if shutdown.load(Ordering::Relaxed) {
            return Err(cleanup_failed_start(recorders, Error::Shutdown));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(cleanup_failed_start(
                recorders,
                Error::RecorderStartupTimeout,
            ));
        }
        match startup_events.recv_timeout(CHILD_POLL_INTERVAL.min(remaining)) {
            Ok(StartupEvent::Ready { camera_id }) => {
                ready.insert(camera_id);
            }
            Ok(StartupEvent::Failed) => {
                return Err(cleanup_failed_start(
                    recorders,
                    Error::RecorderStartupFailed,
                ));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(cleanup_failed_start(
                    recorders,
                    Error::RecorderStartupFailed,
                ));
            }
        }
    }
    Ok(recorders)
}

fn stop_for_shutdown(recorders: RecorderSet) -> Result<()> {
    match recorders.stop() {
        Ok(_) | Err(Error::Shutdown) => Ok(()),
        Err(error) => Err(error),
    }
}

fn cleanup_failed_start(recorders: RecorderSet, error: Error) -> Error {
    if matches!(&error, Error::Shutdown) {
        recorders.cancel_startup();
    } else {
        recorders.fail_startup();
    }
    recorders.finish_failed_start(error)
}

impl RecorderSet {
    fn publish_start(&self, reply: oneshot::Sender<Result<()>>) -> StartupPublication {
        let mut startup = startup_guard(&self.startup);
        match *startup {
            StartupState::Pending => {
                if reply.send(Ok(())).is_ok() {
                    *startup = StartupState::Published;
                    StartupPublication::Published
                } else {
                    *startup = StartupState::Cancelled;
                    StartupPublication::ReplyDropped
                }
            }
            StartupState::Failed | StartupState::Cancelled | StartupState::Published => {
                StartupPublication::Failed(reply)
            }
        }
    }

    fn fail_startup(&self) {
        let mut startup = startup_guard(&self.startup);
        if *startup == StartupState::Pending {
            *startup = StartupState::Failed;
        }
    }

    fn cancel_startup(&self) {
        let mut startup = startup_guard(&self.startup);
        if *startup == StartupState::Pending {
            *startup = StartupState::Cancelled;
        }
    }

    fn stop(self) -> Result<Vec<RecordingSegment>> {
        self.finish(None)
    }

    fn finish_failed_start(self, initial_error: Error) -> Error {
        self.stop.store(true, Ordering::Relaxed);
        let mut cleanup_failed = false;
        for supervisor in self.supervisors {
            match supervisor.thread.join() {
                Ok(Ok(_)) => {}
                Ok(Err(error)) if !matches!(error, Error::RecorderCleanupFailed { .. }) => {}
                Ok(Err(Error::Shutdown)) if matches!(&initial_error, Error::Shutdown) => {}
                Ok(Err(_)) | Err(_) => cleanup_failed = true,
            }
        }
        if cleanup_failed {
            Error::RecorderStartupCleanupFailed
        } else {
            initial_error
        }
    }

    fn finish(self, mut first_error: Option<Error>) -> Result<Vec<RecordingSegment>> {
        // One shared signal reaches every supervisor before the first blocking join.
        self.stop.store(true, Ordering::Relaxed);
        tracing::info!(camera_count = self.supervisors.len(), "stopping recorders");

        let mut segments = Vec::new();
        for supervisor in self.supervisors {
            match supervisor.thread.join() {
                Ok(Ok(mut camera_segments)) => segments.append(&mut camera_segments),
                Ok(Err(error)) => record_first_error(&mut first_error, error),
                Err(_) => record_first_error(&mut first_error, Error::RecorderThread),
            }
        }
        if self.shutdown.load(Ordering::Relaxed) {
            record_first_error(&mut first_error, Error::Shutdown);
        }
        if let Some(error) = first_error {
            if startup_is_published(&self.startup) && !matches!(error, Error::Shutdown) {
                emit_fault_once(&self.fault_emitted, &self.events, None, error.to_string());
            }
            return Err(error);
        }
        segments.sort_by_key(|segment| (segment.camera_id, segment.start_utc_ms));
        Ok(segments)
    }
}

fn startup_guard(startup: &Mutex<StartupState>) -> MutexGuard<'_, StartupState> {
    startup
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn startup_is_published(startup: &Mutex<StartupState>) -> bool {
    *startup_guard(startup) == StartupState::Published
}

fn failure_is_post_start(
    startup: &Mutex<StartupState>,
    startup_events: &Sender<StartupEvent>,
) -> bool {
    let mut state = startup_guard(startup);
    match *state {
        StartupState::Pending => {
            *state = StartupState::Failed;
            drop(state);
            let _ = startup_events.send(StartupEvent::Failed);
            false
        }
        StartupState::Published => true,
        StartupState::Failed | StartupState::Cancelled => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn supervise_camera(
    settings: RecorderSettings,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    camera: RecordingCamera,
    camera_directory: PathBuf,
    deadline: Instant,
    stop: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    startup_state: Arc<Mutex<StartupState>>,
    fault_emitted: Arc<AtomicBool>,
    startup_events: Sender<StartupEvent>,
    events: UnboundedSender<RecorderEvent>,
) -> Result<Vec<RecordingSegment>> {
    if send_status(&events, camera.id, RecorderStatus::Starting, None).is_err() {
        failure_is_post_start(&startup_state, &startup_events);
        return Err(Error::RecorderEventReceiverClosed);
    }

    let mut initial_attempt = true;
    let mut previous_end_utc_ms = None;
    let mut segments = Vec::new();
    loop {
        if cancellation_requested(&stop, &shutdown) {
            return finish_cancelled_supervisor(
                &startup_state,
                &events,
                camera.id,
                &shutdown,
                segments,
            );
        }
        let attempt_id = uuid::Uuid::new_v4();
        let partial_path = camera_directory.join(format!(".attempt-{attempt_id}.partial.mkv"));
        tracing::info!(camera_id = camera.id, %attempt_id, "spawning recorder attempt");
        if !initial_attempt && cancellation_requested(&stop, &shutdown) {
            return finish_cancelled_supervisor(
                &startup_state,
                &events,
                camera.id,
                &shutdown,
                segments,
            );
        }
        let result = run_attempt_with_ready(
            AttemptConfig {
                ffmpeg: &ffmpeg,
                rtsp_url: &camera.rtsp_url,
                partial_path: &partial_path,
                io_timeout: settings.io_timeout,
                stop_timeout: settings.stop_timeout,
            },
            &stop,
            &shutdown,
            initial_attempt.then_some(deadline),
            |_| {
                send_status(&events, camera.id, RecorderStatus::Recording, None)?;
                if initial_attempt {
                    startup_events
                        .send(StartupEvent::Ready {
                            camera_id: camera.id,
                        })
                        .map_err(|_| Error::RecorderCommandClosed)?;
                }
                tracing::info!(camera_id = camera.id, %attempt_id, "recorder attempt ready");
                Ok(())
            },
            || {
                failure_is_post_start(&startup_state, &startup_events);
            },
            utc_now_ms,
        );

        let result = match result {
            Ok(result) => result,
            Err(error) => {
                if failure_is_post_start(&startup_state, &startup_events) {
                    emit_fault_once(&fault_emitted, &events, Some(camera.id), error.to_string());
                }
                return Err(error);
            }
        };

        if initial_attempt {
            if result.stopped && !startup_is_published(&startup_state) {
                return Ok(segments);
            }
            if !result.stopped && !failure_is_post_start(&startup_state, &startup_events) {
                return Err(Error::RecorderStartupFailed);
            }
        }
        if !startup_is_published(&startup_state) {
            return Ok(segments);
        }
        if shutdown.load(Ordering::Relaxed) {
            return Ok(segments);
        }

        match finalize_attempt(
            camera.id,
            &partial_path,
            &ffprobe,
            settings.io_timeout,
            &shutdown,
            result.media_zero_utc_ms,
            previous_end_utc_ms,
        ) {
            Ok(Some(segment)) => {
                previous_end_utc_ms = Some(segment.end_utc_ms);
                tracing::info!(
                    camera_id = camera.id,
                    %attempt_id,
                    path = %segment.path.display(),
                    "finalized recorder attempt"
                );
                segments.push(segment);
            }
            Ok(None) => {}
            Err(Error::Shutdown) if shutdown.load(Ordering::Relaxed) => return Ok(segments),
            Err(error) => {
                emit_fault_once(&fault_emitted, &events, Some(camera.id), error.to_string());
                return Err(error);
            }
        }

        if result.stopped {
            tracing::info!(camera_id = camera.id, "recorder stopped");
            if let Err(error) = send_status(&events, camera.id, RecorderStatus::Stopped, None) {
                emit_fault_once(&fault_emitted, &events, Some(camera.id), error.to_string());
                return Err(error);
            }
            return Ok(segments);
        }

        tracing::info!(camera_id = camera.id, %attempt_id, "recorder attempt exited");
        if let Err(error) = send_status(
            &events,
            camera.id,
            RecorderStatus::Reconnecting,
            Some("recording interrupted; retrying".into()),
        ) {
            emit_fault_once(&fault_emitted, &events, Some(camera.id), error.to_string());
            return Err(error);
        }
        tracing::info!(camera_id = camera.id, "waiting to reconnect recorder");
        if wait_interruptibly(settings.retry_delay, &stop, &shutdown)?
            || cancellation_requested(&stop, &shutdown)
        {
            return finish_cancelled_supervisor(
                &startup_state,
                &events,
                camera.id,
                &shutdown,
                segments,
            );
        }
        let storage_result = probe_storage(&camera_directory);
        if let Err(error) = storage_result {
            tracing::warn!(camera_id = camera.id, "recorder storage probe failed");
            emit_fault_once(&fault_emitted, &events, Some(camera.id), error.to_string());
            return Err(error);
        }
        if cancellation_requested(&stop, &shutdown) {
            return finish_cancelled_supervisor(
                &startup_state,
                &events,
                camera.id,
                &shutdown,
                segments,
            );
        }
        initial_attempt = false;
    }
}

fn cancellation_requested(stop: &AtomicBool, shutdown: &AtomicBool) -> bool {
    stop.load(Ordering::Relaxed) || shutdown.load(Ordering::Relaxed)
}

fn finish_cancelled_supervisor(
    startup: &Mutex<StartupState>,
    events: &UnboundedSender<RecorderEvent>,
    camera_id: u32,
    shutdown: &AtomicBool,
    segments: Vec<RecordingSegment>,
) -> Result<Vec<RecordingSegment>> {
    if startup_is_published(startup) && !shutdown.load(Ordering::Relaxed) {
        send_status(events, camera_id, RecorderStatus::Stopped, None)?;
    }
    Ok(segments)
}

fn wait_interruptibly(
    duration: Duration,
    stop: &AtomicBool,
    shutdown: &AtomicBool,
) -> Result<bool> {
    let deadline = Instant::now()
        .checked_add(duration)
        .ok_or(Error::InvalidRecorderTimeout)?;
    loop {
        if stop.load(Ordering::Relaxed) || shutdown.load(Ordering::Relaxed) {
            return Ok(true);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        thread::sleep(CHILD_POLL_INTERVAL.min(remaining));
    }
}

fn probe_storage(directory: &Path) -> Result<()> {
    let path = directory.join(format!(".storage-probe-{}", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let mut first_error = None;
    if let Err(error) = file.write_all(&[0]) {
        first_error = Some(Error::Io(error));
    }
    if let Err(error) = file.sync_all() {
        record_first_error(&mut first_error, Error::Io(error));
    }
    drop(file);
    if let Err(error) = fs::remove_file(path) {
        record_first_error(&mut first_error, Error::Io(error));
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn send_status(
    events: &UnboundedSender<RecorderEvent>,
    camera_id: u32,
    status: RecorderStatus,
    message: Option<String>,
) -> Result<()> {
    events
        .send(RecorderEvent::Status {
            camera_id,
            status,
            message,
        })
        .map_err(|_| Error::RecorderEventReceiverClosed)
}

fn emit_fault_once(
    emitted: &AtomicBool,
    events: &UnboundedSender<RecorderEvent>,
    camera_id: Option<u32>,
    message: String,
) {
    if emitted
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        tracing::error!(camera_id, "fatal recorder supervision failure");
        let _ = events.send(RecorderEvent::Faulted { camera_id, message });
    }
}

fn utc_now_ms() -> Result<i64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::TimestampOverflow)?
        .as_millis();
    i64::try_from(millis).map_err(|_| Error::TimestampOverflow)
}

fn validate_settings(settings: RecorderSettings) -> Result<()> {
    for timeout in [
        settings.io_timeout,
        settings.retry_delay,
        settings.stop_timeout,
    ] {
        checked_timeout_microseconds(timeout)?;
    }
    Ok(())
}

fn validate_start(cameras: &[RecordingCamera], recordings_root: &Path) -> Result<()> {
    if cameras.is_empty() {
        return Err(Error::EmptyCameraList);
    }
    let mut camera_ids = HashSet::with_capacity(cameras.len());
    for camera in cameras {
        if camera.id == 0 {
            return Err(Error::ZeroCameraId);
        }
        if !camera_ids.insert(camera.id) {
            return Err(Error::DuplicateCamera {
                camera_id: camera.id,
            });
        }
        if !Url::parse(&camera.rtsp_url).is_ok_and(|url| url.scheme() == "rtsp") {
            return Err(Error::InvalidCameraUrl {
                camera_id: camera.id,
            });
        }
    }
    if !fs::symlink_metadata(recordings_root).is_ok_and(|metadata| metadata.file_type().is_dir()) {
        return Err(Error::InvalidRecordingsRoot);
    }
    for camera in cameras {
        let path = recordings_root.join(format!("camera-{}", camera.id));
        if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir()) {
            return Err(Error::InvalidCameraDirectory {
                camera_id: camera.id,
            });
        }
    }
    Ok(())
}

fn preflight(executable: &Path, timeout: Duration) -> Result<()> {
    let started = Instant::now();
    let mut child = Command::new(executable)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut status = None;
    let mut first_error = None;
    loop {
        match child.try_wait() {
            Ok(Some(exit_status)) => {
                status = Some(exit_status);
                break;
            }
            Ok(None) => {}
            Err(error) => {
                first_error = Some(Error::Io(error));
                break;
            }
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            first_error = Some(Error::RecorderPreflightTimeout);
            break;
        }
        thread::sleep(CHILD_POLL_INTERVAL.min(timeout.saturating_sub(elapsed)));
    }

    if status.is_none()
        && let Err(error) = child.kill()
    {
        record_first_error(&mut first_error, Error::Io(error));
    }
    match child.wait() {
        Ok(exit_status) => status = Some(exit_status),
        Err(error) => record_first_error(&mut first_error, Error::Io(error)),
    }
    if first_error.is_none() && status.is_some_and(|status| !status.success()) {
        first_error = Some(Error::RecorderPreflightFailed);
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Inputs needed to execute one bounded FFmpeg recording process.
struct AttemptConfig<'a> {
    ffmpeg: &'a Path,
    rtsp_url: &'a str,
    partial_path: &'a Path,
    io_timeout: Duration,
    stop_timeout: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptEvent {
    Ready { media_zero_utc_ms: i64 },
}

#[derive(Debug)]
struct AttemptResult {
    media_zero_utc_ms: Option<i64>,
    stopped: bool,
}

enum PumpEvent {
    Progress(FfmpegProgress),
    Failed,
    Eof,
}

#[cfg(all(test, unix))]
fn run_attempt_with_clock(
    config: AttemptConfig<'_>,
    stop: &AtomicBool,
    shutdown: &AtomicBool,
    events: &Sender<AttemptEvent>,
    now_utc_ms: impl FnMut() -> Result<i64>,
) -> Result<AttemptResult> {
    run_attempt_with_ready(
        config,
        stop,
        shutdown,
        None,
        |event| {
            events
                .send(event)
                .map_err(|_| Error::RecorderEventReceiverClosed)
        },
        || {},
        now_utc_ms,
    )
}

fn run_attempt_with_ready(
    config: AttemptConfig<'_>,
    stop: &AtomicBool,
    shutdown: &AtomicBool,
    readiness_deadline: Option<Instant>,
    mut on_ready: impl FnMut(AttemptEvent) -> Result<()>,
    mut on_terminal: impl FnMut(),
    mut now_utc_ms: impl FnMut() -> Result<i64>,
) -> Result<AttemptResult> {
    let timeout_microseconds = checked_timeout_microseconds(config.io_timeout)?;
    let mut command = FfmpegCommand::new_with_path(config.ffmpeg);
    command
        .hide_banner()
        .no_overwrite()
        .args(["-rtsp_transport", "tcp", "-timeout"])
        .arg(timeout_microseconds.to_string())
        .arg("-i")
        .arg(config.rtsp_url)
        .args(["-map", "0:v:0", "-an", "-c:v", "copy"])
        .args(["-avoid_negative_ts", "make_zero"])
        .args(["-f", "matroska"])
        .arg(config.partial_path);

    let mut child = command.spawn()?;
    drop(child.take_stdout());

    let mut first_error = None;
    let mut status = None;
    let (pump_tx, pump_rx) = mpsc::channel();
    let pump = match child.take_stderr() {
        Some(stderr) => match thread::Builder::new()
            .name("ffmpeg-stderr".into())
            .spawn(move || pump_stderr(stderr, pump_tx))
        {
            Ok(pump) => Some(pump),
            Err(error) => {
                first_error = Some(Error::Io(error));
                None
            }
        },
        None => {
            first_error = Some(Error::FfmpegPipes);
            None
        }
    };

    let mut media_zero_utc_ms = None;
    let mut stopped = false;
    let mut pump_finished = pump.is_none();
    let mut terminal_reported = false;
    while first_error.is_none() {
        if stop.load(Ordering::Relaxed) || shutdown.load(Ordering::Relaxed) {
            stopped = true;
            break;
        }
        if media_zero_utc_ms.is_none()
            && readiness_deadline.is_some_and(|deadline| Instant::now() >= deadline)
        {
            first_error = Some(Error::RecorderStartupTimeout);
            break;
        }

        if status.is_none() {
            match child.as_inner_mut().try_wait() {
                Ok(Some(exit_status)) => {
                    status = Some(exit_status);
                    if !terminal_reported {
                        on_terminal();
                        terminal_reported = true;
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    first_error = Some(Error::Io(error));
                    break;
                }
            }
        }
        if status.is_some() && pump_finished {
            break;
        }

        if pump_finished {
            thread::sleep(CHILD_POLL_INTERVAL);
            continue;
        }
        match pump_rx.recv_timeout(CHILD_POLL_INTERVAL) {
            Ok(PumpEvent::Progress(progress)) => handle_progress(
                progress,
                config.partial_path,
                &mut on_ready,
                true,
                &mut now_utc_ms,
                &mut media_zero_utc_ms,
                &mut first_error,
            ),
            Ok(PumpEvent::Failed) => first_error = Some(Error::FfmpegParser),
            Ok(PumpEvent::Eof) => pump_finished = true,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                first_error = Some(Error::FfmpegPump);
            }
        }
    }

    if !terminal_reported
        && (first_error.is_some() || !stopped && status.is_some() && pump_finished)
    {
        on_terminal();
        terminal_reported = true;
    }

    drain_pump_with_ready(
        &pump_rx,
        config.partial_path,
        &mut on_ready,
        &mut now_utc_ms,
        &mut media_zero_utc_ms,
        &mut first_error,
    );
    if !terminal_reported && first_error.is_some() {
        on_terminal();
    }
    let mut cleanup_error = None;
    cleanup_child(
        &mut child,
        pump,
        config.stop_timeout,
        &mut status,
        &mut cleanup_error,
    );
    if let Some(source) = cleanup_error {
        first_error = Some(Error::RecorderCleanupFailed {
            source: Box::new(source),
        });
    }
    drain_pump_with_ready(
        &pump_rx,
        config.partial_path,
        &mut on_ready,
        &mut now_utc_ms,
        &mut media_zero_utc_ms,
        &mut first_error,
    );

    match first_error {
        Some(error) => Err(error),
        None => Ok(AttemptResult {
            media_zero_utc_ms,
            stopped,
        }),
    }
}

fn checked_timeout_microseconds(timeout: Duration) -> Result<i64> {
    i64::try_from(timeout.as_micros())
        .ok()
        .filter(|microseconds| *microseconds > 0)
        .ok_or(Error::InvalidRecorderTimeout)
}

fn pump_stderr(stderr: ChildStderr, events: Sender<PumpEvent>) {
    let mut parser = FfmpegLogParser::new(stderr);
    loop {
        let event = match parser.parse_next_event() {
            Ok(event) => event,
            Err(_) => {
                let _ = events.send(PumpEvent::Failed);
                break;
            }
        };
        match event {
            FfmpegEvent::Progress(progress) => {
                if events.send(PumpEvent::Progress(progress)).is_err() {
                    break;
                }
            }
            FfmpegEvent::LogEOF => {
                let _ = events.send(PumpEvent::Eof);
                break;
            }
            _ => {}
        }
    }
}

fn handle_progress(
    progress: FfmpegProgress,
    partial_path: &Path,
    on_ready: &mut impl FnMut(AttemptEvent) -> Result<()>,
    emit_ready: bool,
    now_utc_ms: &mut impl FnMut() -> Result<i64>,
    media_zero_utc_ms: &mut Option<i64>,
    first_error: &mut Option<Error>,
) {
    let Some(progress_time) = parse_progress_time(&progress.time) else {
        record_first_error(first_error, Error::InvalidFfmpegProgress);
        return;
    };
    if media_zero_utc_ms.is_some()
        || progress.frame == 0
        || !fs::symlink_metadata(partial_path)
            .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.len() > 0)
    {
        return;
    }

    let observed_utc_ms = match now_utc_ms() {
        Ok(observed_utc_ms) => observed_utc_ms,
        Err(error) => {
            record_first_error(first_error, error);
            return;
        }
    };
    let Some(frozen_media_zero) =
        observed_utc_ms.checked_sub(progress_time.as_micros().div_euclid(1_000))
    else {
        record_first_error(first_error, Error::TimestampOverflow);
        return;
    };
    if emit_ready
        && let Err(error) = on_ready(AttemptEvent::Ready {
            media_zero_utc_ms: frozen_media_zero,
        })
    {
        record_first_error(first_error, error);
        return;
    }
    *media_zero_utc_ms = Some(frozen_media_zero);
}

fn parse_progress_time(value: &str) -> Option<FfmpegTimeDuration> {
    let value = value.trim();
    let (numeric, unit_microseconds) = if let Some(value) = value.strip_suffix("us") {
        (value, 1.0)
    } else if let Some(value) = value.strip_suffix("ms") {
        (value, 1_000.0)
    } else {
        (value, 1_000_000.0)
    };
    let components = numeric.split(':').collect::<Vec<_>>();
    if components.is_empty()
        || components.len() > 3
        || unit_microseconds != 1_000_000.0 && components.len() != 1
    {
        return None;
    }
    let mut seconds = 0.0;
    let mut multiplier = 1.0;
    for component in components.into_iter().rev() {
        let component = component.trim();
        let parsed = component.parse::<f64>().ok()?;
        if component.starts_with('-') || !parsed.is_finite() || parsed < 0.0 {
            return None;
        }
        seconds += parsed * multiplier;
        multiplier *= 60.0;
    }
    let microseconds = seconds * unit_microseconds;
    if !microseconds.is_finite() || microseconds < 0.0 || microseconds >= i64::MAX as f64 {
        return None;
    }
    FfmpegTimeDuration::from_str(value).filter(|duration| duration.as_micros() >= 0)
}

fn cleanup_child(
    child: &mut FfmpegChild,
    pump: Option<JoinHandle<()>>,
    stop_timeout: Duration,
    status: &mut Option<ExitStatus>,
    first_error: &mut Option<Error>,
) {
    match child.as_inner_mut().try_wait() {
        Ok(Some(exit_status)) => *status = Some(exit_status),
        Ok(None) => {}
        Err(error) => record_first_error(first_error, Error::Io(error)),
    }

    let mut force_kill = false;
    if status.is_none() {
        if child.quit().is_err() {
            record_first_error(first_error, Error::FfmpegQuit);
            force_kill = true;
        }

        let started = Instant::now();
        while !force_kill && status.is_none() {
            match child.as_inner_mut().try_wait() {
                Ok(Some(exit_status)) => *status = Some(exit_status),
                Ok(None) => {}
                Err(error) => {
                    record_first_error(first_error, Error::Io(error));
                    force_kill = true;
                }
            }
            if status.is_some() {
                break;
            }
            let elapsed = started.elapsed();
            if elapsed >= stop_timeout {
                force_kill = true;
                break;
            }
            thread::sleep(CHILD_POLL_INTERVAL.min(stop_timeout.saturating_sub(elapsed)));
        }
    }

    if force_kill && status.is_none() {
        tracing::warn!("forcing recorder child termination");
        if let Err(error) = child.kill() {
            record_first_error(first_error, Error::Io(error));
        }
    }
    match child.wait() {
        Ok(exit_status) => *status = Some(exit_status),
        Err(error) => record_first_error(first_error, Error::Io(error)),
    }
    if pump.is_some_and(|pump| pump.join().is_err()) {
        record_first_error(first_error, Error::FfmpegPump);
    }
}

fn drain_pump_with_ready(
    pump: &Receiver<PumpEvent>,
    partial_path: &Path,
    on_ready: &mut impl FnMut(AttemptEvent) -> Result<()>,
    now_utc_ms: &mut impl FnMut() -> Result<i64>,
    media_zero_utc_ms: &mut Option<i64>,
    first_error: &mut Option<Error>,
) {
    while let Ok(event) = pump.try_recv() {
        match event {
            PumpEvent::Progress(progress) => handle_progress(
                progress,
                partial_path,
                on_ready,
                false,
                now_utc_ms,
                media_zero_utc_ms,
                first_error,
            ),
            PumpEvent::Failed => record_first_error(first_error, Error::FfmpegParser),
            PumpEvent::Eof => {}
        }
    }
}

fn record_first_error(first_error: &mut Option<Error>, error: Error) {
    if first_error.is_none() {
        *first_error = Some(error);
    }
}

fn finalize_attempt(
    camera_id: u32,
    partial_path: &Path,
    ffprobe: &Path,
    probe_timeout: Duration,
    shutdown: &AtomicBool,
    media_zero_utc_ms: Option<i64>,
    previous_end_utc_ms: Option<i64>,
) -> Result<Option<RecordingSegment>> {
    let metadata = match fs::symlink_metadata(partial_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::Io(error)),
    };
    if !metadata.file_type().is_file() {
        return Err(Error::InvalidAttemptOutput);
    }
    if metadata.len() == 0 {
        fs::remove_file(partial_path)?;
        return Ok(None);
    }
    let Some(media_zero_utc_ms) = media_zero_utc_ms else {
        return Ok(None);
    };

    let probe = match probe_media(ffprobe, partial_path, probe_timeout, shutdown) {
        Ok(probe) => probe,
        Err(Error::InvalidMedia | Error::InvalidMediaDuration) => return Ok(None),
        Err(error) => return Err(error),
    };
    let candidate_start = media_zero_utc_ms
        .checked_add(probe.start_time_ms)
        .ok_or(Error::TimestampOverflow)?;
    let start_utc_ms = candidate_start.max(previous_end_utc_ms.unwrap_or(i64::MIN));
    let end_utc_ms = start_utc_ms
        .checked_add(probe.media_span_ms)
        .ok_or(Error::TimestampOverflow)?;
    let final_path = partial_path
        .parent()
        .ok_or(Error::InvalidAttemptOutput)?
        .join(format!("{start_utc_ms}.mkv"));

    let mut temporary = tempfile::TempPath::try_from_path(partial_path)?;
    temporary.disable_cleanup(true);
    if let Err(error) = temporary.persist_noclobber(&final_path) {
        return Err(Error::Io(error.error));
    }

    Ok(Some(RecordingSegment {
        camera_id,
        start_utc_ms,
        end_utc_ms,
        path: final_path,
    }))
}

#[cfg(all(test, unix))]
#[path = "tests/recorder.rs"]
mod tests;

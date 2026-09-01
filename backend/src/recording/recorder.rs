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
    spawn_with_executables(settings, ffmpeg, ffprobe)
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

#[cfg(all(test, unix))]
fn drain_pump(
    pump: &Receiver<PumpEvent>,
    partial_path: &Path,
    events: &Sender<AttemptEvent>,
    now_utc_ms: &mut impl FnMut() -> Result<i64>,
    media_zero_utc_ms: &mut Option<i64>,
    first_error: &mut Option<Error>,
) {
    drain_pump_with_ready(
        pump,
        partial_path,
        &mut |event| {
            events
                .send(event)
                .map_err(|_| Error::RecorderEventReceiverClosed)
        },
        now_utc_ms,
        media_zero_utc_ms,
        first_error,
    );
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
mod tests {
    use std::{
        cell::Cell,
        fs,
        future::Future,
        os::unix::fs::{PermissionsExt, symlink},
        path::{Path, PathBuf},
        process::{Command, Stdio},
        sync::{
            Arc, Condvar, Mutex,
            atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    use ffmpeg_sidecar::log_parser::try_parse_progress;
    use serde_json::json;
    use tempfile::TempDir;
    use tracing::{
        Event, Metadata, Subscriber,
        field::{Field, Visit},
        span::{Attributes, Id, Record},
    };

    use crate::recording::{Error, RecordingSegment};

    use super::{
        AttemptConfig, AttemptEvent, AttemptResult, CameraSupervisor, PumpEvent, RecorderEvent,
        RecorderSet, RecorderSettings, RecorderStatus, RecordingCamera, StartupState,
        cleanup_failed_start, drain_pump, finalize_attempt, parse_progress_time,
        run_attempt_with_clock, spawn_with_executables, supervise_camera,
    };

    const PROGRESS_ONE_SECOND: &str = "[info] frame=    1 fps=1.0 q=-1.0 size=       1kB time=00:00:01.000 bitrate=   8.0kbits/s speed=1x";

    fn write_script(directory: &TempDir, name: &str, body: &str) -> PathBuf {
        let path = directory.path().join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn attempt_config<'a>(
        ffmpeg: &'a Path,
        rtsp_url: &'a str,
        partial_path: &'a Path,
    ) -> AttemptConfig<'a> {
        AttemptConfig {
            ffmpeg,
            rtsp_url,
            partial_path,
            io_timeout: Duration::from_millis(750),
            stop_timeout: Duration::from_secs(1),
        }
    }

    fn run<F>(
        config: AttemptConfig<'_>,
        stop: &AtomicBool,
        now_utc_ms: F,
    ) -> (Result<AttemptResult, Error>, Vec<AttemptEvent>)
    where
        F: FnMut() -> Result<i64, Error>,
    {
        let shutdown = AtomicBool::new(false);
        run_with_tokens(config, stop, &shutdown, now_utc_ms)
    }

    fn run_with_tokens<F>(
        config: AttemptConfig<'_>,
        stop: &AtomicBool,
        shutdown: &AtomicBool,
        now_utc_ms: F,
    ) -> (Result<AttemptResult, Error>, Vec<AttemptEvent>)
    where
        F: FnMut() -> Result<i64, Error>,
    {
        let (events_tx, events_rx) = mpsc::channel();
        let result = run_attempt_with_clock(config, stop, shutdown, &events_tx, now_utc_ms);
        drop(events_tx);
        (result, events_rx.try_iter().collect())
    }

    fn args_path(ffmpeg: &Path) -> PathBuf {
        ffmpeg.with_extension("args")
    }

    fn marker_path(ffmpeg: &Path, extension: &str) -> PathBuf {
        ffmpeg.with_extension(extension)
    }

    fn wait_for_file(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !path.exists() {
            assert!(Instant::now() < deadline, "timed out waiting for {path:?}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_process_exit(pid: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while process_exists(pid) {
            assert!(Instant::now() < deadline, "process {pid:?} did not exit");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn process_exists(pid: &str) -> bool {
        Command::new("ps")
            .args(["-p", pid.trim()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    fn fake_ffprobe(directory: &TempDir, name: &str, output: &str, exit_code: i32) -> PathBuf {
        let ffprobe = directory.path().join(name);
        fs::write(ffprobe.with_extension("stdout"), output).unwrap();
        let body = format!(
            r#"if [ "$#" -ne 9 ] || [ "$1" != "-v" ] || [ "$2" != "error" ] || [ "$3" != "-select_streams" ] || [ "$4" != "v" ] || [ "$5" != "-show_entries" ] || [ "$6" != "stream=index:format=start_time,duration" ] || [ "$7" != "-of" ] || [ "$8" != "json" ]; then
    exit 97
fi
cat "$0.stdout"
exit {exit_code}"#
        );
        write_script(directory, name, &body);
        ffprobe
    }

    fn valid_probe(start_time: &str, duration: &str) -> String {
        json!({
            "streams": [{"index": 0}],
            "format": {"start_time": start_time, "duration": duration}
        })
        .to_string()
    }

    fn recorder_settings() -> RecorderSettings {
        RecorderSettings {
            io_timeout: Duration::from_secs(2),
            retry_delay: Duration::from_millis(100),
            stop_timeout: Duration::from_secs(1),
        }
    }

    fn preflight_executable(directory: &TempDir, name: &str, exit_code: i32) -> PathBuf {
        write_script(
            directory,
            name,
            &format!(
                r#"if [ "$#" -ne 1 ] || [ "$1" != "-version" ]; then
    printf invoked > "$0.recording-invoked"
    exit 97
fi
exit {exit_code}"#
            ),
        )
    }

    fn valid_recordings_root(directory: &TempDir, camera_ids: &[u32]) -> PathBuf {
        let root = directory.path().join("recordings");
        fs::create_dir(&root).unwrap();
        for camera_id in camera_ids {
            fs::create_dir(root.join(format!("camera-{camera_id}"))).unwrap();
        }
        root
    }

    fn startup_ffmpeg(directory: &TempDir, name: &str) -> PathBuf {
        write_script(
            directory,
            name,
            &format!(
                r#"if [ "$#" -eq 1 ] && [ "$1" = "-version" ]; then
    exit 0
fi
previous=
for argument in "$@"
do
    if [ "$previous" = "-i" ]; then
        source_url="$argument"
    fi
    previous="$argument"
    output_path="$argument"
done
scenario="${{source_url##*/}}"
printf '%s\n' "$$" > "$0.$scenario.pid"
if [ "$scenario" = "slow" ]; then
    while [ ! -f "$0.release" ]; do sleep 0.01; done
fi
if [ "$scenario" = "fail" ]; then
    while [ ! -f "$0.fail" ]; do sleep 0.01; done
    exit 23
fi
printf media > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2
if [ "$scenario" = "ready-fail" ]; then
    while [ ! -f "$0.ready-fail-exit" ]; do sleep 0.01; done
    : > "$0.ready-fail-exited"
    exit 23
fi
quit=$(dd bs=1 count=1 2>/dev/null)
printf '%s' "$quit" > "$0.$scenario.quit""#
            ),
        )
    }

    fn scenario_marker(ffmpeg: &Path, scenario: &str, suffix: &str) -> PathBuf {
        ffmpeg.with_extension(format!("{scenario}.{suffix}"))
    }

    fn wait_for_status(
        events: &mut tokio::sync::mpsc::UnboundedReceiver<RecorderEvent>,
        camera_id: u32,
        status: RecorderStatus,
    ) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match events.try_recv() {
                Ok(RecorderEvent::Status {
                    camera_id: actual_camera,
                    status: actual_status,
                    ..
                }) if actual_camera == camera_id && actual_status == status => return,
                Ok(RecorderEvent::Faulted { message, .. }) => {
                    panic!("unexpected recorder fault: {message}")
                }
                Ok(_) | Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    panic!("recorder event channel disconnected")
                }
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {status:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn supervision_ffmpeg(directory: &TempDir, name: &str) -> PathBuf {
        write_script(
            directory,
            name,
            &format!(
                r#"if [ "$#" -eq 1 ] && [ "$1" = "-version" ]; then
    exit 0
fi
previous=
for argument in "$@"
do
    if [ "$previous" = "-i" ]; then
        source_url="$argument"
    fi
    previous="$argument"
    output_path="$argument"
done
scenario="${{source_url##*/}}"
count_file="$0.$scenario.count"
count=1
if [ -f "$count_file" ]; then count=$(( $(cat "$count_file") + 1 )); fi
printf '%s' "$count" > "$count_file"
printf '%s\n' "$output_path" >> "$0.paths"
printf '%s\n' "$$" > "$0.$scenario.$count.pid"
printf media > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2
if [ "$scenario" = "reconnect" ] && [ "$count" -eq 1 ]; then
    while [ ! -f "$0.reconnect-exit" ]; do sleep 0.01; done
    exit 0
fi
if [ "$scenario" = "storage" ]; then
    while [ ! -f "$0.exit" ]; do sleep 0.01; done
    exit 0
fi
case "$scenario" in
    fatal-*) while [ ! -f "$0.fatal" ]; do sleep 0.01; done; exit 0 ;;
esac
quit=$(dd bs=1 count=1 2>/dev/null)
printf '%s' "$quit" > "$0.$scenario.$count.quit""#
            ),
        )
    }

    fn runtime_ffprobe(directory: &TempDir, name: &str, probe_body: &str) -> PathBuf {
        let path = directory.path().join(name);
        fs::write(path.with_extension("stdout"), valid_probe("0", "1")).unwrap();
        write_script(
            directory,
            name,
            &format!(
                r#"if [ "$#" -eq 1 ] && [ "$1" = "-version" ]; then
    exit 0
fi
if [ "$#" -ne 9 ] || [ "$1" != "-v" ] || [ "$2" != "error" ] || [ "$3" != "-select_streams" ] || [ "$4" != "v" ] || [ "$5" != "-show_entries" ] || [ "$6" != "stream=index:format=start_time,duration" ] || [ "$7" != "-of" ] || [ "$8" != "json" ]; then
    exit 97
fi
{probe_body}"#
            ),
        )
    }

    fn valid_runtime_ffprobe(directory: &TempDir, name: &str) -> PathBuf {
        runtime_ffprobe(directory, name, r#"cat "$0.stdout""#)
    }

    fn wait_for_fault(
        events: &mut tokio::sync::mpsc::UnboundedReceiver<RecorderEvent>,
    ) -> (Option<u32>, String) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match events.try_recv() {
                Ok(RecorderEvent::Faulted { camera_id, message }) => return (camera_id, message),
                Ok(_) | Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    panic!("recorder event channel disconnected")
                }
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for recorder fault"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    struct TraceBoundary {
        message: &'static str,
        occurrence: usize,
        event_count: AtomicUsize,
        reached: Mutex<bool>,
        reached_changed: Condvar,
        released: Mutex<bool>,
        released_changed: Condvar,
    }

    impl TraceBoundary {
        fn new(message: &'static str, occurrence: usize) -> Self {
            Self {
                message,
                occurrence,
                event_count: AtomicUsize::new(0),
                reached: Mutex::new(false),
                reached_changed: Condvar::new(),
                released: Mutex::new(false),
                released_changed: Condvar::new(),
            }
        }

        fn wait(&self) {
            let mut reached = self.reached.lock().unwrap();
            while !*reached {
                let (next, timeout) = self
                    .reached_changed
                    .wait_timeout(reached, Duration::from_secs(2))
                    .unwrap();
                assert!(!timeout.timed_out(), "trace boundary was not reached");
                reached = next;
            }
        }

        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.released_changed.notify_one();
        }
    }

    struct BoundarySubscriber {
        boundary: Arc<TraceBoundary>,
        next_span_id: AtomicU64,
    }

    impl Subscriber for BoundarySubscriber {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(self.next_span_id.fetch_add(1, Ordering::Relaxed) + 1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut message = EventMessage::default();
            event.record(&mut message);
            if message.0.as_deref() != Some(self.boundary.message)
                || self.boundary.event_count.fetch_add(1, Ordering::Relaxed) + 1
                    != self.boundary.occurrence
            {
                return;
            }

            *self.boundary.reached.lock().unwrap() = true;
            self.boundary.reached_changed.notify_one();
            let mut released = self.boundary.released.lock().unwrap();
            while !*released {
                released = self.boundary.released_changed.wait(released).unwrap();
            }
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    #[derive(Default)]
    struct EventMessage(Option<String>);

    impl Visit for EventMessage {
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "message" {
                self.0 = Some(value.into());
            }
        }

        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.0 = Some(format!("{value:?}").trim_matches('"').into());
            }
        }
    }

    #[test]
    fn spawn_rejects_missing_or_failing_ffmpeg_and_ffprobe() {
        let directory = tempfile::tempdir().unwrap();
        let success = preflight_executable(&directory, "preflight-success", 0);
        let failure = preflight_executable(&directory, "preflight-failure", 23);
        let missing = directory.path().join("missing");

        for (ffmpeg, ffprobe) in [
            (&missing, &success),
            (&failure, &success),
            (&success, &missing),
            (&success, &failure),
        ] {
            assert!(
                spawn_with_executables(
                    recorder_settings(),
                    ffmpeg.to_path_buf(),
                    ffprobe.to_path_buf(),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn hanging_preflight_is_killed_reaped_and_times_out() {
        let directory = tempfile::tempdir().unwrap();
        let hanging = write_script(
            &directory,
            "preflight-hanging",
            r#"if [ "$#" -ne 1 ] || [ "$1" != "-version" ]; then
    exit 97
fi
printf '%s\n' "$$" > "$0.pid"
exec sleep 30"#,
        );
        let success = preflight_executable(&directory, "preflight-success", 0);
        let mut settings = recorder_settings();
        settings.io_timeout = Duration::from_secs(1);
        let started = Instant::now();

        assert!(spawn_with_executables(settings, hanging.clone(), success).is_err());

        assert!(started.elapsed() < Duration::from_secs(2));
        let pid = fs::read_to_string(marker_path(&hanging, "pid")).unwrap();
        assert!(!process_exists(&pid), "preflight process {pid:?} leaked");
    }

    #[test]
    fn spawn_rejects_zero_or_unrepresentable_settings() {
        let directory = tempfile::tempdir().unwrap();
        let executable = preflight_executable(&directory, "preflight-settings", 0);
        let valid = recorder_settings();
        let invalid = [
            RecorderSettings {
                io_timeout: Duration::ZERO,
                ..valid
            },
            RecorderSettings {
                retry_delay: Duration::ZERO,
                ..valid
            },
            RecorderSettings {
                stop_timeout: Duration::ZERO,
                ..valid
            },
            RecorderSettings {
                io_timeout: Duration::MAX,
                ..valid
            },
            RecorderSettings {
                retry_delay: Duration::MAX,
                ..valid
            },
            RecorderSettings {
                stop_timeout: Duration::MAX,
                ..valid
            },
        ];

        for settings in invalid {
            assert!(
                spawn_with_executables(settings, executable.clone(), executable.clone()).is_err()
            );
        }
        assert!(!marker_path(&executable, "recording-invoked").exists());
    }

    #[tokio::test]
    async fn start_rejects_empty_duplicate_zero_and_non_rtsp_cameras() {
        let directory = tempfile::tempdir().unwrap();
        let executable = preflight_executable(&directory, "preflight-cameras", 0);
        let (runtime, handle, _events) =
            spawn_with_executables(recorder_settings(), executable.clone(), executable.clone())
                .unwrap();
        let root = valid_recordings_root(&directory, &[0, 1]);

        assert!(handle.start(Vec::new(), root.clone()).await.is_err());
        assert!(
            handle
                .start(
                    vec![RecordingCamera {
                        id: 0,
                        rtsp_url: "rtsp://camera.invalid/stream".into(),
                    }],
                    root.clone(),
                )
                .await
                .is_err()
        );
        assert!(
            handle
                .start(
                    vec![
                        RecordingCamera {
                            id: 1,
                            rtsp_url: "rtsp://camera.invalid/one".into(),
                        },
                        RecordingCamera {
                            id: 1,
                            rtsp_url: "rtsp://camera.invalid/two".into(),
                        },
                    ],
                    root.clone(),
                )
                .await
                .is_err()
        );
        let secret_url = "https://student:secret@camera.invalid/private";
        let error = handle
            .start(
                vec![RecordingCamera {
                    id: 1,
                    rtsp_url: secret_url.into(),
                }],
                root,
            )
            .await
            .unwrap_err();
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains(secret_url));
        assert!(!rendered.contains("student:secret"));
        assert!(!marker_path(&executable, "recording-invoked").exists());
        runtime.shutdown().unwrap();
    }

    #[tokio::test]
    async fn start_rejects_missing_symlinked_and_non_directory_output_paths() {
        let directory = tempfile::tempdir().unwrap();
        let executable = preflight_executable(&directory, "preflight-paths", 0);
        let (runtime, handle, _events) =
            spawn_with_executables(recorder_settings(), executable.clone(), executable.clone())
                .unwrap();
        let camera = || {
            vec![RecordingCamera {
                id: 1,
                rtsp_url: "rtsp://camera.invalid/stream".into(),
            }]
        };

        assert!(
            handle
                .start(camera(), directory.path().join("missing"))
                .await
                .is_err()
        );

        let root_file = directory.path().join("root-file");
        fs::write(&root_file, b"not a directory").unwrap();
        assert!(handle.start(camera(), root_file).await.is_err());

        let root_target = directory.path().join("root-target");
        fs::create_dir(&root_target).unwrap();
        fs::create_dir(root_target.join("camera-1")).unwrap();
        let root_link = directory.path().join("root-link");
        symlink(&root_target, &root_link).unwrap();
        assert!(handle.start(camera(), root_link).await.is_err());

        let missing_camera_root = directory.path().join("missing-camera-root");
        fs::create_dir(&missing_camera_root).unwrap();
        assert!(handle.start(camera(), missing_camera_root).await.is_err());

        let camera_file_root = directory.path().join("camera-file-root");
        fs::create_dir(&camera_file_root).unwrap();
        fs::write(camera_file_root.join("camera-1"), b"not a directory").unwrap();
        assert!(handle.start(camera(), camera_file_root).await.is_err());

        let camera_link_root = directory.path().join("camera-link-root");
        fs::create_dir(&camera_link_root).unwrap();
        let camera_target = directory.path().join("camera-target");
        fs::create_dir(&camera_target).unwrap();
        symlink(&camera_target, camera_link_root.join("camera-1")).unwrap();
        assert!(handle.start(camera(), camera_link_root).await.is_err());

        assert!(!marker_path(&executable, "recording-invoked").exists());
        runtime.shutdown().unwrap();
    }

    #[tokio::test]
    async fn start_waits_for_every_camera() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = startup_ffmpeg(&directory, "startup-all");
        let ffprobe = preflight_executable(&directory, "startup-all-probe", 0);
        let (runtime, handle, mut events) =
            spawn_with_executables(recorder_settings(), ffmpeg.clone(), ffprobe).unwrap();
        let root = valid_recordings_root(&directory, &[1, 2]);
        let start_handle = handle.clone();
        let start = tokio::spawn(async move {
            start_handle
                .start(
                    vec![
                        RecordingCamera {
                            id: 1,
                            rtsp_url: "rtsp://camera.invalid/ready".into(),
                        },
                        RecordingCamera {
                            id: 2,
                            rtsp_url: "rtsp://camera.invalid/slow".into(),
                        },
                    ],
                    root,
                )
                .await
        });
        tokio::task::yield_now().await;

        wait_for_status(&mut events, 1, RecorderStatus::Recording);
        wait_for_file(&scenario_marker(&ffmpeg, "slow", "pid"));
        assert!(
            !start.is_finished(),
            "Start completed before every camera was ready"
        );
        fs::write(marker_path(&ffmpeg, "release"), []).unwrap();

        start.await.unwrap().unwrap();
        handle.stop().await.unwrap();
        for scenario in ["ready", "slow"] {
            let pid = fs::read_to_string(scenario_marker(&ffmpeg, scenario, "pid")).unwrap();
            assert!(!process_exists(&pid), "startup process {pid:?} leaked");
        }
        runtime.shutdown().unwrap();
    }

    #[tokio::test]
    async fn one_startup_failure_stops_and_reaps_ready_cameras() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = startup_ffmpeg(&directory, "startup-rollback");
        let ffprobe = preflight_executable(&directory, "startup-rollback-probe", 0);
        let (runtime, handle, mut events) =
            spawn_with_executables(recorder_settings(), ffmpeg.clone(), ffprobe).unwrap();
        let root = valid_recordings_root(&directory, &[1, 2]);
        let start_handle = handle.clone();
        let start = tokio::spawn(async move {
            start_handle
                .start(
                    vec![
                        RecordingCamera {
                            id: 1,
                            rtsp_url: "rtsp://camera.invalid/ready".into(),
                        },
                        RecordingCamera {
                            id: 2,
                            rtsp_url: "rtsp://camera.invalid/fail".into(),
                        },
                    ],
                    root,
                )
                .await
        });
        tokio::task::yield_now().await;

        wait_for_status(&mut events, 1, RecorderStatus::Recording);
        wait_for_file(&scenario_marker(&ffmpeg, "fail", "pid"));
        fs::write(marker_path(&ffmpeg, "fail"), []).unwrap();

        assert!(matches!(
            start.await.unwrap(),
            Err(Error::RecorderStartupFailed)
        ));
        for scenario in ["ready", "fail"] {
            let pid = fs::read_to_string(scenario_marker(&ffmpeg, scenario, "pid")).unwrap();
            assert!(!process_exists(&pid), "startup process {pid:?} leaked");
        }
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, RecorderEvent::Faulted { .. }));
        }
        runtime.shutdown().unwrap();
    }

    #[test]
    fn startup_cleanup_failure_is_reported_distinctly() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (events, _receiver) = tokio::sync::mpsc::unbounded_channel();
        let recorders = RecorderSet {
            stop: Arc::new(AtomicBool::new(false)),
            shutdown,
            startup: Arc::new(Mutex::new(StartupState::Pending)),
            fault_emitted: Arc::new(AtomicBool::new(false)),
            events,
            supervisors: vec![
                CameraSupervisor {
                    thread: thread::spawn(|| {
                        Err(Error::RecorderCleanupFailed {
                            source: Box::new(Error::FfmpegQuit),
                        })
                    }),
                },
                CameraSupervisor {
                    thread: thread::spawn(|| Err(Error::RecorderStartupFailed)),
                },
            ],
        };

        let error = cleanup_failed_start(recorders, Error::RecorderStartupFailed);

        assert!(matches!(error, Error::RecorderStartupCleanupFailed));
    }

    #[test]
    fn simultaneous_ordinary_startup_failures_are_not_cleanup_failures() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (events, _receiver) = tokio::sync::mpsc::unbounded_channel();
        let recorders = RecorderSet {
            stop: Arc::new(AtomicBool::new(false)),
            shutdown,
            startup: Arc::new(Mutex::new(StartupState::Pending)),
            fault_emitted: Arc::new(AtomicBool::new(false)),
            events,
            supervisors: vec![
                CameraSupervisor {
                    thread: thread::spawn(|| Err(Error::RecorderStartupFailed)),
                },
                CameraSupervisor {
                    thread: thread::spawn(|| Err(Error::InvalidFfmpegProgress)),
                },
            ],
        };

        let error = cleanup_failed_start(recorders, Error::RecorderStartupFailed);

        assert!(matches!(error, Error::RecorderStartupFailed));
    }

    #[tokio::test]
    async fn ready_camera_failure_before_publication_fails_start_without_fault() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = startup_ffmpeg(&directory, "startup-ready-failure");
        let ffprobe = valid_runtime_ffprobe(&directory, "startup-ready-failure-probe");
        let mut settings = recorder_settings();
        settings.io_timeout = Duration::from_secs(5);
        settings.stop_timeout = Duration::from_millis(100);
        let (runtime, handle, mut events) =
            spawn_with_executables(settings, ffmpeg.clone(), ffprobe).unwrap();
        let root = valid_recordings_root(&directory, &[1, 2]);
        let start_handle = handle.clone();
        let start = tokio::spawn(async move {
            start_handle
                .start(
                    vec![
                        RecordingCamera {
                            id: 1,
                            rtsp_url: "rtsp://camera.invalid/ready-fail".into(),
                        },
                        RecordingCamera {
                            id: 2,
                            rtsp_url: "rtsp://camera.invalid/slow".into(),
                        },
                    ],
                    root,
                )
                .await
        });
        tokio::task::yield_now().await;

        wait_for_status(&mut events, 1, RecorderStatus::Recording);
        let slow_pid_path = scenario_marker(&ffmpeg, "slow", "pid");
        wait_for_file(&slow_pid_path);
        let slow_pid = fs::read_to_string(&slow_pid_path).unwrap();
        fs::write(marker_path(&ffmpeg, "ready-fail-exit"), []).unwrap();
        wait_for_file(&marker_path(&ffmpeg, "ready-fail-exited"));
        wait_for_process_exit(&slow_pid);

        let result = start.await.unwrap();
        assert!(
            result.is_err(),
            "Start succeeded after a ready camera had exited"
        );
        for scenario in ["ready-fail", "slow"] {
            let pid = fs::read_to_string(scenario_marker(&ffmpeg, scenario, "pid")).unwrap();
            assert!(!process_exists(&pid), "startup process {pid:?} leaked");
        }
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, RecorderEvent::Faulted { .. }));
        }
        runtime.shutdown().unwrap();
    }

    #[tokio::test]
    async fn failure_after_successful_start_reply_is_faulted() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = startup_ffmpeg(&directory, "startup-published-failure");
        let ffprobe = runtime_ffprobe(&directory, "startup-published-failure-probe", "printf '{'");
        let (runtime, handle, mut events) =
            spawn_with_executables(recorder_settings(), ffmpeg.clone(), ffprobe).unwrap();
        let root = valid_recordings_root(&directory, &[1]);

        handle
            .start(
                vec![RecordingCamera {
                    id: 1,
                    rtsp_url: "rtsp://camera.invalid/ready-fail".into(),
                }],
                root,
            )
            .await
            .unwrap();
        fs::write(marker_path(&ffmpeg, "ready-fail-exit"), []).unwrap();

        let (camera_id, _) = wait_for_fault(&mut events);
        assert_eq!(camera_id, Some(1));
        assert!(handle.stop().await.is_err());
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, RecorderEvent::Faulted { .. }));
        }
        let pid = fs::read_to_string(scenario_marker(&ffmpeg, "ready-fail", "pid")).unwrap();
        assert!(!process_exists(&pid), "post-start process {pid:?} leaked");
        runtime.shutdown().unwrap();
    }

    #[tokio::test]
    async fn dropped_start_reply_cancels_without_fault() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = startup_ffmpeg(&directory, "startup-dropped-reply");
        let ffprobe = valid_runtime_ffprobe(&directory, "startup-dropped-reply-probe");
        let (runtime, handle, mut events) =
            spawn_with_executables(recorder_settings(), ffmpeg.clone(), ffprobe).unwrap();
        let root = valid_recordings_root(&directory, &[1, 2]);
        let start_handle = handle.clone();
        let start = tokio::spawn(async move {
            start_handle
                .start(
                    vec![
                        RecordingCamera {
                            id: 1,
                            rtsp_url: "rtsp://camera.invalid/ready".into(),
                        },
                        RecordingCamera {
                            id: 2,
                            rtsp_url: "rtsp://camera.invalid/slow".into(),
                        },
                    ],
                    root,
                )
                .await
        });
        tokio::task::yield_now().await;

        wait_for_status(&mut events, 1, RecorderStatus::Recording);
        wait_for_file(&scenario_marker(&ffmpeg, "slow", "pid"));
        start.abort();
        assert!(start.await.unwrap_err().is_cancelled());
        fs::write(marker_path(&ffmpeg, "release"), []).unwrap();

        for scenario in ["ready", "slow"] {
            let pid = fs::read_to_string(scenario_marker(&ffmpeg, scenario, "pid")).unwrap();
            wait_for_process_exit(&pid);
        }
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, RecorderEvent::Faulted { .. }));
        }
        runtime.shutdown().unwrap();
    }

    #[tokio::test]
    async fn duplicate_start_and_stop_commands_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = startup_ffmpeg(&directory, "startup-duplicates");
        let ffprobe = preflight_executable(&directory, "startup-duplicates-probe", 0);
        let (runtime, handle, _events) =
            spawn_with_executables(recorder_settings(), ffmpeg, ffprobe).unwrap();
        let root = valid_recordings_root(&directory, &[1]);
        let cameras = vec![RecordingCamera {
            id: 1,
            rtsp_url: "rtsp://camera.invalid/ready".into(),
        }];

        handle.start(cameras.clone(), root.clone()).await.unwrap();
        assert!(handle.start(cameras, root).await.is_err());
        handle.stop().await.unwrap();
        assert!(handle.stop().await.is_err());
        runtime.shutdown().unwrap();
    }

    #[tokio::test]
    async fn ordinary_exit_finalizes_and_emits_reconnecting() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = supervision_ffmpeg(&directory, "supervision-ordinary");
        let ffprobe = valid_runtime_ffprobe(&directory, "supervision-ordinary-probe");
        let (runtime, handle, mut events) =
            spawn_with_executables(recorder_settings(), ffmpeg.clone(), ffprobe).unwrap();
        let root = valid_recordings_root(&directory, &[1]);

        handle
            .start(
                vec![RecordingCamera {
                    id: 1,
                    rtsp_url: "rtsp://camera.invalid/reconnect".into(),
                }],
                root,
            )
            .await
            .unwrap();
        fs::write(marker_path(&ffmpeg, "reconnect-exit"), []).unwrap();
        wait_for_status(&mut events, 1, RecorderStatus::Reconnecting);

        let segments = handle.stop().await.unwrap();
        assert_eq!(segments.len(), 1);
        assert!(segments[0].path.exists());
        runtime.shutdown().unwrap();
    }

    #[tokio::test]
    async fn retry_uses_a_new_partial_path_and_returns_to_recording() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = supervision_ffmpeg(&directory, "supervision-retry");
        let ffprobe = valid_runtime_ffprobe(&directory, "supervision-retry-probe");
        let (runtime, handle, mut events) =
            spawn_with_executables(recorder_settings(), ffmpeg.clone(), ffprobe).unwrap();
        let root = valid_recordings_root(&directory, &[1]);

        handle
            .start(
                vec![RecordingCamera {
                    id: 1,
                    rtsp_url: "rtsp://camera.invalid/reconnect".into(),
                }],
                root,
            )
            .await
            .unwrap();
        fs::write(marker_path(&ffmpeg, "reconnect-exit"), []).unwrap();
        wait_for_status(&mut events, 1, RecorderStatus::Reconnecting);
        wait_for_status(&mut events, 1, RecorderStatus::Recording);

        let paths = fs::read_to_string(marker_path(&ffmpeg, "paths")).unwrap();
        let paths = paths.lines().collect::<Vec<_>>();
        assert_eq!(paths.len(), 2);
        assert_ne!(paths[0], paths[1]);
        assert!(
            paths
                .iter()
                .all(|path| path.contains(".attempt-") && path.ends_with(".partial.mkv"))
        );

        let segments = handle.stop().await.unwrap();
        assert_eq!(segments.len(), 2);
        runtime.shutdown().unwrap();
    }

    #[test]
    fn startup_parser_failure_is_classified_before_child_cleanup_finishes() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = write_script(
            &directory,
            "startup-parser-failure",
            &format!(
                r#"for output_path
do
    :
done
printf '%s\n' "$$" > "$0.pid"
printf media > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2
printf '%s\n' '[info] Stream #0:0: Video: h264, yuv420p, 16x16, 1 fps' >&2
exec sleep 30"#
            ),
        );
        let root = valid_recordings_root(&directory, &[43]);
        let mut settings = recorder_settings();
        settings.stop_timeout = Duration::from_millis(200);
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(AtomicBool::new(false));
        let startup_state = Arc::new(Mutex::new(StartupState::Pending));
        let observed_startup = Arc::clone(&startup_state);
        let fault_emitted = Arc::new(AtomicBool::new(false));
        let (startup, _startup_events) = mpsc::channel();
        let (events, _event_receiver) = tokio::sync::mpsc::unbounded_channel();
        let boundary = Arc::new(TraceBoundary::new("forcing recorder child termination", 1));
        let thread_boundary = Arc::clone(&boundary);
        let supervisor = thread::spawn(move || {
            let subscriber = BoundarySubscriber {
                boundary: thread_boundary,
                next_span_id: AtomicU64::new(0),
            };
            let dispatch = tracing::Dispatch::new(subscriber);
            tracing::dispatcher::with_default(&dispatch, || {
                supervise_camera(
                    settings,
                    ffmpeg,
                    PathBuf::from("unused-ffprobe"),
                    RecordingCamera {
                        id: 43,
                        rtsp_url: "rtsp://camera.invalid/parser-failure".into(),
                    },
                    root.join("camera-43"),
                    Instant::now() + Duration::from_secs(1),
                    stop,
                    shutdown,
                    startup_state,
                    fault_emitted,
                    startup,
                    events,
                )
            })
        });

        boundary.wait();
        let classified = matches!(*observed_startup.lock().unwrap(), StartupState::Failed);
        boundary.release();

        assert!(supervisor.join().unwrap().is_err());
        let pid = fs::read_to_string(directory.path().join("startup-parser-failure.pid")).unwrap();
        assert!(
            !process_exists(&pid),
            "parser-failure process {pid:?} leaked"
        );
        assert!(
            classified,
            "parser failure remained publishable during child cleanup"
        );
    }

    #[test]
    fn stop_at_reconnect_boundary_does_not_launch_second_attempt() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = supervision_ffmpeg(&directory, "supervision-boundary-stop");
        let ffprobe = valid_runtime_ffprobe(&directory, "supervision-boundary-stop-probe");
        let root = valid_recordings_root(&directory, &[42]);
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(AtomicBool::new(false));
        let startup_state = Arc::new(Mutex::new(StartupState::Published));
        let fault_emitted = Arc::new(AtomicBool::new(false));
        let (startup, _startup_events) = mpsc::channel();
        let (events, _event_receiver) = tokio::sync::mpsc::unbounded_channel();
        let boundary = Arc::new(TraceBoundary::new("spawning recorder attempt", 2));
        fs::write(marker_path(&ffmpeg, "reconnect-exit"), []).unwrap();
        let thread_boundary = Arc::clone(&boundary);
        let thread_stop = Arc::clone(&stop);
        let supervisor = thread::spawn(move || {
            let subscriber = BoundarySubscriber {
                boundary: thread_boundary,
                next_span_id: AtomicU64::new(0),
            };
            let dispatch = tracing::Dispatch::new(subscriber);
            tracing::dispatcher::with_default(&dispatch, || {
                supervise_camera(
                    recorder_settings(),
                    ffmpeg,
                    ffprobe,
                    RecordingCamera {
                        id: 42,
                        rtsp_url: "rtsp://camera.invalid/reconnect".into(),
                    },
                    root.join("camera-42"),
                    Instant::now() + Duration::from_secs(1),
                    thread_stop,
                    shutdown,
                    startup_state,
                    fault_emitted,
                    startup,
                    events,
                )
            })
        });

        boundary.wait();
        stop.store(true, Ordering::Relaxed);
        boundary.release();

        let segments = supervisor.join().unwrap().unwrap();
        assert_eq!(segments.len(), 1);
        let paths =
            fs::read_to_string(directory.path().join("supervision-boundary-stop.paths")).unwrap();
        assert_eq!(paths.lines().count(), 1, "Stop launched a retry attempt");
        let first_pid = fs::read_to_string(
            directory
                .path()
                .join("supervision-boundary-stop.reconnect.1.pid"),
        )
        .unwrap();
        assert!(
            !process_exists(&first_pid),
            "first attempt {first_pid:?} leaked"
        );
        assert!(
            !directory
                .path()
                .join("supervision-boundary-stop.reconnect.2.pid")
                .exists(),
            "retry child was launched"
        );
    }

    #[test]
    fn stop_during_failing_storage_probe_preserves_fault() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = supervision_ffmpeg(&directory, "supervision-storage-stop");
        let ffprobe = valid_runtime_ffprobe(&directory, "supervision-storage-stop-probe");
        let root = valid_recordings_root(&directory, &[44]);
        let camera_directory = root.join("camera-44");
        let stop = Arc::new(AtomicBool::new(false));
        let observed_stop = Arc::clone(&stop);
        let shutdown = Arc::new(AtomicBool::new(false));
        let set_shutdown = Arc::clone(&shutdown);
        let startup_state = Arc::new(Mutex::new(StartupState::Published));
        let fault_emitted = Arc::new(AtomicBool::new(false));
        let (startup, _startup_events) = mpsc::channel();
        let (events, mut event_receiver) = tokio::sync::mpsc::unbounded_channel();
        let boundary = Arc::new(TraceBoundary::new("recorder storage probe failed", 1));
        let thread_boundary = Arc::clone(&boundary);
        let thread_stop = Arc::clone(&stop);
        let thread_startup = Arc::clone(&startup_state);
        let thread_fault = Arc::clone(&fault_emitted);
        let thread_events = events.clone();
        let supervisor = thread::spawn(move || {
            let subscriber = BoundarySubscriber {
                boundary: thread_boundary,
                next_span_id: AtomicU64::new(0),
            };
            let dispatch = tracing::Dispatch::new(subscriber);
            tracing::dispatcher::with_default(&dispatch, || {
                supervise_camera(
                    recorder_settings(),
                    ffmpeg,
                    ffprobe,
                    RecordingCamera {
                        id: 44,
                        rtsp_url: "rtsp://student:secret@camera.invalid/storage".into(),
                    },
                    camera_directory,
                    Instant::now() + Duration::from_secs(1),
                    thread_stop,
                    shutdown,
                    thread_startup,
                    thread_fault,
                    startup,
                    thread_events,
                )
            })
        });
        let recorders = RecorderSet {
            stop,
            shutdown: set_shutdown,
            startup: startup_state,
            fault_emitted,
            events,
            supervisors: vec![CameraSupervisor { thread: supervisor }],
        };

        wait_for_status(&mut event_receiver, 44, RecorderStatus::Recording);
        let partial =
            fs::read_to_string(directory.path().join("supervision-storage-stop.paths")).unwrap();
        fs::remove_file(partial.trim()).unwrap();
        fs::remove_dir(root.join("camera-44")).unwrap();
        fs::write(directory.path().join("supervision-storage-stop.exit"), []).unwrap();
        boundary.wait();

        let stopper = thread::spawn(move || recorders.stop());
        let deadline = Instant::now() + Duration::from_secs(2);
        while !observed_stop.load(Ordering::Relaxed) {
            assert!(
                Instant::now() < deadline,
                "Stop did not signal the supervisor"
            );
            thread::sleep(Duration::from_millis(10));
        }
        boundary.release();

        assert!(stopper.join().unwrap().is_err());
        let (camera_id, message) = wait_for_fault(&mut event_receiver);
        assert_eq!(camera_id, Some(44));
        assert!(!message.contains("student:secret"));
        assert!(!message.contains("rtsp://"));
        while let Ok(event) = event_receiver.try_recv() {
            assert!(!matches!(event, RecorderEvent::Faulted { .. }));
        }
        let pid = fs::read_to_string(
            directory
                .path()
                .join("supervision-storage-stop.storage.1.pid"),
        )
        .unwrap();
        assert!(
            !process_exists(&pid),
            "storage-boundary process {pid:?} leaked"
        );
        assert!(
            !directory
                .path()
                .join("supervision-storage-stop.storage.2.pid")
                .exists(),
            "storage failure launched a retry child"
        );
    }

    #[tokio::test]
    async fn storage_probe_failure_emits_faulted() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = supervision_ffmpeg(&directory, "supervision-storage");
        let ffprobe = valid_runtime_ffprobe(&directory, "supervision-storage-probe");
        let (runtime, handle, mut events) =
            spawn_with_executables(recorder_settings(), ffmpeg.clone(), ffprobe).unwrap();
        let root = valid_recordings_root(&directory, &[1]);
        let camera_directory = root.join("camera-1");

        handle
            .start(
                vec![RecordingCamera {
                    id: 1,
                    rtsp_url: "rtsp://student:secret@camera.invalid/storage".into(),
                }],
                root,
            )
            .await
            .unwrap();
        wait_for_status(&mut events, 1, RecorderStatus::Recording);
        let partial = fs::read_to_string(marker_path(&ffmpeg, "paths")).unwrap();
        fs::remove_file(partial.trim()).unwrap();
        fs::remove_dir(camera_directory).unwrap();
        fs::write(marker_path(&ffmpeg, "exit"), []).unwrap();

        let (camera_id, message) = wait_for_fault(&mut events);
        assert_eq!(camera_id, Some(1));
        assert!(!message.contains("student:secret"));
        assert!(!message.contains("rtsp://"));
        thread::sleep(Duration::from_millis(100));
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, RecorderEvent::Faulted { .. }));
        }
        assert!(handle.stop().await.is_err());
        runtime.shutdown().unwrap();
    }

    #[tokio::test]
    async fn simultaneous_post_start_failures_emit_one_faulted() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = supervision_ffmpeg(&directory, "supervision-double-fatal");
        let ffprobe = runtime_ffprobe(
            &directory,
            "supervision-double-fatal-probe",
            r#"case "$9" in
    */camera-1/*) marker="$0.camera-1" ;;
    */camera-2/*) marker="$0.camera-2" ;;
    *) exit 98 ;;
esac
: > "$marker"
while [ ! -f "$0.camera-1" ] || [ ! -f "$0.camera-2" ]; do sleep 0.01; done
printf '{'"#,
        );
        let (runtime, handle, mut events) =
            spawn_with_executables(recorder_settings(), ffmpeg.clone(), ffprobe).unwrap();
        let root = valid_recordings_root(&directory, &[1, 2]);

        handle
            .start(
                vec![
                    RecordingCamera {
                        id: 1,
                        rtsp_url: "rtsp://camera.invalid/fatal-1".into(),
                    },
                    RecordingCamera {
                        id: 2,
                        rtsp_url: "rtsp://camera.invalid/fatal-2".into(),
                    },
                ],
                root,
            )
            .await
            .unwrap();
        fs::write(marker_path(&ffmpeg, "fatal"), []).unwrap();

        let _ = wait_for_fault(&mut events);
        assert!(handle.stop().await.is_err());
        let mut additional_faults = 0;
        while let Ok(event) = events.try_recv() {
            additional_faults += usize::from(matches!(event, RecorderEvent::Faulted { .. }));
        }
        assert_eq!(additional_faults, 0);
        for scenario in ["fatal-1", "fatal-2"] {
            let pid = fs::read_to_string(scenario_marker(&ffmpeg, scenario, "1.pid")).unwrap();
            assert!(!process_exists(&pid), "fatal process {pid:?} leaked");
        }
        runtime.shutdown().unwrap();
    }

    #[tokio::test]
    async fn stop_finalizes_and_reaps_every_camera_concurrently() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = supervision_ffmpeg(&directory, "supervision-concurrent");
        let ffprobe = runtime_ffprobe(
            &directory,
            "supervision-concurrent-probe",
            r#"case "$9" in
    */camera-1/*) marker="$0.camera-1" ;;
    */camera-2/*) marker="$0.camera-2" ;;
    *) exit 98 ;;
esac
: > "$marker"
while [ ! -f "$0.camera-1" ] || [ ! -f "$0.camera-2" ]; do sleep 0.01; done
cat "$0.stdout""#,
        );
        let mut settings = recorder_settings();
        settings.io_timeout = Duration::from_secs(2);
        let (runtime, handle, _events) =
            spawn_with_executables(settings, ffmpeg.clone(), ffprobe).unwrap();
        let root = valid_recordings_root(&directory, &[1, 2]);

        handle
            .start(
                vec![
                    RecordingCamera {
                        id: 1,
                        rtsp_url: "rtsp://camera.invalid/camera-1".into(),
                    },
                    RecordingCamera {
                        id: 2,
                        rtsp_url: "rtsp://camera.invalid/camera-2".into(),
                    },
                ],
                root,
            )
            .await
            .unwrap();
        let started = Instant::now();

        let segments = handle.stop().await.unwrap();

        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(
            directory
                .path()
                .join("supervision-concurrent-probe.camera-1")
                .exists(),
            "camera 1 FFprobe was not invoked"
        );
        assert!(
            directory
                .path()
                .join("supervision-concurrent-probe.camera-2")
                .exists(),
            "camera 2 FFprobe was not invoked"
        );
        assert_eq!(
            segments
                .iter()
                .map(|segment| segment.camera_id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        for scenario in ["camera-1", "camera-2"] {
            let pid = fs::read_to_string(scenario_marker(&ffmpeg, scenario, "1.pid")).unwrap();
            assert!(!process_exists(&pid), "recorder process {pid:?} leaked");
        }
        runtime.shutdown().unwrap();
    }

    #[tokio::test]
    async fn hanging_stop_probe_times_out_without_leaking_a_process() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = supervision_ffmpeg(&directory, "supervision-hanging-probe");
        let ffprobe = runtime_ffprobe(
            &directory,
            "supervision-hanging-probe-ffprobe",
            r#"printf '%s\n' "$$" > "$0.pid"
exec sleep 30"#,
        );
        let mut settings = recorder_settings();
        settings.io_timeout = Duration::from_secs(1);
        let (runtime, handle, mut events) =
            spawn_with_executables(settings, ffmpeg.clone(), ffprobe.clone()).unwrap();
        let root = valid_recordings_root(&directory, &[1]);
        handle
            .start(
                vec![RecordingCamera {
                    id: 1,
                    rtsp_url: "rtsp://camera.invalid/hold".into(),
                }],
                root,
            )
            .await
            .unwrap();
        let started = Instant::now();

        assert!(handle.stop().await.is_err());

        assert!(started.elapsed() < Duration::from_secs(2));
        let pid = fs::read_to_string(marker_path(&ffprobe, "pid")).unwrap();
        assert!(!process_exists(&pid), "FFprobe process {pid:?} leaked");
        let (camera_id, _) = wait_for_fault(&mut events);
        assert_eq!(camera_id, Some(1));
        let ffmpeg_pid = fs::read_to_string(scenario_marker(&ffmpeg, "hold", "1.pid")).unwrap();
        assert!(
            !process_exists(&ffmpeg_pid),
            "FFmpeg process {ffmpeg_pid:?} leaked"
        );
        runtime.shutdown().unwrap();
    }

    #[test]
    fn shutdown_interrupts_initial_readiness_and_reaps_children() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = write_script(
            &directory,
            "shutdown-startup",
            r#"if [ "$#" -eq 1 ] && [ "$1" = "-version" ]; then
    exit 0
fi
printf '%s\n' "$$" > "$0.pid"
exec sleep 30"#,
        );
        let ffprobe = valid_runtime_ffprobe(&directory, "shutdown-startup-probe");
        let mut settings = recorder_settings();
        settings.io_timeout = Duration::from_secs(5);
        settings.stop_timeout = Duration::from_millis(100);
        let (runtime, handle, _events) =
            spawn_with_executables(settings, ffmpeg.clone(), ffprobe).unwrap();
        let root = valid_recordings_root(&directory, &[1]);
        let start_handle = handle.clone();
        let start = thread::spawn(move || {
            block_on(start_handle.start(
                vec![RecordingCamera {
                    id: 1,
                    rtsp_url: "rtsp://camera.invalid/never-ready".into(),
                }],
                root,
            ))
        });
        wait_for_file(&marker_path(&ffmpeg, "pid"));
        let pid = fs::read_to_string(marker_path(&ffmpeg, "pid")).unwrap();
        let started = Instant::now();

        runtime.shutdown().unwrap();

        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(start.join().unwrap().is_err());
        assert!(!process_exists(&pid), "startup process {pid:?} leaked");
        assert!(block_on(handle.stop()).is_err());
    }

    #[test]
    fn shutdown_interrupts_reconnect_delay() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = supervision_ffmpeg(&directory, "shutdown-reconnect-delay");
        let ffprobe = valid_runtime_ffprobe(&directory, "shutdown-reconnect-delay-probe");
        let mut settings = recorder_settings();
        settings.retry_delay = Duration::from_secs(5);
        let (runtime, handle, mut events) =
            spawn_with_executables(settings, ffmpeg.clone(), ffprobe).unwrap();
        let root = valid_recordings_root(&directory, &[1]);
        block_on(handle.start(
            vec![RecordingCamera {
                id: 1,
                rtsp_url: "rtsp://camera.invalid/reconnect".into(),
            }],
            root,
        ))
        .unwrap();
        fs::write(marker_path(&ffmpeg, "reconnect-exit"), []).unwrap();
        wait_for_status(&mut events, 1, RecorderStatus::Reconnecting);
        let pid = fs::read_to_string(scenario_marker(&ffmpeg, "reconnect", "1.pid")).unwrap();
        let started = Instant::now();

        drop(runtime);

        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!process_exists(&pid), "reconnect process {pid:?} leaked");
        assert_eq!(
            fs::read_to_string(marker_path(&ffmpeg, "paths"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert!(block_on(handle.stop()).is_err());
    }

    #[test]
    fn shutdown_during_stop_finalization_still_joins_runtime() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = supervision_ffmpeg(&directory, "shutdown-stop-finalization");
        let ffprobe = runtime_ffprobe(
            &directory,
            "shutdown-stop-finalization-probe",
            r#"printf '%s\n' "$$" > "$0.pid"
exec sleep 30"#,
        );
        let mut settings = recorder_settings();
        settings.io_timeout = Duration::from_secs(5);
        let (runtime, handle, _events) =
            spawn_with_executables(settings, ffmpeg.clone(), ffprobe.clone()).unwrap();
        let root = valid_recordings_root(&directory, &[1]);
        block_on(handle.start(
            vec![RecordingCamera {
                id: 1,
                rtsp_url: "rtsp://camera.invalid/hold".into(),
            }],
            root,
        ))
        .unwrap();
        let stop_handle = handle.clone();
        let stop = thread::spawn(move || block_on(stop_handle.stop()));
        wait_for_file(&marker_path(&ffprobe, "pid"));
        let probe_pid = fs::read_to_string(marker_path(&ffprobe, "pid")).unwrap();
        let ffmpeg_pid = fs::read_to_string(scenario_marker(&ffmpeg, "hold", "1.pid")).unwrap();
        let started = Instant::now();

        runtime.shutdown().unwrap();

        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(matches!(stop.join().unwrap(), Err(Error::Shutdown)));
        assert!(
            !process_exists(&probe_pid),
            "FFprobe process {probe_pid:?} leaked"
        );
        assert!(
            !process_exists(&ffmpeg_pid),
            "FFmpeg process {ffmpeg_pid:?} leaked"
        );
    }

    #[test]
    fn shutdown_interrupts_ffprobe_and_reaps_it() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = supervision_ffmpeg(&directory, "shutdown-active-probe");
        let ffprobe = runtime_ffprobe(
            &directory,
            "shutdown-active-probe-ffprobe",
            r#"printf '%s\n' "$$" > "$0.pid"
exec sleep 30"#,
        );
        let mut settings = recorder_settings();
        settings.io_timeout = Duration::from_secs(5);
        let (runtime, handle, _events) =
            spawn_with_executables(settings, ffmpeg.clone(), ffprobe.clone()).unwrap();
        let root = valid_recordings_root(&directory, &[1]);
        block_on(handle.start(
            vec![RecordingCamera {
                id: 1,
                rtsp_url: "rtsp://camera.invalid/reconnect".into(),
            }],
            root,
        ))
        .unwrap();
        fs::write(marker_path(&ffmpeg, "reconnect-exit"), []).unwrap();
        wait_for_file(&marker_path(&ffprobe, "pid"));
        let pid = fs::read_to_string(marker_path(&ffprobe, "pid")).unwrap();
        assert!(process_exists(&pid));
        let started = Instant::now();

        runtime.shutdown().unwrap();

        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!process_exists(&pid), "FFprobe process {pid:?} leaked");
        assert!(block_on(handle.stop()).is_err());
    }

    #[test]
    fn ffmpeg_command_uses_tcp_timeout_video_copy_and_matroska() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = write_script(
            &directory,
            "ffmpeg-command",
            &format!(
                r#"FAKE_FFMPEG_ARGS="$0.args"
printf '%s\n' "$@" > "$FAKE_FFMPEG_ARGS"
for output_path
do
    :
done
printf media > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2
quit=$(dd bs=1 count=1 2>/dev/null)
printf '%s' "$quit" > "$0.quit""#
            ),
        );
        let partial_path = directory.path().join(".attempt-command.partial.mkv");
        let command_args = args_path(&ffmpeg);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_signal = Arc::clone(&stop);
        let stopper = thread::spawn(move || {
            wait_for_file(&command_args);
            stop_signal.store(true, Ordering::Relaxed);
        });

        run(
            attempt_config(
                &ffmpeg,
                "rtsp://camera.invalid/axis-media/media.amp",
                &partial_path,
            ),
            &stop,
            || Ok(10_000),
        )
        .0
        .unwrap();
        stopper.join().unwrap();

        let args = fs::read_to_string(args_path(&ffmpeg)).unwrap();
        assert_eq!(
            args.lines().collect::<Vec<_>>(),
            vec![
                "-loglevel",
                "level+info",
                "-hide_banner",
                "-n",
                "-rtsp_transport",
                "tcp",
                "-timeout",
                "750000",
                "-i",
                "rtsp://camera.invalid/axis-media/media.amp",
                "-map",
                "0:v:0",
                "-an",
                "-c:v",
                "copy",
                "-avoid_negative_ts",
                "make_zero",
                "-f",
                "matroska",
                partial_path.to_str().unwrap(),
            ]
        );
    }

    #[test]
    fn timeout_microseconds_are_checked_before_command_creation() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = write_script(
            &directory,
            "ffmpeg-timeout",
            r#"printf invoked > "$0.invoked""#,
        );
        let partial_path = directory.path().join(".attempt-timeout.partial.mkv");
        let stop = AtomicBool::new(true);
        let mut config = attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path);
        config.io_timeout = Duration::MAX;

        let error = run(config, &stop, || Ok(10_000)).0.unwrap_err();

        assert!(!format!("{error:?} {error}").contains("camera.invalid"));
        assert!(!marker_path(&ffmpeg, "invoked").exists());
        assert!(!args_path(&ffmpeg).exists());
    }

    #[test]
    fn recorder_errors_and_events_never_expose_rtsp_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = write_script(
            &directory,
            "ffmpeg-secret",
            r#"previous=
for argument in "$@"
do
    if [ "$previous" = "-i" ]; then
        source_url="$argument"
        break
    fi
    previous="$argument"
done
printf '%s\n' "$$" > "$0.pid"
printf '[info] Stream #0:0: Video: h264, yuv420p, 16x16, 1 fps, source=%s\n' "$source_url" >&2
exec sleep 30"#,
        );
        let partial_path = directory.path().join(".attempt-secret.partial.mkv");
        let secret_url = "rtsp://student:secret@camera.invalid/private";
        let stop = AtomicBool::new(false);
        let (result, events) = run(
            attempt_config(&ffmpeg, secret_url, &partial_path),
            &stop,
            || Ok(10_000),
        );

        let error = result.unwrap_err();
        let rendered = format!("{error:?} {error} {events:?}");
        assert!(!rendered.contains(secret_url));
        assert!(!rendered.contains("student:secret"));
        let pid = fs::read_to_string(marker_path(&ffmpeg, "pid")).unwrap();
        assert!(!process_exists(&pid), "fake FFmpeg process {pid:?} leaked");
    }

    #[test]
    fn first_qualifying_progress_freezes_media_timeline_zero() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = write_script(
            &directory,
            "ffmpeg-first-progress",
            r#"for output_path
do
    :
done
printf media > "$output_path"
printf '%s\r' '[info] frame=    1 fps=1.0 q=-1.0 size=       1kB time=00:00:02.500 bitrate=   3.2kbits/s speed=1x' >&2"#,
        );
        let partial_path = directory.path().join(".attempt-first.partial.mkv");
        let stop = AtomicBool::new(false);

        let (result, events) = run(
            attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path),
            &stop,
            || Ok(10_000),
        );

        assert_eq!(result.unwrap().media_zero_utc_ms, Some(7_500));
        assert_eq!(
            events,
            vec![AttemptEvent::Ready {
                media_zero_utc_ms: 7_500
            }]
        );
    }

    #[test]
    fn later_progress_does_not_move_media_timeline_zero() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = write_script(
            &directory,
            "ffmpeg-later-progress",
            &format!(
                r#"for output_path
do
    :
done
printf media > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2
printf '%s\r' '[info] frame=    2 fps=1.0 q=-1.0 size=       2kB time=00:00:05.000 bitrate=   3.2kbits/s speed=1x' >&2"#
            ),
        );
        let partial_path = directory.path().join(".attempt-later.partial.mkv");
        let stop = AtomicBool::new(false);
        let clock_calls = Cell::new(0);

        let (result, events) = run(
            attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path),
            &stop,
            || {
                clock_calls.set(clock_calls.get() + 1);
                Ok(if clock_calls.get() == 1 {
                    10_000
                } else {
                    100_000
                })
            },
        );

        assert_eq!(clock_calls.get(), 1);
        assert_eq!(result.unwrap().media_zero_utc_ms, Some(9_000));
        assert_eq!(
            events,
            vec![AttemptEvent::Ready {
                media_zero_utc_ms: 9_000
            }]
        );
    }

    #[test]
    fn terminating_drain_freezes_queued_progress_without_ready_event() {
        let directory = tempfile::tempdir().unwrap();
        let partial_path = directory.path().join(".attempt-queued.partial.mkv");
        fs::write(&partial_path, b"media").unwrap();
        let (pump_tx, pump_rx) = mpsc::channel();
        pump_tx
            .send(PumpEvent::Progress(
                try_parse_progress(PROGRESS_ONE_SECOND).unwrap(),
            ))
            .unwrap();
        let (events_tx, events_rx) = mpsc::channel();
        let mut media_zero_utc_ms = None;
        let mut first_error = None;

        drain_pump(
            &pump_rx,
            &partial_path,
            &events_tx,
            &mut || Ok(10_000),
            &mut media_zero_utc_ms,
            &mut first_error,
        );

        assert_eq!(media_zero_utc_ms, Some(9_000));
        assert!(first_error.is_none());
        assert!(matches!(
            events_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn graceful_cleanup_keeps_final_progress_without_reporting_ready() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = write_script(
            &directory,
            "ffmpeg-final-progress",
            &format!(
                r#"for output_path
do
    :
done
sleep 0.5
printf media > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2"#
            ),
        );
        let partial_path = directory.path().join(".attempt-final-progress.partial.mkv");
        let stop = AtomicBool::new(true);

        let mut config = attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path);
        config.stop_timeout = Duration::from_secs(1);
        let (result, events) = run(config, &stop, || Ok(10_000));

        assert_eq!(result.unwrap().media_zero_utc_ms, Some(9_000));
        assert!(events.is_empty());
    }

    #[test]
    fn progress_time_rejects_negative_and_non_finite_values() {
        for value in ["-00:00:00.000", "-0.0000001", "01:-01:00", "NaN", "inf"] {
            assert!(parse_progress_time(value).is_none(), "accepted {value}");
        }
        assert_eq!(
            parse_progress_time("00:00:01.250").unwrap().as_micros(),
            1_250_000
        );
    }

    #[test]
    fn progress_time_rejects_values_outside_i64_microseconds() {
        for value in ["1e300", "1e300ms", "1e300us"] {
            assert!(parse_progress_time(value).is_none(), "accepted {value}");
        }
    }

    #[test]
    fn readiness_requires_a_frame_and_nonempty_regular_output() {
        let directory = tempfile::tempdir().unwrap();
        let cases = [
            (
                "ffmpeg-zero-frame",
                format!(
                    r#"for output_path
do
    :
done
printf media > "$output_path"
printf '%s\r' '{}' >&2"#,
                    PROGRESS_ONE_SECOND.replace("frame=    1", "frame=    0")
                ),
                false,
            ),
            (
                "ffmpeg-empty-output",
                format!(
                    r#"for output_path
do
    :
done
: > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2"#
                ),
                false,
            ),
            (
                "ffmpeg-directory-output",
                format!(
                    r#"for output_path
do
    :
done
mkdir "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2"#
                ),
                false,
            ),
            (
                "ffmpeg-ready",
                format!(
                    r#"for output_path
do
    :
done
printf media > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2"#
                ),
                true,
            ),
        ];

        for (name, body, expected_ready) in cases {
            let ffmpeg = write_script(&directory, name, &body);
            let partial_path = directory.path().join(format!(".{name}.partial.mkv"));
            let stop = AtomicBool::new(false);
            let (result, events) = run(
                attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path),
                &stop,
                || Ok(10_000),
            );

            assert_eq!(
                result.unwrap().media_zero_utc_ms.is_some(),
                expected_ready,
                "unexpected readiness for {name}"
            );
            assert_eq!(
                !events.is_empty(),
                expected_ready,
                "unexpected readiness event for {name}"
            );
        }
    }

    #[test]
    fn graceful_stop_sends_q_and_reaps() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = write_script(
            &directory,
            "ffmpeg-graceful",
            &format!(
                r#"for output_path
do
    :
done
printf media > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2
printf '%s\n' "$$" > "$0.pid"
quit=$(dd bs=1 count=1 2>/dev/null)
printf '%s' "$quit" > "$0.quit""#
            ),
        );
        let partial_path = directory.path().join(".attempt-graceful.partial.mkv");
        let pid_path = marker_path(&ffmpeg, "pid");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_signal = Arc::clone(&stop);
        let stopper = thread::spawn(move || {
            wait_for_file(&pid_path);
            stop_signal.store(true, Ordering::Relaxed);
        });

        let result = run(
            attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path),
            &stop,
            || Ok(10_000),
        )
        .0
        .unwrap();
        stopper.join().unwrap();

        assert!(result.stopped);
        assert_eq!(
            fs::read_to_string(marker_path(&ffmpeg, "quit")).unwrap(),
            "q"
        );
        let pid = fs::read_to_string(marker_path(&ffmpeg, "pid")).unwrap();
        assert!(!process_exists(&pid), "fake FFmpeg process {pid:?} leaked");
    }

    #[test]
    fn stop_timeout_kills_and_reaps() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = write_script(
            &directory,
            "ffmpeg-forced",
            r#"printf '%s\n' "$$" > "$0.pid"
exec sleep 30"#,
        );
        let partial_path = directory.path().join(".attempt-forced.partial.mkv");
        let pid_path = marker_path(&ffmpeg, "pid");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_signal = Arc::clone(&stop);
        let stopper = thread::spawn(move || {
            wait_for_file(&pid_path);
            stop_signal.store(true, Ordering::Relaxed);
        });
        let mut config = attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path);
        config.stop_timeout = Duration::from_millis(100);
        let started = Instant::now();

        let result = run(config, &stop, || Ok(10_000)).0.unwrap();
        stopper.join().unwrap();

        assert!(result.stopped);
        assert!(started.elapsed() < Duration::from_secs(2));
        let pid = fs::read_to_string(marker_path(&ffmpeg, "pid")).unwrap();
        assert!(!process_exists(&pid), "fake FFmpeg process {pid:?} leaked");
    }

    #[test]
    fn shutdown_token_stops_kills_and_reaps() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = write_script(
            &directory,
            "ffmpeg-shutdown",
            r#"printf '%s\n' "$$" > "$0.pid"
exec sleep 30"#,
        );
        let partial_path = directory.path().join(".attempt-shutdown.partial.mkv");
        let pid_path = marker_path(&ffmpeg, "pid");
        let stop = AtomicBool::new(false);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_signal = Arc::clone(&shutdown);
        let shutdown_setter = thread::spawn(move || {
            wait_for_file(&pid_path);
            shutdown_signal.store(true, Ordering::Relaxed);
        });
        let mut config = attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path);
        config.stop_timeout = Duration::from_millis(100);
        let started = Instant::now();

        let result = run_with_tokens(config, &stop, &shutdown, || Ok(10_000))
            .0
            .unwrap();
        shutdown_setter.join().unwrap();

        assert!(result.stopped);
        assert!(started.elapsed() < Duration::from_secs(2));
        let pid = fs::read_to_string(marker_path(&ffmpeg, "pid")).unwrap();
        assert!(!process_exists(&pid), "fake FFmpeg process {pid:?} leaked");
    }

    #[test]
    fn failed_q_forces_kill_and_reap_preserving_first_error() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = write_script(
            &directory,
            "ffmpeg-failed-q",
            r#"exec /bin/sh -c 'printf "%s\n" "$$" > "$1"; exec sleep 30' failed-q "$0.pid" 0<&-"#,
        );
        let partial_path = directory.path().join(".attempt-failed-q.partial.mkv");
        let pid_path = marker_path(&ffmpeg, "pid");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_signal = Arc::clone(&stop);
        let stopper = thread::spawn(move || {
            wait_for_file(&pid_path);
            stop_signal.store(true, Ordering::Relaxed);
        });
        let mut config = attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path);
        config.stop_timeout = Duration::from_secs(5);
        let started = Instant::now();

        let error = run(config, &stop, || Ok(10_000)).0.unwrap_err();
        stopper.join().unwrap();

        assert!(matches!(
            error,
            Error::RecorderCleanupFailed { source }
                if matches!(*source, Error::FfmpegQuit)
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        let pid = fs::read_to_string(marker_path(&ffmpeg, "pid")).unwrap();
        assert!(!process_exists(&pid), "fake FFmpeg process {pid:?} leaked");
    }

    #[test]
    fn natural_nonzero_exit_and_eof_is_an_ordinary_attempt() {
        let directory = tempfile::tempdir().unwrap();
        let ffmpeg = write_script(
            &directory,
            "ffmpeg-nonzero",
            r#"printf '%s\n' "$$" > "$0.pid"
exit 23"#,
        );
        let partial_path = directory.path().join(".attempt-nonzero.partial.mkv");
        let stop = AtomicBool::new(false);
        let started = Instant::now();

        let (result, events) = run(
            attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path),
            &stop,
            || Ok(10_000),
        );
        let result = result.unwrap();

        assert!(!result.stopped);
        assert_eq!(result.media_zero_utc_ms, None);
        assert!(events.is_empty());
        assert!(started.elapsed() < Duration::from_secs(2));
        let pid = fs::read_to_string(marker_path(&ffmpeg, "pid")).unwrap();
        assert!(!process_exists(&pid), "fake FFmpeg process {pid:?} leaked");
    }

    #[test]
    fn positive_container_start_adjusts_segment_bounds() {
        let directory = tempfile::tempdir().unwrap();
        let partial_path = directory.path().join(".attempt-positive.partial.mkv");
        fs::write(&partial_path, b"media").unwrap();
        let ffprobe = fake_ffprobe(
            &directory,
            "ffprobe-positive",
            &valid_probe("0.0679", "2.0001"),
            0,
        );

        let segment = finalize_attempt(
            7,
            &partial_path,
            &ffprobe,
            Duration::from_secs(1),
            &AtomicBool::new(false),
            Some(10_000),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            segment,
            RecordingSegment {
                camera_id: 7,
                start_utc_ms: 10_067,
                end_utc_ms: 12_001,
                path: directory.path().join("10067.mkv"),
            }
        );
        assert!(!partial_path.exists());
    }

    #[test]
    fn reconnect_start_is_clamped_to_previous_end() {
        let directory = tempfile::tempdir().unwrap();
        let partial_path = directory.path().join(".attempt-reconnect.partial.mkv");
        fs::write(&partial_path, b"media").unwrap();
        let ffprobe = fake_ffprobe(&directory, "ffprobe-reconnect", &valid_probe("0", "1"), 0);

        let segment = finalize_attempt(
            3,
            &partial_path,
            &ffprobe,
            Duration::from_secs(1),
            &AtomicBool::new(false),
            Some(9_500),
            Some(10_000),
        )
        .unwrap()
        .unwrap();

        assert_eq!(segment.start_utc_ms, 10_000);
        assert_eq!(segment.end_utc_ms, 11_000);
        assert_eq!(segment.path, directory.path().join("10000.mkv"));
    }

    #[test]
    fn valid_attempt_is_promoted_without_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        let ffprobe = fake_ffprobe(&directory, "ffprobe-promote", &valid_probe("0", "1"), 0);
        let shutdown = AtomicBool::new(false);
        let first_partial = directory.path().join(".attempt-promote-first.partial.mkv");
        fs::write(&first_partial, b"first media").unwrap();

        let segment = finalize_attempt(
            1,
            &first_partial,
            &ffprobe,
            Duration::from_secs(1),
            &shutdown,
            Some(5_000),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(segment.path, directory.path().join("5000.mkv"));
        assert_eq!(fs::read(&segment.path).unwrap(), b"first media");
        assert!(!first_partial.exists());

        let second_partial = directory.path().join(".attempt-promote-second.partial.mkv");
        fs::write(&second_partial, b"second media").unwrap();
        finalize_attempt(
            1,
            &second_partial,
            &ffprobe,
            Duration::from_secs(1),
            &shutdown,
            Some(5_000),
            None,
        )
        .unwrap_err();

        assert_eq!(fs::read(&segment.path).unwrap(), b"first media");
        assert_eq!(fs::read(&second_partial).unwrap(), b"second media");
    }

    #[test]
    fn empty_attempt_is_removed() {
        let directory = tempfile::tempdir().unwrap();
        let partial_path = directory.path().join(".attempt-empty.partial.mkv");
        fs::write(&partial_path, []).unwrap();
        let ffprobe = write_script(
            &directory,
            "ffprobe-empty",
            r#"printf invoked > "$0.invoked"
exit 99"#,
        );

        let segment = finalize_attempt(
            1,
            &partial_path,
            &ffprobe,
            Duration::from_secs(1),
            &AtomicBool::new(false),
            Some(5_000),
            None,
        )
        .unwrap();

        assert!(segment.is_none());
        assert!(!partial_path.exists());
        assert!(!marker_path(&ffprobe, "invoked").exists());
    }

    #[test]
    fn invalid_nonempty_attempt_is_retained() {
        let directory = tempfile::tempdir().unwrap();
        let partial_path = directory.path().join(".attempt-invalid.partial.mkv");
        fs::write(&partial_path, b"invalid media").unwrap();
        let ffprobe = fake_ffprobe(
            &directory,
            "ffprobe-invalid",
            &json!({
                "streams": [],
                "format": {"start_time": "0", "duration": "1"}
            })
            .to_string(),
            0,
        );

        let segment = finalize_attempt(
            1,
            &partial_path,
            &ffprobe,
            Duration::from_secs(1),
            &AtomicBool::new(false),
            Some(5_000),
            None,
        )
        .unwrap();

        assert!(segment.is_none());
        assert_eq!(fs::read(&partial_path).unwrap(), b"invalid media");
    }
}

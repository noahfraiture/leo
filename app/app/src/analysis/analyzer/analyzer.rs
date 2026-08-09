use std::{
    collections::{BTreeMap, HashSet},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use rig_core::{
    OneOrMany,
    completion::{CompletionModel, Message},
    message::{ImageMediaType, UserContent},
};
use tempfile::TempDir;

use crate::{
    analysis::{
        agent::{Agent, AnalysisResponse},
        video::{Frame, FrameSet, SampleSequence, SamplingSchedule, Video, extract_jpeg},
    },
    recording::SynologyClient,
    session::Session,
};

use super::{
    error::{Error, Result},
    progress::{AnalysisCheckpoint, CompletedBatch},
};

/// Plans catalogue-backed samples, materializes one batch at a time, and checkpoints each result.
struct Analyzer<M: CompletionModel> {
    agent: Agent<M>,
    synology: SynologyClient,
    session: Session,
    checklist: String,
    videos: Vec<Video>,
    frame_sets: Vec<FrameSet>,
    frame_sets_per_batch: NonZeroUsize,
    progress_path: PathBuf,
    checkpoint: AnalysisCheckpoint,
}

impl<M: CompletionModel> Analyzer<M> {
    /// Rebuilds the canonical plan and resumes from a validated checkpoint, or starts at batch zero.
    async fn resume(
        agent: Agent<M>,
        synology: SynologyClient,
        session: Session,
        checklist: String,
        frame_sets_per_batch: NonZeroUsize,
        progress_path: PathBuf,
    ) -> Result<Self> {
        let mut schedules = Vec::new();
        for camera in &session.cameras {
            let schedule = SamplingSchedule::from_session(&session, camera.id)?;
            if !schedule.periods.is_empty() {
                schedules.push(schedule);
            }
        }
        if schedules.is_empty() {
            return Err(Error::EmptyPlan);
        }

        let end_offset_ms = i64::try_from(session.end_offset.as_nanos().div_ceil(1_000_000))
            .map_err(|_| Error::SessionEndUtcOverflow)?;
        let end_utc_ms = session
            .start_utc_ms
            .checked_add(end_offset_ms)
            .ok_or(Error::SessionEndUtcOverflow)?;
        let camera_ids = schedules
            .iter()
            .map(|schedule| schedule.camera_id)
            .collect::<Vec<_>>();
        let videos = synology
            .list_videos(&camera_ids, session.start_utc_ms, end_utc_ms)
            .await?;
        let mut recording_ids = HashSet::with_capacity(videos.len());
        for video in &videos {
            if !recording_ids.insert(video.recording_id) {
                return Err(Error::DuplicateRecordingId {
                    recording_id: video.recording_id,
                });
            }
        }
        let sequences = schedules
            .iter()
            .map(|schedule| SampleSequence::from_videos(session.start_utc_ms, schedule, &videos))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let frame_sets = FrameSet::from_sequences(sequences)?;
        if frame_sets.is_empty() {
            return Err(Error::EmptyPlan);
        }
        let total_batches = frame_sets.chunks(frame_sets_per_batch.get()).count();
        let checkpoint = AnalysisCheckpoint::load(&progress_path, session.id, total_batches)?;

        Ok(Self {
            agent,
            synology,
            session,
            checklist,
            videos,
            frame_sets,
            frame_sets_per_batch,
            progress_path,
            checkpoint,
        })
    }

    /// Index the caller should materialize next after rebuilding the batch plan.
    fn next_batch_index(&self) -> usize {
        self.checkpoint.next_batch_index()
    }

    /// Materializes and analyzes the first incomplete batch, then durably checkpoints it.
    async fn analyze_next(&mut self) -> Result<&AnalysisResponse> {
        let index = self.next_batch_index();
        if index >= self.checkpoint.total_batches {
            return Err(Error::AnalysisComplete {
                total: self.checkpoint.total_batches,
            });
        }

        let batch_size = self.frame_sets_per_batch.get();
        let start = index * batch_size;
        let end = (start + batch_size).min(self.frame_sets.len());
        let (prompt, _media) = self
            .materialize_prompt(&self.frame_sets[start..end])
            .await?;
        self.submit_prompt(prompt).await
    }

    async fn submit_prompt(&mut self, prompt: Message) -> Result<&AnalysisResponse> {
        let index = self.next_batch_index();
        if index >= self.checkpoint.total_batches {
            return Err(Error::AnalysisComplete {
                total: self.checkpoint.total_batches,
            });
        }

        let response = self.agent.analyze(prompt).await?;
        self.checkpoint
            .completed_batches
            .push(CompletedBatch { index, response });
        if let Err(error) = self.checkpoint.save(&self.progress_path) {
            self.checkpoint.completed_batches.pop();
            return Err(error);
        }

        Ok(&self
            .checkpoint
            .completed_batches
            .last()
            .expect("the completed response was just appended")
            .response)
    }

    async fn materialize_prompt(&self, batch: &[FrameSet]) -> Result<(Message, TempDir)> {
        // ponytail: each batch downloads its required video windows locally;
        // move extraction onto the NAS if transfer becomes a bottleneck.
        let directory = TempDir::new()?;
        let downloads =
            download_batch(&self.synology, batch, &self.videos, directory.path()).await?;
        let mut content = prompt_content(&self.checklist, self.checkpoint.previous_response())?;

        for frame_set in batch {
            let timestamp = format_timestamp(frame_set.session_offset);
            append_prompt_frame_set(&mut content, &timestamp);
            for frame in &frame_set.frames {
                let camera = self
                    .session
                    .cameras
                    .iter()
                    .find(|camera| camera.id == frame.camera_id)
                    .ok_or(Error::MissingCamera {
                        camera_id: frame.camera_id,
                    })?;
                let download = downloads
                    .get(&frame.recording_id)
                    .ok_or(Error::MissingVideo {
                        recording_id: frame.recording_id,
                    })?;
                let offset = local_offset(frame, download.start)?;
                let input = download.path.clone();
                let jpeg =
                    tokio::task::spawn_blocking(move || extract_jpeg(&input, offset)).await??;
                append_prompt_frame(&mut content, camera.id, &camera.name, &timestamp, &jpeg);
                drop(jpeg);
            }
        }

        Ok((Message::User { content }, directory))
    }
}

struct DownloadWindow<'a> {
    video: &'a Video,
    start: Duration,
    end: Duration,
}

struct DownloadedVideo {
    path: PathBuf,
    start: Duration,
}

fn batch_windows<'a>(
    frame_sets: &[FrameSet],
    videos: &'a [Video],
) -> Result<Vec<DownloadWindow<'a>>> {
    let mut requested = BTreeMap::new();
    for frame in frame_sets.iter().flat_map(|frame_set| &frame_set.frames) {
        requested
            .entry(frame.recording_id)
            .and_modify(|(start, end): &mut (Duration, Duration)| {
                *start = (*start).min(frame.recording_offset);
                *end = (*end).max(frame.recording_offset);
            })
            .or_insert((frame.recording_offset, frame.recording_offset));
    }

    requested
        .into_iter()
        .map(|(recording_id, (start, last_frame))| {
            let video = videos
                .iter()
                .find(|video| video.recording_id == recording_id)
                .ok_or(Error::MissingVideo { recording_id })?;
            let duration_ms = video
                .end_utc_ms
                .checked_sub(video.start_utc_ms)
                .and_then(|duration| u64::try_from(duration).ok())
                .filter(|duration| *duration > 0)
                .ok_or(Error::InvalidVideoBounds { recording_id })?;
            let end = last_frame
                .saturating_add(Duration::from_secs(1))
                .min(Duration::from_millis(duration_ms));
            if start >= end {
                return Err(Error::InvalidBatchWindow {
                    recording_id,
                    start,
                    end,
                });
            }
            Ok(DownloadWindow { video, start, end })
        })
        .collect()
}

async fn download_batch(
    synology: &SynologyClient,
    frame_sets: &[FrameSet],
    videos: &[Video],
    directory: &Path,
) -> Result<BTreeMap<u64, DownloadedVideo>> {
    let mut downloads = BTreeMap::new();
    for window in batch_windows(frame_sets, videos)? {
        let recording_id = window.video.recording_id;
        let path = directory.join(format!("recording-{recording_id}.mp4"));
        synology
            .download(window.video, window.start..window.end, &path)
            .await?;
        downloads.insert(
            recording_id,
            DownloadedVideo {
                path,
                start: window.start,
            },
        );
    }
    Ok(downloads)
}

fn local_offset(frame: &Frame, download_start: Duration) -> Result<Duration> {
    frame
        .recording_offset
        .checked_sub(download_start)
        .ok_or(Error::InvalidLocalOffset {
            recording_id: frame.recording_id,
            offset: frame.recording_offset,
            download_start,
        })
}

fn format_timestamp(offset: Duration) -> String {
    let total_millis = offset.as_millis();
    let total_seconds = total_millis / 1_000;
    let hours = total_seconds / 3_600;
    let minutes = total_seconds / 60 % 60;
    let seconds = total_seconds % 60;
    let millis = total_millis % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

fn prompt_content(
    checklist: &str,
    previous: Option<&AnalysisResponse>,
) -> Result<OneOrMany<UserContent>> {
    let previous = previous
        .map(serde_json::to_string)
        .transpose()?
        .map(|response| format!("Previous complete analysis response:\n{response}"))
        .unwrap_or_else(|| "This is the first batch; there is no previous response.".into());
    let mut content = OneOrMany::one(UserContent::text(format!(
        "Correct sequence checklist:\n{checklist}"
    )));
    content.push(UserContent::text(previous));
    Ok(content)
}

fn append_prompt_frame_set(content: &mut OneOrMany<UserContent>, timestamp: &str) {
    content.push(UserContent::text(format!(
        "Frame set timestamp: {timestamp}"
    )));
}

fn append_prompt_frame(
    content: &mut OneOrMany<UserContent>,
    camera_id: u32,
    camera_name: &str,
    timestamp: &str,
    jpeg: &[u8],
) {
    content.push(UserContent::text(format!(
        "Frame source: camera {camera_id} ({camera_name}) at {timestamp}"
    )));
    content.push(UserContent::image_base64(
        STANDARD.encode(jpeg),
        Some(ImageMediaType::JPEG),
        None,
    ));
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        num::NonZeroUsize,
        path::Path,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use axum::{
        Json, Router,
        body::Body,
        extract::{Query, State},
        http::{StatusCode, header::CONTENT_TYPE},
        response::{IntoResponse, Response},
        routing::get,
    };
    use rig_core::{
        completion::{CompletionModel, Message},
        message::{DocumentSourceKind, UserContent},
        test_utils::{MockCompletionModel, MockTurn},
    };
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use uuid::Uuid;

    use crate::{
        analysis::{
            agent::{Agent, AnalysisResponse, ChecklistProgress, Observation, OpenAiAgent},
            video::{Error as VideoError, Frame, FrameSet, Video},
        },
        recording::SynologyClient,
        session::{Session, SessionCamera},
    };

    use super::{
        Analyzer, append_prompt_frame, append_prompt_frame_set, batch_windows, download_batch,
        format_timestamp, local_offset, prompt_content,
    };

    const SESSION_START_UTC_MS: i64 = 1_786_204_800_000;

    #[derive(Clone)]
    struct ServerState {
        response: Value,
        media: Option<Vec<u8>>,
        queries: Arc<Mutex<Vec<HashMap<String, String>>>>,
    }

    async fn recording_api(
        State(state): State<ServerState>,
        Query(query): Query<HashMap<String, String>>,
    ) -> Response {
        let is_download = query
            .get("method")
            .is_some_and(|method| method == "Download");
        state.queries.lock().unwrap().push(query);
        if is_download {
            match state.media {
                Some(media) => Response::builder()
                    .header(CONTENT_TYPE, "video/mp4")
                    .body(Body::from(media))
                    .unwrap(),
                None => StatusCode::SERVICE_UNAVAILABLE.into_response(),
            }
        } else {
            Json(state.response).into_response()
        }
    }

    async fn spawn_server(
        response: Value,
    ) -> (reqwest::Url, Arc<Mutex<Vec<HashMap<String, String>>>>) {
        spawn_server_with_media(response, None).await
    }

    async fn spawn_server_with_media(
        response: Value,
        media: Option<Vec<u8>>,
    ) -> (reqwest::Url, Arc<Mutex<Vec<HashMap<String, String>>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let queries = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/webapi/entry.cgi", get(recording_api))
            .with_state(ServerState {
                response,
                media,
                queries: Arc::clone(&queries),
            });
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        (
            reqwest::Url::parse(&format!("http://{address}")).unwrap(),
            queries,
        )
    }

    fn session(cameras: Vec<SessionCamera>) -> Session {
        Session {
            id: Uuid::from_u128(1),
            start_utc_ms: SESSION_START_UTC_MS,
            end_offset: Duration::from_secs(5),
            cameras,
            actions: Vec::new(),
        }
    }

    fn camera(id: u32, enabled: bool, sample_every_secs: u64) -> SessionCamera {
        SessionCamera {
            id,
            name: format!("Camera {id}"),
            enabled,
            sample_every: Duration::from_secs(sample_every_secs),
        }
    }

    fn video(recording_id: u64, camera_id: u32, duration_secs: i64) -> Video {
        Video {
            recording_id,
            camera_id,
            start_utc_ms: SESSION_START_UTC_MS,
            end_utc_ms: SESSION_START_UTC_MS + duration_secs * 1_000,
        }
    }

    fn frame(
        camera_id: u32,
        recording_id: u64,
        session_offset_ms: u64,
        recording_offset_ms: u64,
    ) -> Frame {
        Frame {
            camera_id,
            recording_id,
            sample_index: 0,
            session_offset: Duration::from_millis(session_offset_ms),
            recording_offset: Duration::from_millis(recording_offset_ms),
        }
    }

    fn recordings_response() -> Value {
        json!({
            "success": true,
            "data": {
                "total": 1,
                "events": [{
                    "id": 10,
                    "cameraId": 1,
                    "startTime": SESSION_START_UTC_MS / 1_000 - 1,
                    "stopTime": SESSION_START_UTC_MS / 1_000 + 6
                }]
            }
        })
    }

    fn two_camera_recordings_response(stop_offset_secs: i64) -> Value {
        json!({
            "success": true,
            "data": {
                "total": 2,
                "events": [
                    {
                        "id": 10,
                        "cameraId": 1,
                        "startTime": SESSION_START_UTC_MS / 1_000 - 1,
                        "stopTime": SESSION_START_UTC_MS / 1_000 + stop_offset_secs
                    },
                    {
                        "id": 20,
                        "cameraId": 2,
                        "startTime": SESSION_START_UTC_MS / 1_000 - 1,
                        "stopTime": SESSION_START_UTC_MS / 1_000 + stop_offset_secs
                    }
                ]
            }
        })
    }

    fn fixture_media() -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../camera/fixtures/default.mp4"),
        )
        .expect("video fixture should be readable")
    }

    async fn resume_analyzer<M: CompletionModel>(
        model: M,
        checkpoint: std::path::PathBuf,
        frame_sets_per_batch: usize,
        media: Option<Vec<u8>>,
    ) -> (Analyzer<M>, Arc<Mutex<Vec<HashMap<String, String>>>>) {
        let (base_url, queries) = spawn_server_with_media(recordings_response(), media).await;
        let analyzer = Analyzer::resume(
            Agent::new(model),
            SynologyClient::new(base_url),
            session(vec![camera(1, true, 2)]),
            "Start the exercise".into(),
            NonZeroUsize::new(frame_sets_per_batch).unwrap(),
            checkpoint,
        )
        .await
        .expect("analysis plan should resume");
        (analyzer, queries)
    }

    fn response(summary: &str) -> AnalysisResponse {
        AnalysisResponse {
            observations: vec![Observation {
                timestamp: "00:00:01".into(),
                description: "The student starts the exercise.".into(),
            }],
            sequence_summary: summary.into(),
            checklist_progress: vec![ChecklistProgress {
                item: "Start the exercise".into(),
                status: "respected".into(),
                note: String::new(),
            }],
        }
    }

    #[test]
    fn prompt_preserves_previous_response_and_frame_order() {
        let previous = response("The first step has started.");
        let mut content = prompt_content("Open the valve", Some(&previous))
            .expect("prompt header should be built");

        append_prompt_frame_set(&mut content, "00:00:01.000");
        append_prompt_frame(&mut content, 1, "Front", "00:00:01.000", &[1, 2]);
        append_prompt_frame(&mut content, 2, "Side", "00:00:01.000", &[3]);
        append_prompt_frame_set(&mut content, "00:00:02.000");
        append_prompt_frame(&mut content, 1, "Front", "00:00:02.000", &[4]);

        let content = content.iter().collect::<Vec<_>>();
        assert!(matches!(
            content[0],
            UserContent::Text(text) if text.text.contains("Open the valve")
        ));
        assert!(matches!(
            content[1],
            UserContent::Text(text) if text.text.contains("The first step has started.")
        ));
        assert!(matches!(
            content[2],
            UserContent::Text(text) if text.text.contains("00:00:01.000")
        ));
        assert!(matches!(
            content[3],
            UserContent::Text(text)
                if text.text.contains("camera 1")
                    && text.text.contains("Front")
                    && text.text.contains("00:00:01.000")
        ));
        assert!(matches!(
            content[4],
            UserContent::Image(image)
                if image.data == DocumentSourceKind::Base64("AQI=".into())
        ));
        assert!(matches!(
            content[5],
            UserContent::Text(text)
                if text.text.contains("camera 2") && text.text.contains("Side")
        ));
        assert!(matches!(
            content[6],
            UserContent::Image(image)
                if image.data == DocumentSourceKind::Base64("Aw==".into())
        ));
        assert!(matches!(
            content[7],
            UserContent::Text(text) if text.text.contains("00:00:02.000")
        ));
        assert!(matches!(
            content[8],
            UserContent::Text(text) if text.text.contains("camera 1")
        ));
        assert!(matches!(
            content[9],
            UserContent::Image(image)
                if image.data == DocumentSourceKind::Base64("BA==".into())
        ));
    }

    #[test]
    fn session_timestamps_include_zero_padded_milliseconds() {
        assert_eq!(
            format_timestamp(Duration::from_millis(3_723_004)),
            "01:02:03.004"
        );
    }

    #[test]
    fn batch_windows_merge_recordings_and_clamp_to_recording_duration() {
        let videos = vec![video(10, 1, 10), video(20, 2, 4)];
        let frame_sets = vec![
            FrameSet {
                session_offset: Duration::from_secs(1),
                frames: vec![frame(1, 10, 1_000, 2_000), frame(2, 20, 1_000, 3_500)],
            },
            FrameSet {
                session_offset: Duration::from_secs(2),
                frames: vec![frame(1, 10, 2_000, 5_000)],
            },
        ];

        let windows = batch_windows(&frame_sets, &videos).expect("batch windows should be built");

        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].video.recording_id, 10);
        assert_eq!(windows[0].start, Duration::from_secs(2));
        assert_eq!(windows[0].end, Duration::from_secs(6));
        assert_eq!(windows[1].video.recording_id, 20);
        assert_eq!(windows[1].start, Duration::from_millis(3_500));
        assert_eq!(windows[1].end, Duration::from_secs(4));
        assert_eq!(
            local_offset(&frame_sets[1].frames[0], windows[0].start)
                .expect("frame should be inside its download"),
            Duration::from_secs(3)
        );
    }

    #[tokio::test]
    async fn batch_downloads_each_recording_once_with_its_merged_range() {
        let (base_url, queries) = spawn_server_with_media(json!({}), Some(b"media".to_vec())).await;
        let videos = vec![video(10, 1, 10), video(20, 2, 4)];
        let frame_sets = vec![
            FrameSet {
                session_offset: Duration::from_secs(1),
                frames: vec![frame(1, 10, 1_000, 2_000), frame(2, 20, 1_000, 3_500)],
            },
            FrameSet {
                session_offset: Duration::from_secs(2),
                frames: vec![frame(1, 10, 2_000, 5_000)],
            },
        ];
        let directory = tempfile::tempdir().expect("temporary directory should be created");

        let downloads = download_batch(
            &SynologyClient::new(base_url),
            &frame_sets,
            &videos,
            directory.path(),
        )
        .await
        .expect("recording windows should download");

        assert_eq!(downloads.len(), 2);
        assert_eq!(downloads[&10].start, Duration::from_secs(2));
        assert_eq!(downloads[&20].start, Duration::from_millis(3_500));
        let queries = queries.lock().unwrap();
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0]["id"], "10");
        assert_eq!(queries[0]["offsetTimeMs"], "2000");
        assert_eq!(queries[0]["playTimeMs"], "4000");
        assert_eq!(queries[1]["id"], "20");
        assert_eq!(queries[1]["offsetTimeMs"], "3500");
        assert_eq!(queries[1]["playTimeMs"], "500");
    }

    #[tokio::test]
    async fn download_failure_happens_before_model_invocation() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let model = MockCompletionModel::text(
            serde_json::to_string(&response("unused")).expect("response should serialize"),
        );
        let recorded_model = model.clone();
        let (mut analyzer, queries) = resume_analyzer(model, checkpoint.clone(), 2, None).await;

        let result = analyzer.analyze_next().await;

        assert!(result.is_err());
        assert!(recorded_model.requests().is_empty());
        assert_eq!(analyzer.next_batch_index(), 0);
        assert!(!checkpoint.exists());
        assert_eq!(queries.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn model_failure_does_not_modify_progress() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let model = MockCompletionModel::new([MockTurn::error("provider unavailable")]);
        let (mut analyzer, _) = resume_analyzer(model, checkpoint.clone(), 2, None).await;

        let result = analyzer
            .submit_prompt(Message::user("prebuilt prompt"))
            .await;

        assert!(result.is_err());
        assert_eq!(analyzer.next_batch_index(), 0);
        assert!(!checkpoint.exists());
    }

    #[tokio::test]
    async fn failed_checkpoint_save_rolls_back_the_completed_batch() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("missing").join("analysis.json");
        let expected = response("The first batch is complete.");
        let model = MockCompletionModel::text(
            serde_json::to_string(&expected).expect("response should serialize"),
        );
        let (mut analyzer, _) = resume_analyzer(model, checkpoint.clone(), 2, None).await;

        let result = analyzer
            .submit_prompt(Message::user("prebuilt prompt"))
            .await;

        assert!(result.is_err());
        assert_eq!(analyzer.next_batch_index(), 0);
        assert!(!checkpoint.exists());
    }

    #[tokio::test]
    async fn completed_analysis_rejects_another_batch() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let expected = response("The analysis is complete.");
        let model = MockCompletionModel::text(
            serde_json::to_string(&expected).expect("response should serialize"),
        );
        let (mut analyzer, _) = resume_analyzer(model, checkpoint.clone(), 10, None).await;

        let actual = analyzer
            .submit_prompt(Message::user("prebuilt prompt"))
            .await
            .expect("only batch should complete");
        assert_eq!(actual, &expected);
        assert_eq!(analyzer.next_batch_index(), 1);
        assert!(checkpoint.exists());

        let result = analyzer.analyze_next().await;
        assert!(matches!(
            result,
            Err(super::Error::AnalysisComplete { total: 1 })
        ));
    }

    #[tokio::test]
    async fn resume_starts_at_the_first_incomplete_batch_with_previous_context() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let first = response("The first batch is complete.");
        let first_model = MockCompletionModel::text(
            serde_json::to_string(&first).expect("response should serialize"),
        );
        let (mut first_analyzer, _) =
            resume_analyzer(first_model, checkpoint.clone(), 2, None).await;
        first_analyzer
            .submit_prompt(Message::user("prebuilt prompt"))
            .await
            .expect("first batch should complete");
        drop(first_analyzer);

        let second_model = MockCompletionModel::text(
            serde_json::to_string(&response("Both batches are complete."))
                .expect("response should serialize"),
        );
        let (resumed, _) = resume_analyzer(second_model, checkpoint, 2, None).await;

        assert_eq!(resumed.next_batch_index(), 1);
        let content = prompt_content(&resumed.checklist, resumed.checkpoint.previous_response())
            .expect("resumed prompt should be built");
        assert!(content.iter().any(|content| matches!(
            content,
            UserContent::Text(text) if text.text.contains("The first batch is complete.")
        )));
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg on PATH"]
    async fn full_http_ffmpeg_and_model_pipeline_uses_the_existing_fixture() {
        let (base_url, queries) =
            spawn_server_with_media(two_camera_recordings_response(3), Some(fixture_media())).await;
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let expected = response("The batch is complete.");
        let model = MockCompletionModel::text(
            serde_json::to_string(&expected).expect("response should serialize"),
        );
        let recorded_model = model.clone();
        let mut exercise = session(vec![camera(1, true, 1), camera(2, true, 1)]);
        exercise.end_offset = Duration::from_secs(2);
        let mut analyzer = Analyzer::resume(
            Agent::new(model),
            SynologyClient::new(base_url),
            exercise,
            "Start the exercise".into(),
            NonZeroUsize::new(2).unwrap(),
            checkpoint.clone(),
        )
        .await
        .expect("analysis plan should resume");

        let actual = analyzer
            .analyze_next()
            .await
            .expect("fixture batch should be analyzed");

        assert_eq!(actual, &expected);
        assert!(checkpoint.exists());
        assert_eq!(queries.lock().unwrap().len(), 3);
        let requests = recorded_model.requests();
        let Message::User { content } = requests[0]
            .chat_history
            .iter()
            .last()
            .expect("request should contain a user message")
        else {
            panic!("last request message should be from the user");
        };
        let content = content.iter().collect::<Vec<_>>();
        assert!(matches!(
            content[2],
            UserContent::Text(text) if text.text.contains("00:00:00.000")
        ));
        assert!(matches!(
            content[3],
            UserContent::Text(text) if text.text.contains("camera 1")
        ));
        assert!(matches!(content[4], UserContent::Image(_)));
        assert!(matches!(
            content[5],
            UserContent::Text(text) if text.text.contains("camera 2")
        ));
        assert!(matches!(content[6], UserContent::Image(_)));
        assert!(matches!(
            content[7],
            UserContent::Text(text) if text.text.contains("00:00:01.000")
        ));
    }

    #[tokio::test]
    #[ignore = "costs money; requires LEO_RUN_PAID_OPENAI_TEST=1 and OpenAI environment"]
    async fn paid_openai_analyzes_table_setting_fixture() {
        if !matches!(
            std::env::var("LEO_RUN_PAID_OPENAI_TEST").as_deref(),
            Ok("1")
        ) {
            panic!(
                "paid OpenAI test is disabled; run exactly:\n\
                 LEO_RUN_PAID_OPENAI_TEST=1 cargo test -p app \
                 analysis::analyzer::analyzer::tests::paid_openai_analyzes_table_setting_fixture \
                 -- --ignored --exact --nocapture"
            );
        }

        let agent = OpenAiAgent::from_env().expect("OpenAI environment should configure the model");
        let media =
            std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../videos/table_1.mov"))
                .expect("table-setting fixture should be readable");
        let (base_url, queries) = spawn_server_with_media(
            json!({
                "success": true,
                "data": {
                    "total": 1,
                    "events": [{
                        "id": 10,
                        "cameraId": 1,
                        "startTime": SESSION_START_UTC_MS / 1_000,
                        "stopTime": SESSION_START_UTC_MS / 1_000 + 18
                    }]
                }
            }),
            Some(media),
        )
        .await;
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let mut exercise = session(vec![camera(1, true, 4)]);
        exercise.end_offset = Duration::from_secs(17);
        let checklist = "1. Place one plate in front of each of two chairs.\n\
                         2. Leave and return carrying two clear glasses.\n\
                         3. Place one glass to the right of each plate.";
        let mut analyzer = Analyzer::resume(
            agent,
            SynologyClient::new(base_url),
            exercise,
            checklist.into(),
            NonZeroUsize::new(5).unwrap(),
            checkpoint.clone(),
        )
        .await
        .expect("table-setting analysis should plan one batch");
        assert_eq!(
            analyzer
                .frame_sets
                .iter()
                .map(|frame_set| frame_set.session_offset)
                .collect::<Vec<_>>(),
            [0, 4, 8, 12, 16].map(Duration::from_secs)
        );
        assert_eq!(analyzer.checkpoint.total_batches, 1);

        let response = analyzer
            .analyze_next()
            .await
            .expect("OpenAI should analyze the table-setting batch");

        println!(
            "{}",
            serde_json::to_string_pretty(response).expect("analysis response should serialize")
        );
        assert!(!response.observations.is_empty());
        assert!(!response.sequence_summary.trim().is_empty());
        assert_eq!(response.checklist_progress.len(), 3);
        assert_eq!(analyzer.next_batch_index(), 1);
        assert!(checkpoint.exists());
        let queries = queries.lock().unwrap();
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0]["method"], "List");
        assert_eq!(queries[1]["method"], "Download");
        assert_eq!(queries[1]["offsetTimeMs"], "0");
        assert_eq!(queries[1]["playTimeMs"], "17000");
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg on PATH"]
    async fn full_http_ffmpeg_pipeline_resumes_with_previous_response_and_next_frames() {
        let (base_url, queries) =
            spawn_server_with_media(two_camera_recordings_response(5), Some(fixture_media())).await;
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let first = response("The first batch is complete.");
        let second = response("Both batches are complete.");
        let model = MockCompletionModel::new([
            MockTurn::text(serde_json::to_string(&first).expect("response should serialize")),
            MockTurn::text(serde_json::to_string(&second).expect("response should serialize")),
        ]);
        let recorded_model = model.clone();
        let mut first_session = session(vec![camera(1, true, 1), camera(2, true, 1)]);
        first_session.end_offset = Duration::from_secs(4);
        let mut first_analyzer = Analyzer::resume(
            Agent::new(model.clone()),
            SynologyClient::new(base_url.clone()),
            first_session,
            "Start the exercise".into(),
            NonZeroUsize::new(2).unwrap(),
            checkpoint.clone(),
        )
        .await
        .expect("first analyzer should plan two batches");

        first_analyzer
            .analyze_next()
            .await
            .expect("first batch should be analyzed");
        assert_eq!(first_analyzer.next_batch_index(), 1);
        assert!(checkpoint.exists());
        drop(first_analyzer);

        let mut resumed_session = session(vec![camera(1, true, 1), camera(2, true, 1)]);
        resumed_session.end_offset = Duration::from_secs(4);
        let mut resumed = Analyzer::resume(
            Agent::new(model),
            SynologyClient::new(base_url),
            resumed_session,
            "Start the exercise".into(),
            NonZeroUsize::new(2).unwrap(),
            checkpoint,
        )
        .await
        .expect("second analyzer should resume the saved plan");

        assert_eq!(resumed.next_batch_index(), 1);
        resumed
            .analyze_next()
            .await
            .expect("second batch should be analyzed");
        assert_eq!(resumed.next_batch_index(), 2);

        let requests = recorded_model.requests();
        assert_eq!(requests.len(), 2);
        let Message::User { content } = requests[1]
            .chat_history
            .iter()
            .last()
            .expect("request should contain a user message")
        else {
            panic!("last request message should be from the user");
        };
        let content = content.iter().collect::<Vec<_>>();
        assert!(matches!(
            content[1],
            UserContent::Text(text)
                if text.text == format!(
                    "Previous complete analysis response:\n{}",
                    serde_json::to_string(&first).unwrap()
                )
        ));
        assert!(matches!(
            content[2],
            UserContent::Text(text) if text.text.contains("00:00:02.000")
        ));
        assert!(matches!(
            content[3],
            UserContent::Text(text) if text.text.contains("camera 1")
        ));
        assert!(matches!(content[4], UserContent::Image(_)));
        assert!(matches!(
            content[5],
            UserContent::Text(text) if text.text.contains("camera 2")
        ));
        assert!(matches!(content[6], UserContent::Image(_)));
        assert!(matches!(
            content[7],
            UserContent::Text(text) if text.text.contains("00:00:03.000")
        ));
        assert_eq!(queries.lock().unwrap().len(), 6);
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg on PATH"]
    async fn concrete_ffmpeg_extraction_failure_precedes_model_and_checkpoint() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let model = MockCompletionModel::text(
            serde_json::to_string(&response("unused")).expect("response should serialize"),
        );
        let recorded_model = model.clone();
        let (mut analyzer, queries) = resume_analyzer(
            model,
            checkpoint.clone(),
            2,
            Some(b"not valid video media".to_vec()),
        )
        .await;

        let result = analyzer.analyze_next().await;

        assert!(matches!(
            result,
            Err(super::Error::Video(VideoError::FfmpegExit { .. }))
        ));
        assert!(recorded_model.requests().is_empty());
        assert!(!checkpoint.exists());
        assert_eq!(analyzer.next_batch_index(), 0);
        assert_eq!(queries.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn resume_rebuilds_the_canonical_plan_and_fixed_batches() {
        let (base_url, queries) = spawn_server(json!({
            "success": true,
            "data": {
                "total": 1,
                "events": [{
                    "id": 10,
                    "cameraId": 1,
                    "startTime": SESSION_START_UTC_MS / 1_000 - 1,
                    "stopTime": SESSION_START_UTC_MS / 1_000 + 6
                }]
            }
        }))
        .await;
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let analyzer = Analyzer::resume(
            Agent::new(MockCompletionModel::text("unused")),
            SynologyClient::new(base_url),
            session(vec![camera(1, true, 2), camera(2, false, 1)]),
            "Start the exercise".into(),
            NonZeroUsize::new(2).unwrap(),
            checkpoint.clone(),
        )
        .await
        .expect("new analysis should start");

        assert_eq!(analyzer.videos.len(), 1);
        assert_eq!(
            analyzer
                .frame_sets
                .iter()
                .map(|frame_set| frame_set.session_offset)
                .collect::<Vec<_>>(),
            [0, 2, 4].map(Duration::from_secs)
        );
        assert_eq!(analyzer.frame_sets_per_batch.get(), 2);
        assert_eq!(analyzer.frame_sets.chunks(2).count(), 2);
        assert_eq!(analyzer.checkpoint.total_batches, 2);
        assert_eq!(analyzer.next_batch_index(), 0);
        assert!(!checkpoint.exists());

        let queries = queries.lock().unwrap();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0]["cameraIds"], "1");
        assert_eq!(
            queries[0]["fromTime"],
            (SESSION_START_UTC_MS / 1_000).to_string()
        );
        assert_eq!(
            queries[0]["toTime"],
            (SESSION_START_UTC_MS / 1_000 + 5).to_string()
        );
    }

    #[tokio::test]
    async fn resume_rejects_duplicate_recording_ids_before_sequence_planning() {
        let (base_url, queries) = spawn_server(json!({
            "success": true,
            "data": {
                "total": 2,
                "events": [
                    {
                        "id": 10,
                        "cameraId": 1,
                        "startTime": SESSION_START_UTC_MS / 1_000 - 1,
                        "stopTime": SESSION_START_UTC_MS / 1_000 + 6
                    },
                    {
                        "id": 10,
                        "cameraId": 2,
                        "startTime": SESSION_START_UTC_MS / 1_000 - 2,
                        "stopTime": SESSION_START_UTC_MS / 1_000 + 7
                    }
                ]
            }
        }))
        .await;
        let directory = tempfile::tempdir().expect("temporary directory should be created");

        let result = Analyzer::resume(
            Agent::new(MockCompletionModel::text("unused")),
            SynologyClient::new(base_url),
            session(vec![camera(1, true, 1), camera(2, true, 1)]),
            "Start the exercise".into(),
            NonZeroUsize::new(2).unwrap(),
            directory.path().join("analysis.json"),
        )
        .await;

        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("duplicate recording IDs must be rejected"),
        };
        assert!(matches!(
            error,
            super::Error::DuplicateRecordingId { recording_id: 10 }
        ));
        assert_eq!(queries.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn resume_rejects_an_empty_generated_plan() {
        let (base_url, queries) = spawn_server(json!({
            "success": true,
            "data": {"total": 0, "events": []}
        }))
        .await;
        let directory = tempfile::tempdir().expect("temporary directory should be created");

        let result = Analyzer::resume(
            Agent::new(MockCompletionModel::text("unused")),
            SynologyClient::new(base_url),
            session(vec![camera(1, false, 1)]),
            "Start the exercise".into(),
            NonZeroUsize::new(2).unwrap(),
            directory.path().join("analysis.json"),
        )
        .await;

        assert!(matches!(result, Err(super::Error::EmptyPlan)));
        assert!(queries.lock().unwrap().is_empty());
    }
}

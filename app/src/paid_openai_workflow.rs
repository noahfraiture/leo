#![cfg(feature = "paid-openai-test")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use backend::{
    analysis::{AnalysisCheckpoint, AnalyzeSession, analyze_session},
    recording::{RecorderRuntime, RecorderSettings},
    session::mark_recording_complete,
};
use serde_json::json;
use uuid::Uuid;

use crate::{camera_config::CameraConfig, workflow::Workflow};

#[tokio::test]
#[ignore = "costs money; requires explicit approval and LEO_RUN_PAID_OPENAI_TEST=1"]
async fn paid_openai_analyzes_one_local_application_session() {
    assert_eq!(
        std::env::var("LEO_RUN_PAID_OPENAI_TEST").as_deref(),
        Ok("1"),
        "paid test requires explicit approval and LEO_RUN_PAID_OPENAI_TEST=1"
    );

    let root = paid_evaluation_root();
    let sessions_root = root.join("sessions");
    fs::create_dir_all(&sessions_root).expect("sessions root should be created");
    println!("PAID_EVAL_ROOT={}", root.display());
    let session_id = Uuid::new_v4();
    let start_utc_ms = 1_786_552_800_000_i64;
    let session_directory = create_local_session(
        &sessions_root,
        session_id,
        start_utc_ms,
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../camera/fixtures/salon-1.mp4")
            .as_path(),
    );

    let (runtime, handle, _events) = RecorderRuntime::spawn(RecorderSettings {
        io_timeout: Duration::from_secs(10),
        retry_delay: Duration::from_secs(1),
        stop_timeout: Duration::from_secs(5),
    })
    .expect("paid test requires ffmpeg and ffprobe");
    let mut workflow = Workflow::new(camera_configs(), sessions_root, handle, None)
        .expect("workflow should initialize");
    workflow
        .refresh_sessions()
        .expect("session should be discovered");
    workflow.selected_session_id = Some(session_id);
    let request = workflow
        .begin_analysis("Describe the visible exercise in order.".into())
        .expect("analysis should start");
    let mut callback_counts = Vec::new();

    let checkpoint = analyze_session(request, |snapshot| {
        callback_counts.push(snapshot.responses.len());
        workflow.apply_checkpoint(snapshot);
    })
    .await
    .expect("paid analysis should complete");

    assert_eq!(callback_counts, [0, 1]);
    assert_eq!(checkpoint.responses.len(), 1);
    println!(
        "PAID_EVAL_CASE=natural-fixture\n{}",
        serde_json::to_string_pretty(&checkpoint).expect("checkpoint should serialize")
    );
    assert_eq!(workflow.running_analysis_id, None);
    assert_eq!(
        AnalysisCheckpoint::read(&session_directory.join("analysis.json"), session_id)
            .expect("saved checkpoint should reload"),
        checkpoint
    );
    assert_no_temporary_media(&session_directory);
    runtime
        .shutdown()
        .expect("recorder runtime should shut down");
}

#[tokio::test]
#[ignore = "costs money; requires explicit approval and LEO_RUN_PAID_OPENAI_TEST=1"]
async fn paid_openai_evaluates_controlled_visual_payloads() {
    assert_eq!(
        std::env::var("LEO_RUN_PAID_OPENAI_TEST").as_deref(),
        Ok("1"),
        "paid test requires explicit approval and LEO_RUN_PAID_OPENAI_TEST=1"
    );
    assert!(
        std::env::var_os("OPENAI_BASE_URL").is_none(),
        "controlled evaluation must use OpenAI directly"
    );

    let root = paid_evaluation_root();
    fs::create_dir_all(&root).expect("paid evaluation root should be created");
    println!("PAID_EVAL_ROOT={}", root.display());
    let horizontal = root.join("horizontal.mp4");
    let vertical = root.join("vertical.mp4");
    create_synthetic_fixture(
        &horizontal,
        11,
        "drawbox=x=40:y=140:w=80:h=80:color=red:t=fill:enable='lt(t,4)',\
         drawbox=x=280:y=140:w=80:h=80:color=red:t=fill:enable='between(t,4,6.999)',\
         drawbox=x=520:y=140:w=80:h=80:color=red:t=fill:enable='between(t,7,7.999)',\
         drawbox=x=520:y=140:w=80:h=80:color=green:t=fill:enable='gte(t,8)'",
    );
    create_synthetic_fixture(
        &vertical,
        5,
        "drawbox=x=280:y=30:w=80:h=80:color=blue:t=fill:enable='lt(t,2)',\
         drawbox=x=280:y=140:w=80:h=80:color=blue:t=fill:enable='between(t,2,3.999)',\
         drawbox=x=280:y=250:w=80:h=80:color=blue:t=fill:enable='gte(t,4)'",
    );

    run_paid_evaluation(
        &root,
        "horizontal-sequence",
        &[("Horizontal view", horizontal.as_path())],
        11_000,
        1_000,
        "1. The red square begins on the left.\n\
         2. The red square moves to the center.\n\
         3. The red square moves to the right.\n\
         4. The square changes from red to green while staying on the right.",
        (4, 3),
    )
    .await;
    run_paid_evaluation(
        &root,
        "negative-control",
        &[("Horizontal view", horizontal.as_path())],
        11_000,
        2_000,
        "1. A blue square appears.\n2. The red square changes to green.",
        (2, 2),
    )
    .await;
    run_paid_evaluation(
        &root,
        "two-camera-pairing",
        &[
            ("Horizontal view", horizontal.as_path()),
            ("Vertical view", vertical.as_path()),
        ],
        5_000,
        1_000,
        "1. Camera 1's red square moves from the left to the center.\n\
         2. Camera 2's blue square moves from the top through the center to the bottom.",
        (2, 1),
    )
    .await;
}

async fn run_paid_evaluation(
    root: &Path,
    name: &str,
    fixtures: &[(&str, &Path)],
    duration_ms: u64,
    sample_every_ms: u64,
    checklist: &str,
    expected: (usize, usize),
) {
    let (expected_items, expected_batches) = expected;
    let session_id = Uuid::new_v4();
    let directory = create_evaluation_session(
        root,
        name,
        session_id,
        fixtures,
        duration_ms,
        sample_every_ms,
    );
    let checkpoint = analyze_session(
        AnalyzeSession {
            directory,
            checklist: checklist.into(),
        },
        |_| {},
    )
    .await
    .unwrap_or_else(|error| panic!("paid evaluation {name} failed: {error}"));

    assert_eq!(checkpoint.total_batches, expected_batches, "case {name}");
    assert_eq!(checkpoint.responses.len(), expected_batches, "case {name}");
    for response in &checkpoint.responses {
        assert!(!response.observations.is_empty(), "case {name}");
        assert_eq!(
            response.checklist_progress.len(),
            expected_items,
            "case {name} must update every checklist item"
        );
    }
    // Prose remains human-graded; these assertions cover transport and response completeness.
    println!(
        "PAID_EVAL_CASE={name}\n{}",
        serde_json::to_string_pretty(&checkpoint).expect("checkpoint should serialize")
    );
}

fn paid_evaluation_root() -> PathBuf {
    let utc_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow Unix epoch")
        .as_millis();
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target/paid-openai-eval")
        .join(utc_ms.to_string())
}

fn create_synthetic_fixture(path: &Path, duration_secs: u64, filter: &str) {
    let input = format!("color=c=white:s=640x360:r=15:d={duration_secs}");
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(input)
        .args([
            "-vf",
            filter,
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-g",
            "15",
        ])
        .arg(path)
        .status()
        .expect("FFmpeg should start for synthetic fixture");
    assert!(status.success(), "FFmpeg should create synthetic fixture");
}

fn create_evaluation_session(
    root: &Path,
    name: &str,
    session_id: Uuid,
    fixtures: &[(&str, &Path)],
    duration_ms: u64,
    sample_every_ms: u64,
) -> PathBuf {
    let directory = root.join(name);
    let recordings = directory.join("recordings");
    fs::create_dir_all(&recordings).expect("recordings directory should be created");
    let start_utc_ms = 1_786_552_800_000_i64;
    let cameras = fixtures
        .iter()
        .enumerate()
        .map(|(index, (camera_name, _))| {
            json!({
                "camera_id": index + 1,
                "name": camera_name,
                "enabled": true,
                "sample_every_ms": sample_every_ms
            })
        })
        .collect::<Vec<_>>();
    let events = [
        json!({
            "schema_version": 1,
            "sequence": 0,
            "session_id": session_id,
            "utc_ms": start_utc_ms,
            "session_offset_ms": 0,
            "action": {"type": "session_started", "cameras": cameras}
        }),
        json!({
            "schema_version": 1,
            "sequence": 1,
            "session_id": session_id,
            "utc_ms": start_utc_ms + i64::try_from(duration_ms).expect("duration should fit i64"),
            "session_offset_ms": duration_ms,
            "action": {"type": "session_ended"}
        }),
    ]
    .into_iter()
    .map(|event| serde_json::to_string(&event).expect("event should serialize"))
    .collect::<Vec<_>>()
    .join("\n")
        + "\n";
    fs::write(directory.join("events.jsonl"), events).expect("event log should be written");

    let duration = format!("{duration_ms}ms");
    for (index, (_, fixture)) in fixtures.iter().enumerate() {
        let camera = recordings.join(format!("camera-{}", index + 1));
        fs::create_dir(&camera).expect("camera directory should be created");
        let segment = camera.join(format!("{start_utc_ms}.mkv"));
        let status = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-t"])
            .arg(&duration)
            .arg("-i")
            .arg(fixture)
            .args(["-map", "0:v:0", "-an", "-c:v", "copy", "-f", "matroska"])
            .arg(segment)
            .status()
            .expect("FFmpeg should start for evaluation segment");
        assert!(status.success(), "FFmpeg should create evaluation segment");
    }
    mark_recording_complete(&directory).expect("session should be marked complete");
    directory
}

fn camera_configs() -> Vec<CameraConfig> {
    [1_u32, 2]
        .into_iter()
        .map(|id| CameraConfig {
            id,
            name: format!("Salon {id}"),
            rtsp_url: format!("rtsp://127.0.0.1:855{}/axis-media/media.amp", id + 3),
            enabled: id == 1,
            sample_every_ms: 1_000,
        })
        .collect()
}

fn create_local_session(
    sessions_root: &Path,
    session_id: Uuid,
    start_utc_ms: i64,
    fixture: &Path,
) -> PathBuf {
    let directory = sessions_root.join(start_utc_ms.to_string());
    let camera_1 = directory.join("recordings/camera-1");
    let camera_2 = directory.join("recordings/camera-2");
    fs::create_dir_all(&camera_1).expect("camera 1 directory should be created");
    fs::create_dir(&camera_2).expect("camera 2 directory should be created");
    let events = [
        json!({
            "schema_version": 1,
            "sequence": 0,
            "session_id": session_id,
            "utc_ms": start_utc_ms,
            "session_offset_ms": 0,
            "action": {
                "type": "session_started",
                "cameras": [
                    {"camera_id": 1, "name": "Salon 1", "enabled": true, "sample_every_ms": 1_000},
                    {"camera_id": 2, "name": "Salon 2", "enabled": false, "sample_every_ms": 1_000}
                ]
            }
        }),
        json!({
            "schema_version": 1,
            "sequence": 1,
            "session_id": session_id,
            "utc_ms": start_utc_ms + 5_000,
            "session_offset_ms": 5_000,
            "action": {"type": "session_ended"}
        }),
    ]
    .into_iter()
    .map(|event| serde_json::to_string(&event).expect("event should serialize"))
    .collect::<Vec<_>>()
    .join("\n")
        + "\n";
    fs::write(directory.join("events.jsonl"), events).expect("event log should be written");

    let segment = camera_1.join(format!("{start_utc_ms}.mkv"));
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            "0",
            "-t",
            "5",
            "-i",
        ])
        .arg(fixture)
        .args(["-map", "0:v:0", "-an", "-c:v", "copy", "-f", "matroska"])
        .arg(&segment)
        .status()
        .expect("ffmpeg should start");
    assert!(status.success(), "ffmpeg should create the local MKV");
    mark_recording_complete(&directory).expect("session should be marked complete");
    directory
}

fn assert_no_temporary_media(directory: &Path) {
    for entry in fs::read_dir(directory).expect("session directory should be readable") {
        let path = entry.expect("session entry should be readable").path();
        if path.is_dir() {
            assert_no_temporary_media(&path);
        } else {
            assert!(!matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("jpg" | "mp4")
            ));
        }
    }
}

use backend::analysis::AnalyzeSession;
use dioxus::prelude::{Signal, WritableExt};
use uuid::Uuid;

use crate::workflow::Workflow;

/// Runs one analysis independently of route lifetimes and projects durable snapshots.
pub fn spawn_analysis(mut workflow: Signal<Workflow>, request: AnalyzeSession, session_id: Uuid) {
    tracing::info!(%session_id, "analysis started");
    dioxus::dioxus_core::spawn_forever(async move {
        let mut checkpoint_workflow = workflow;
        let result = backend::analysis::analyze_session(request, move |checkpoint| {
            checkpoint_workflow.write().apply_checkpoint(checkpoint);
        })
        .await;

        match result {
            Ok(checkpoint) => tracing::info!(
                %session_id,
                completed_batches = checkpoint.responses.len(),
                total_batches = checkpoint.total_batches,
                "analysis completed"
            ),
            Err(error) => {
                tracing::error!(%session_id, error = %error, "analysis failed");
                workflow
                    .write()
                    .analysis_failed(session_id, error.to_string());
            }
        }
    });
}

#[cfg(all(test, unix))]
mod tests {
    use std::{cell::RefCell, fs, os::unix::fs::PermissionsExt, time::Duration};

    use backend::{
        analysis::{
            AnalysisCheckpoint, AnalysisResponse, AnalysisWarning, AnalyzeSession,
            ChecklistProgress, Observation,
        },
        recording::{RecorderRuntime, RecorderSettings, spawn_for_test},
        session::mark_recording_complete,
    };
    use dioxus::prelude::Signal;
    use serde_json::json;
    use uuid::Uuid;

    use super::spawn_analysis;
    use crate::{settings::CameraSettings, workflow::Workflow};

    struct Presentation {
        completed: usize,
        total: usize,
        warnings: Vec<AnalysisWarning>,
        observations: Vec<(String, String)>,
        summary: Option<String>,
        checklist: Vec<ChecklistProgress>,
    }

    fn test_workflow() -> (tempfile::TempDir, RecorderRuntime, Workflow) {
        let temporary = tempfile::tempdir().expect("temporary root should be created");
        let executable = temporary.path().join("successful-preflight");
        fs::write(&executable, "#!/bin/sh\nexit 0\n")
            .expect("fake preflight executable should be written");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
            .expect("fake preflight executable should be executable");
        let (runtime, recorder, _events) = spawn_for_test(
            RecorderSettings {
                io_timeout: Duration::from_secs(1),
                retry_delay: Duration::from_secs(1),
                stop_timeout: Duration::from_secs(1),
            },
            executable.clone(),
            executable,
        )
        .expect("test recorder runtime should start");
        let workflow = Workflow::new(
            [1_u32, 2]
                .into_iter()
                .map(|id| CameraSettings {
                    id,
                    name: format!("Salon {id}"),
                    rtsp_url: format!("rtsp://camera-{id}.example/live"),
                    initially_included_in_analysis: true,
                    sample_every_ms: 1_000,
                })
                .collect(),
            temporary.path().join("sessions"),
            recorder,
            Some(crate::test_openai_config()),
        )
        .expect("workflow should initialize");
        (temporary, runtime, workflow)
    }

    fn write_completed_session(workflow: &mut Workflow, session_id: Uuid) {
        let directory = workflow.session_root.join("completed");
        fs::create_dir_all(&directory).expect("session directory should be created");
        let events = [
            json!({
                "schema_version": 1,
                "sequence": 0,
                "session_id": session_id,
                "utc_ms": 1_786_552_800_000_i64,
                "session_offset_ms": 0,
                "action": {
                    "type": "session_started",
                    "cameras": [
                        {
                            "camera_id": 1,
                            "name": "Salon 1",
                            "enabled": true,
                            "sample_every_ms": 1_000
                        },
                        {
                            "camera_id": 2,
                            "name": "Salon 2",
                            "enabled": true,
                            "sample_every_ms": 1_000
                        }
                    ]
                }
            }),
            json!({
                "schema_version": 1,
                "sequence": 1,
                "session_id": session_id,
                "utc_ms": 1_786_552_802_000_i64,
                "session_offset_ms": 2_000,
                "action": { "type": "session_ended" }
            }),
        ]
        .into_iter()
        .map(|event| serde_json::to_string(&event).expect("event should serialize"))
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        fs::write(directory.join("events.jsonl"), events)
            .expect("session events should be written");
        mark_recording_complete(&directory).expect("session should be marked complete");
        workflow
            .refresh_sessions()
            .expect("completed session should be discovered");
        workflow.selected_session_id = Some(session_id);
    }

    fn response(timestamp: &str, description: &str, summary: &str) -> AnalysisResponse {
        AnalysisResponse {
            observations: vec![Observation {
                timestamp: timestamp.into(),
                description: description.into(),
            }],
            sequence_summary: summary.into(),
            checklist_progress: vec![ChecklistProgress {
                item: "Complete the exercise".into(),
                status: summary.into(),
                note: format!("Evidence at {timestamp}"),
            }],
        }
    }

    fn checkpoint(session_id: Uuid, responses: Vec<AnalysisResponse>) -> AnalysisCheckpoint {
        AnalysisCheckpoint {
            schema_version: 2,
            session_id,
            checklist: "Persisted checklist".into(),
            plan_fingerprint: "0123456789abcdef".into(),
            total_batches: 2,
            warnings: vec![AnalysisWarning::RecordingGap {
                camera_id: 2,
                start_offset_ms: 500,
                end_offset_ms: 1_000,
            }],
            responses,
        }
    }

    fn project(workflow: &Workflow, session_id: Uuid) -> Presentation {
        let checkpoint = workflow
            .sessions
            .iter()
            .find(|row| row.stored.session.id == session_id)
            .expect("session row should remain")
            .checkpoint
            .as_ref()
            .expect("checkpoint should be valid")
            .as_ref()
            .expect("checkpoint should be present");
        let latest = checkpoint.responses.last();
        Presentation {
            completed: checkpoint.responses.len(),
            total: checkpoint.total_batches,
            warnings: checkpoint.warnings.clone(),
            observations: checkpoint
                .responses
                .iter()
                .flat_map(|response| &response.observations)
                .map(|observation| {
                    (
                        observation.timestamp.clone(),
                        observation.description.clone(),
                    )
                })
                .collect(),
            summary: latest.map(|response| response.sequence_summary.clone()),
            checklist: latest
                .map(|response| response.checklist_progress.clone())
                .unwrap_or_default(),
        }
    }

    #[test]
    fn checkpoint_callback_outlives_recreated_presentation_and_retry() {
        let _: fn(Signal<Workflow>, AnalyzeSession, Uuid) = spawn_analysis;
        let (_temporary, runtime, mut workflow) = test_workflow();
        let session_id = Uuid::from_u128(41);
        write_completed_session(&mut workflow, session_id);
        workflow
            .begin_analysis("Persisted checklist".into())
            .expect("analysis should begin");
        let workflow = RefCell::new(workflow);
        let callback = |checkpoint| workflow.borrow_mut().apply_checkpoint(checkpoint);

        callback(checkpoint(session_id, Vec::new()));
        let initial = project(&workflow.borrow(), session_id);
        assert_eq!((initial.completed, initial.total), (0, 2));
        assert_eq!(initial.warnings.len(), 1);
        assert!(initial.observations.is_empty());
        assert_eq!(initial.summary, None);

        callback(checkpoint(
            session_id,
            vec![response("00:00:00.250", "First movement", "started")],
        ));
        let partial = project(&workflow.borrow(), session_id);
        assert_eq!((partial.completed, partial.total), (1, 2));
        assert_eq!(partial.warnings, initial.warnings);
        assert_eq!(
            partial.observations,
            [("00:00:00.250".into(), "First movement".into())]
        );
        assert_eq!(partial.summary.as_deref(), Some("started"));

        workflow
            .borrow_mut()
            .analysis_failed(session_id, "temporary failure".into());
        let retry = workflow
            .borrow_mut()
            .begin_analysis("Replacement text".into())
            .expect("saved analysis should retry");
        assert_eq!(retry.checklist, "Persisted checklist");

        callback(checkpoint(
            session_id,
            vec![
                response("00:00:00.250", "First movement", "started"),
                response("00:00:01.250", "Final movement", "respected"),
            ],
        ));
        let complete = project(&workflow.borrow(), session_id);
        assert_eq!((complete.completed, complete.total), (2, 2));
        assert_eq!(complete.warnings, initial.warnings);
        assert_eq!(
            complete.observations,
            [
                ("00:00:00.250".into(), "First movement".into()),
                ("00:00:01.250".into(), "Final movement".into()),
            ]
        );
        assert_eq!(complete.summary.as_deref(), Some("respected"));
        assert_eq!(complete.checklist[0].status, "respected");
        assert_eq!(workflow.borrow().running_analysis_id, None);

        runtime.shutdown().expect("runtime should shut down");
    }
}

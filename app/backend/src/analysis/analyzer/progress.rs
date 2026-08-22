use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::analysis::{agent::AnalysisResponse, error::Error, video::AnalysisWarning};

/// Current on-disk schema and behavior-changing analysis-pipeline revision.
pub const ANALYSIS_SCHEMA_VERSION: u8 = 3;

/// Non-secret provider configuration that identifies analysis output.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AnalysisIdentity {
    /// Exact provider model name used for analysis requests.
    pub model: String,
    /// Stable deployment-defined identifier for the provider endpoint.
    pub endpoint_id: String,
}

/// Durable analysis identity, warnings, and completed response prefix for one session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisCheckpoint {
    /// Version of the persisted checkpoint schema.
    pub schema_version: u8,
    /// Session UUID whose analysis is being checkpointed.
    pub session_id: Uuid,
    /// Exact non-secret provider identity used for every response.
    pub analysis_identity: AnalysisIdentity,
    /// Correct-sequence checklist used for every model request.
    pub checklist: String,
    /// Path-independent SHA-256 identity of the canonical frame plan.
    pub plan_fingerprint: String,
    /// Batch count from the freshly rebuilt canonical plan.
    pub total_batches: usize,
    /// Physical recording gaps found while rebuilding the plan.
    pub warnings: Vec<AnalysisWarning>,
    /// Contiguous completed prefix; vector position is the batch index.
    pub responses: Vec<AnalysisResponse>,
}

impl AnalysisCheckpoint {
    /// Reads and validates a direct regular checkpoint file for the expected session.
    pub fn read(path: &Path, expected_session_id: Uuid) -> Result<Self, Error> {
        if !std::fs::symlink_metadata(path)?.file_type().is_file() {
            return Err(Error::InvalidCheckpointFile);
        }

        #[derive(Deserialize)]
        struct VersionEnvelope {
            schema_version: u8,
        }

        let bytes = std::fs::read(path)?;
        let version = serde_json::from_slice::<VersionEnvelope>(&bytes)?.schema_version;
        if version != ANALYSIS_SCHEMA_VERSION {
            return Err(Error::CheckpointSchema {
                expected: ANALYSIS_SCHEMA_VERSION,
                actual: version,
            });
        }
        let checkpoint: Self = serde_json::from_slice(&bytes)?;
        checkpoint.validate(expected_session_id)?;
        Ok(checkpoint)
    }

    fn validate(&self, expected_session_id: Uuid) -> Result<(), Error> {
        if self.schema_version != ANALYSIS_SCHEMA_VERSION {
            return Err(Error::CheckpointSchema {
                expected: ANALYSIS_SCHEMA_VERSION,
                actual: self.schema_version,
            });
        }
        if self.session_id != expected_session_id {
            return Err(Error::CheckpointSession {
                expected: expected_session_id,
                actual: self.session_id,
            });
        }
        if self.analysis_identity.model.trim().is_empty() {
            return Err(Error::BlankCheckpointModel);
        }
        if self.analysis_identity.endpoint_id.trim().is_empty() {
            return Err(Error::BlankCheckpointEndpointId);
        }
        if self.checklist.is_empty() {
            return Err(Error::EmptyCheckpointChecklist);
        }
        if self.plan_fingerprint.is_empty() {
            return Err(Error::EmptyCheckpointPlanFingerprint);
        }
        if self.responses.len() > self.total_batches {
            return Err(Error::ProgressExceedsPlan {
                completed: self.responses.len(),
                total: self.total_batches,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use crate::analysis::{
        agent::{AnalysisResponse, ChecklistProgress, Observation},
        error::Error,
        video::AnalysisWarning,
    };

    use super::{ANALYSIS_SCHEMA_VERSION, AnalysisCheckpoint, AnalysisIdentity};

    const TOTAL_BATCHES: usize = 2;

    fn session_id() -> Uuid {
        Uuid::from_u128(1)
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

    fn analysis_identity() -> AnalysisIdentity {
        AnalysisIdentity {
            model: "model-byte-sentinel".into(),
            endpoint_id: "endpoint-byte-sentinel".into(),
        }
    }

    fn checkpoint() -> AnalysisCheckpoint {
        AnalysisCheckpoint {
            schema_version: ANALYSIS_SCHEMA_VERSION,
            session_id: session_id(),
            analysis_identity: analysis_identity(),
            checklist: "Start the exercise".into(),
            plan_fingerprint: "0123456789abcdef".into(),
            total_batches: TOTAL_BATCHES,
            warnings: vec![AnalysisWarning::RecordingGap {
                camera_id: 2,
                start_offset_ms: 1_000,
                end_offset_ms: 2_000,
            }],
            responses: vec![response("Batch zero is complete.")],
        }
    }

    #[test]
    fn checkpoint_v3_round_trips_identity_and_results() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("analysis.json");
        let expected = checkpoint();
        assert_eq!(ANALYSIS_SCHEMA_VERSION, 3);
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&expected).expect("checkpoint should serialize"),
        )
        .expect("checkpoint should be written");

        let actual =
            AnalysisCheckpoint::read(&path, session_id()).expect("valid v3 checkpoint should load");

        assert_eq!(actual, expected);
        assert_eq!(actual.analysis_identity, analysis_identity());
    }

    #[test]
    fn genuine_v2_checkpoint_is_schema_mismatch() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("analysis.json");
        let fixture = json!({
            "schema_version": 2,
            "session_id": session_id(),
            "checklist": "Start the exercise",
            "plan_fingerprint": "0123456789abcdef",
            "total_batches": 1,
            "warnings": [],
            "responses": []
        });
        assert!(fixture.get("analysis_identity").is_none());
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&fixture).expect("v2 fixture should serialize"),
        )
        .expect("v2 fixture should be written");

        assert!(matches!(
            AnalysisCheckpoint::read(&path, session_id()),
            Err(Error::CheckpointSchema {
                expected: ANALYSIS_SCHEMA_VERSION,
                actual: 2
            })
        ));
    }

    #[test]
    fn read_rejects_unknown_or_blank_identity() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("analysis.json");
        let valid = serde_json::to_value(checkpoint()).expect("checkpoint should serialize");

        for field in ["model", "endpoint_id"] {
            let mut value = valid.clone();
            value["analysis_identity"][field] = json!(" \n\t");
            std::fs::write(
                &path,
                serde_json::to_vec_pretty(&value).expect("invalid checkpoint should serialize"),
            )
            .expect("invalid checkpoint should be written");

            let error = AnalysisCheckpoint::read(&path, session_id())
                .expect_err("blank identity field must be rejected");
            match field {
                "model" => assert!(matches!(error, Error::BlankCheckpointModel)),
                "endpoint_id" => {
                    assert!(matches!(error, Error::BlankCheckpointEndpointId))
                }
                _ => unreachable!(),
            }
        }

        let mut unknown = valid;
        unknown["analysis_identity"]
            .as_object_mut()
            .expect("identity should be an object")
            .insert("provider".into(), json!("must be rejected"));
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&unknown).expect("invalid checkpoint should serialize"),
        )
        .expect("invalid checkpoint should be written");

        assert!(matches!(
            AnalysisCheckpoint::read(&path, session_id()),
            Err(Error::Json(_))
        ));
    }

    #[test]
    fn read_rejects_wrong_schema_session_empty_plan_fields_and_excess_responses() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("analysis.json");
        let valid = serde_json::to_value(checkpoint()).expect("checkpoint should serialize");
        let invalid = [
            ("wrong schema", {
                let mut value = valid.clone();
                value["schema_version"] = json!(ANALYSIS_SCHEMA_VERSION - 1);
                value
            }),
            ("wrong session", {
                let mut value = valid.clone();
                value["session_id"] = json!(Uuid::from_u128(2));
                value
            }),
            ("empty checklist", {
                let mut value = valid.clone();
                value["checklist"] = json!("");
                value
            }),
            ("empty fingerprint", {
                let mut value = valid.clone();
                value["plan_fingerprint"] = json!("");
                value
            }),
            ("excess responses", {
                let mut value = valid.clone();
                value["total_batches"] = json!(0);
                value
            }),
        ];

        for (reason, value) in invalid {
            std::fs::write(
                &path,
                serde_json::to_vec_pretty(&value).expect("invalid checkpoint should serialize"),
            )
            .expect("invalid checkpoint should be written");

            AnalysisCheckpoint::read(&path, session_id()).expect_err(reason);
        }
    }

    #[cfg(unix)]
    #[test]
    fn read_rejects_a_symlinked_checkpoint() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let target = directory.path().join("target.json");
        let link = directory.path().join("analysis.json");
        std::fs::write(
            &target,
            serde_json::to_vec_pretty(&checkpoint()).expect("checkpoint should serialize"),
        )
        .expect("checkpoint target should be written");
        symlink(&target, &link).expect("checkpoint symlink should be created");

        AnalysisCheckpoint::read(&link, session_id())
            .expect_err("a symlink must not be accepted as a checkpoint file");
    }
}

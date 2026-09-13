use std::time::{SystemTime, UNIX_EPOCH};

use backend::analysis::{HumanCorrection, RunStore};
use dioxus::prelude::*;
use uuid::Uuid;

use super::AnalysisContext;

/// Append an operator judgment to correction history without modifying any model output.
pub fn save_correction(
    mut context: AnalysisContext,
    session_id: Uuid,
    run_id: Uuid,
    criterion_id: Uuid,
) {
    let (directory, submitted, correction) = {
        let mut page = context.state.write();
        let Some(session) = page.sessions.get_mut(&session_id) else {
            return;
        };
        let Some(entry) = session.runs.get_mut(&run_id) else {
            return;
        };
        let draft = entry.corrections.entry(criterion_id).or_default();
        if draft.saving {
            return;
        }
        if draft.reason.trim().is_empty() {
            draft.error = Some("Add a reason for your correction.".into());
            return;
        }
        let created_utc_ms = match SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|time| u64::try_from(time.as_millis()).ok())
        {
            Some(time) => time,
            None => {
                draft.error = Some("The system clock could not timestamp this correction.".into());
                return;
            }
        };
        let supersedes = entry
            .snapshot
            .corrections
            .iter()
            .rev()
            .find(|correction| correction.criterion_id == criterion_id)
            .map(|correction| correction.id);
        let correction = HumanCorrection {
            id: Uuid::new_v4(),
            criterion_id,
            judgment: draft.judgment,
            reason: draft.reason.clone(),
            evidence_ids: draft.evidence_ids.clone(),
            uses_outside_information: draft.outside_information,
            created_utc_ms,
            supersedes,
        };
        draft.saving = true;
        draft.error = None;
        (session.directory.clone(), draft.clone(), correction)
    };
    dioxus::dioxus_core::spawn_forever(async move {
        let result = tokio::task::spawn_blocking(move || {
            let store = RunStore::new(directory);
            store.append_human_correction(run_id, correction)?;
            store.load(run_id).map(|snapshot| snapshot.corrections)
        })
        .await
        .map_err(|_| "The correction could not be saved.".to_owned())
        .and_then(|result| result.map_err(|error| error.to_string()));
        let mut page = context.state.write();
        match result {
            Ok(corrections) => {
                page.apply_corrections(session_id, run_id, corrections);
                if let Some(draft) = page
                    .sessions
                    .get_mut(&session_id)
                    .and_then(|session| session.runs.get_mut(&run_id))
                    .and_then(|entry| entry.corrections.get_mut(&criterion_id))
                {
                    if draft.reason == submitted.reason
                        && draft.judgment == submitted.judgment
                        && draft.evidence_ids == submitted.evidence_ids
                        && draft.outside_information == submitted.outside_information
                    {
                        draft.reason.clear();
                    }
                    draft.saving = false;
                }
            }
            Err(error) => {
                if let Some(draft) = page
                    .sessions
                    .get_mut(&session_id)
                    .and_then(|session| session.runs.get_mut(&run_id))
                    .and_then(|entry| entry.corrections.get_mut(&criterion_id))
                {
                    draft.saving = false;
                    draft.error = Some(error);
                }
            }
        }
    });
}

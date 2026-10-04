//! Operator decisions are append-only and bound to the exact results they reviewed.
use super::{RunError, RunResult, RunStore, model::*, store};
use uuid::Uuid;

impl RunSnapshot {
    /// Identity of one model finding and its current human correction.
    pub fn criterion_basis(&self, id: Uuid) -> RunResult<String> {
        let assessment = self
            .checkpoint
            .assessments
            .iter()
            .find(|assessment| assessment.criterion_id == id)
            .ok_or_else(|| RunError::Invalid("This criterion has not been assessed yet".into()))?;
        let correction = self
            .corrections
            .iter()
            .rev()
            .find(|correction| correction.criterion_id == id);
        Ok(store::hash(&serde_json::to_vec(&(
            self.inputs.id,
            assessment,
            correction,
        ))?))
    }
    pub fn is_reviewed(&self, id: Uuid) -> bool {
        self.criterion_basis(id).is_ok_and(|basis| {
            self.operator_review
                .marks
                .iter()
                .rev()
                .find(|mark| mark.criterion_id == id)
                .is_some_and(|mark| mark.basis == basis)
        })
    }
    pub fn assessment_basis(&self) -> RunResult<String> {
        let criteria = self
            .inputs
            .exercise
            .checklist
            .criteria
            .iter()
            .map(|item| self.criterion_basis(item.id))
            .collect::<RunResult<Vec<_>>>()?;
        Ok(store::hash(&serde_json::to_vec(&criteria)?))
    }
    pub fn current_student_assessment(&self) -> Option<&StudentAssessment> {
        if self.checkpoint.stage != RunStage::Ready {
            return None;
        }
        let basis = self.assessment_basis().ok()?;
        self.operator_review
            .assessments
            .last()
            .filter(|assessment| assessment.basis == basis)
    }
}
impl RunStore {
    pub fn append_human_correction(&self, id: Uuid, correction: HumanCorrection) -> RunResult<()> {
        let _lock = self.review_lock(id)?;
        let mut snapshot = self.load(id)?;
        if correction.id.is_nil()
            || correction.reason.trim().is_empty()
            || snapshot
                .corrections
                .iter()
                .any(|item| item.id == correction.id)
            || !snapshot
                .inputs
                .exercise
                .checklist
                .criteria
                .iter()
                .any(|item| item.id == correction.criterion_id)
        {
            return Err(RunError::Invalid(
                "Correction needs a unique identity, criterion and reason".into(),
            ));
        }
        let previous = snapshot
            .corrections
            .iter()
            .rev()
            .find(|item| item.criterion_id == correction.criterion_id)
            .map(|item| item.id);
        if previous != correction.supersedes {
            return Err(RunError::Invalid(
                "Another correction was saved; reload before editing".into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        if correction
            .evidence_ids
            .iter()
            .any(|id| snapshot.checkpoint.evidence.frame(id).is_none() || !seen.insert(id))
        {
            return Err(RunError::Invalid(
                "Correction has invalid candidate citations".into(),
            ));
        }
        snapshot.corrections.push(correction);
        store::write_json(
            &self.directory(id)?.join("review.json"),
            &snapshot.corrections,
            true,
        )
    }
    pub fn mark_reviewed(
        &self,
        id: Uuid,
        criterion_id: Uuid,
        expected_basis: &str,
    ) -> RunResult<RunSnapshot> {
        let _lock = self.review_lock(id)?;
        let mut snapshot = self.load(id)?;
        let basis = snapshot.criterion_basis(criterion_id)?;
        if basis != expected_basis {
            return Err(RunError::Invalid(
                "The result changed; inspect it again before marking reviewed".into(),
            ));
        }
        snapshot.operator_review.marks.push(ReviewMark {
            criterion_id,
            basis,
            created_utc_ms: now()?,
        });
        store::write_json(
            &self.directory(id)?.join("operator-review.json"),
            &snapshot.operator_review,
            true,
        )?;
        Ok(snapshot)
    }
    pub fn save_student_assessment(
        &self,
        id: Uuid,
        outcome: StudentOutcome,
        note: String,
        expected_basis: &str,
    ) -> RunResult<RunSnapshot> {
        let _execution = self.execution_lock(id)?;
        let _lock = self.review_lock(id)?;
        let mut snapshot = self.load(id)?;
        let basis = snapshot.assessment_basis()?;
        if snapshot.checkpoint.stage != RunStage::Ready
            || basis != expected_basis
            || note.trim().is_empty()
            || !snapshot
                .inputs
                .exercise
                .checklist
                .criteria
                .iter()
                .all(|item| snapshot.is_reviewed(item.id))
        {
            return Err(RunError::Invalid(
                "Finish reviewing every current result and add an assessment note".into(),
            ));
        }
        snapshot
            .operator_review
            .assessments
            .push(StudentAssessment {
                outcome,
                note,
                basis,
                created_utc_ms: now()?,
            });
        store::write_json(
            &self.directory(id)?.join("operator-review.json"),
            &snapshot.operator_review,
            true,
        )?;
        Ok(snapshot)
    }
}
pub fn now() -> RunResult<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|time| time.as_millis().try_into().ok())
        .ok_or_else(|| RunError::Invalid("Invalid system clock".into()))
}

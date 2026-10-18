use backend::analysis::{
    CreateRun, EvidencePolicy, ExecuteRun, RunSnapshot, create_run, execute_run,
};
use dioxus::prelude::*;
use uuid::Uuid;

use crate::{
    exercises::ExerciseContext,
    inference::InferenceGate,
    operator::{OperatorState, SessionRunState},
    settings::ResolvedSettings,
};

use super::{AnalysisContext, state::ActiveRun};

/// Start a new saved run from exactly the operator's published configuration and selected models.
pub fn start(
    mut context: AnalysisContext,
    operator: Signal<OperatorState>,
    exercises: ExerciseContext,
    settings: ResolvedSettings,
    gate: InferenceGate,
    session_id: Uuid,
    model_profile_id: String,
) {
    let prepared = (|| {
        let directory = selected_directory(context, operator, session_id, None)?;
        let revision = exercises
            .state
            .read()
            .selected_revision()
            .ok_or("Save the exercise configuration before starting analysis.")?;
        let workflow = &settings.settings.workflow;
        let model = workflow
            .resolve_model(&model_profile_id)
            .map_err(|error| error.to_string())?;
        let options = context.state.read();
        let evidence = &settings.settings.workflow.evidence;
        Ok::<_, String>(CreateRun {
            session_directory: directory,
            expected_session_id: session_id,
            exercise_store: exercises.store,
            revision,
            model,
            execution: settings.settings.workflow.execution.clone(),
            evidence_policy: EvidencePolicy {
                max_window_span_ms: evidence.max_prompt_span_ms,
                overlap_frame_sets: evidence.overlap_frame_sets,
                image_size: evidence.image_size,
                image_detail: evidence.image_detail,
                reference_images: options.reference_images,
                automatic_recheck: options.automatic_recheck,
            },
        })
    })();
    let request = match prepared {
        Ok(request) => request,
        Err(error) => {
            show_error(context, session_id, error);
            return;
        }
    };
    let lease = match gate.acquire("recording analysis") {
        Ok(lease) => lease,
        Err(error) => {
            show_error(context, session_id, error);
            return;
        }
    };
    let directory = request.session_directory.clone();
    let selected_before = {
        let mut page = context.state.write();
        let session = page.sessions.entry(session_id).or_default();
        session.directory = directory.clone();
        session.error = None;
        session.selected_run_id
    };
    let mut activity = RunActivity::begin(context, operator, session_id, None);
    dioxus::dioxus_core::spawn_forever(async move {
        let _lease = lease;
        let result = async {
            let created = create_run(request)
                .await
                .map_err(|error| error.to_string())?;
            let run_id = created.inputs.id;
            activity.set_run(run_id);
            {
                let mut page = context.state.write();
                page.apply_snapshot(created);
                let session = page
                    .sessions
                    .get_mut(&session_id)
                    .expect("active recording history exists");
                if session.selected_run_id == selected_before {
                    session.selected_run_id = Some(run_id);
                }
            }
            execute(
                context,
                ExecuteRun {
                    session_directory: directory,
                    run_id,
                    providers: settings.settings.workflow.providers,
                },
            )
            .await
        }
        .await;
        if let Err(error) = result {
            show_error(context, session_id, error);
        }
        drop(activity);
    });
}

/// Resume with frozen run inputs. Current settings supply credentials, not replacement model choices.
pub fn resume(
    mut context: AnalysisContext,
    operator: Signal<OperatorState>,
    settings: ResolvedSettings,
    gate: InferenceGate,
    session_id: Uuid,
    run_id: Uuid,
) {
    let directory = match selected_directory(context, operator, session_id, Some(run_id)) {
        Ok(directory) => directory,
        Err(error) => {
            show_error(context, session_id, error);
            return;
        }
    };
    let lease = match gate.acquire("recording analysis") {
        Ok(lease) => lease,
        Err(error) => {
            show_error(context, session_id, error);
            return;
        }
    };
    if let Some(session) = context.state.write().sessions.get_mut(&session_id) {
        session.error = None;
    }
    let activity = RunActivity::begin(context, operator, session_id, Some(run_id));
    dioxus::dioxus_core::spawn_forever(async move {
        let _lease = lease;
        let result = execute(
            context,
            ExecuteRun {
                session_directory: directory,
                run_id,
                providers: settings.settings.workflow.providers,
            },
        )
        .await;
        if let Err(error) = result {
            show_error(context, session_id, error);
        }
        drop(activity);
    });
}

async fn execute(mut context: AnalysisContext, request: ExecuteRun) -> Result<(), String> {
    let run_id = request.run_id;
    let result = execute_run(request, move |snapshot: RunSnapshot| {
        if snapshot.inputs.id == run_id {
            context.state.write().apply_snapshot(snapshot);
        }
    })
    .await
    .map_err(|error| error.to_string())?;
    context.state.write().apply_snapshot(result);
    Ok(())
}

/// Reject queued actions before they can bind old cached results to another recording folder.
fn selected_directory(
    context: AnalysisContext,
    operator: Signal<OperatorState>,
    expected: Uuid,
    run_id: Option<Uuid>,
) -> Result<std::path::PathBuf, String> {
    let operator = operator.read();
    if !matches!(operator.session, SessionRunState::Idle) {
        return Err("Analysis is unavailable while recording is active.".into());
    }
    if operator.running_analysis_id.is_some() {
        return Err("Another analysis is already running.".into());
    }
    if operator.selected_session_id != Some(expected) {
        return Err(
            "The selected recording changed. Start again from its current selection.".into(),
        );
    }
    let directory = operator
        .sessions
        .iter()
        .find(|row| row.stored.session.id == expected)
        .map(|row| row.stored.directory.clone())
        .ok_or("Select a completed recording.")?;
    let page = context.state.read();
    let session = page.sessions.get(&expected);
    if session.is_some_and(|session| session.directory != directory) {
        return Err(
            "The recording folder changed. Wait for its saved history to load, then try again."
                .into(),
        );
    }
    if session.is_some_and(|session| session.loading) {
        return Err("Wait for this recording's saved history to finish loading.".into());
    }
    if let Some(id) = run_id {
        let entry = session
            .and_then(|session| session.runs.get(&id))
            .filter(|entry| entry.available)
            .ok_or("Refresh this recording's history and select an available run.")?;
        if entry.assessment_draft.saving {
            return Err("Wait for this run's review decision to finish saving.".into());
        }
    }
    Ok(directory)
}

fn show_error(mut context: AnalysisContext, session_id: Uuid, error: String) {
    context
        .state
        .write()
        .sessions
        .entry(session_id)
        .or_default()
        .error = Some(error);
}

/// Keeps recording admission and run progress ownership aligned, including failed preparation.
struct RunActivity {
    context: AnalysisContext,
    operator: Signal<OperatorState>,
    identity: ActiveRun,
}
impl RunActivity {
    fn begin(
        mut context: AnalysisContext,
        mut operator: Signal<OperatorState>,
        session_id: Uuid,
        run_id: Option<Uuid>,
    ) -> Self {
        let identity = ActiveRun {
            token: Uuid::new_v4(),
            session_id,
            run_id,
        };
        context.state.write().active = Some(identity);
        operator.write().running_analysis_id = Some(identity.token);
        Self {
            context,
            operator,
            identity,
        }
    }
    fn set_run(&mut self, run_id: Uuid) {
        self.identity.run_id = Some(run_id);
        self.context.state.write().active = Some(self.identity);
    }
}
impl Drop for RunActivity {
    fn drop(&mut self) {
        let context = self.context;
        let operator = self.operator;
        let token = self.identity.token;
        if !release_activity(context, operator, token) {
            // Root tasks outlive route callbacks. Retry after any current UI borrow is released.
            dioxus::dioxus_core::spawn_forever(async move {
                while !release_activity(context, operator, token) {
                    tokio::task::yield_now().await;
                }
            });
        }
    }
}

fn release_activity(
    mut context: AnalysisContext,
    mut operator: Signal<OperatorState>,
    token: Uuid,
) -> bool {
    let page_released = if let Ok(mut page) = context.state.try_write() {
        if page.active.is_some_and(|active| active.token == token) {
            page.active = None;
        }
        true
    } else {
        false
    };
    let recording_released = if let Ok(mut operator) = operator.try_write() {
        if operator.running_analysis_id == Some(token) {
            operator.running_analysis_id = None;
        }
        true
    } else {
        false
    };
    page_released && recording_released
}

#[cfg(test)]
#[path = "tests/execution.rs"]
mod tests;

/// Reserve and execute the same one-per-criterion closer look used by automatic analysis.
pub fn recheck(
    mut context: AnalysisContext,
    operator: Signal<OperatorState>,
    settings: ResolvedSettings,
    gate: InferenceGate,
    session_id: Uuid,
    run_id: Uuid,
    criterion_id: Uuid,
    frame_id: Option<String>,
) {
    let directory = match selected_directory(context, operator, session_id, Some(run_id)) {
        Ok(directory) => directory,
        Err(error) => {
            show_error(context, session_id, error);
            return;
        }
    };
    let lease = match gate.acquire("independent closer look") {
        Ok(lease) => lease,
        Err(error) => {
            show_error(context, session_id, error);
            return;
        }
    };
    let activity = RunActivity::begin(context, operator, session_id, Some(run_id));
    dioxus::dioxus_core::spawn_forever(async move {
        let _lease = lease;
        let result = async {
            let snapshot = backend::analysis::request_recheck(
                directory.clone(),
                run_id,
                criterion_id,
                frame_id,
            )
            .await
            .map_err(|error| error.to_string())?;
            context.state.write().apply_snapshot(snapshot);
            execute(
                context,
                ExecuteRun {
                    session_directory: directory,
                    run_id,
                    providers: settings.settings.workflow.providers,
                },
            )
            .await
        }
        .await;
        if let Err(error) = result {
            show_error(context, session_id, error);
        }
        drop(activity);
    });
}

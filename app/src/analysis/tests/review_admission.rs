use super::*;
use crate::analysis::{self, AnalysisContext};
use dioxus::{dioxus_core::NoOpMutations, prelude::*};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
struct Props(Rc<RefCell<Option<AnalysisContext>>>);

fn root(props: Props) -> Element {
    analysis::use_analysis_context();
    *props.0.borrow_mut() = Some(use_context());
    rsx! { div {} }
}

fn with_run(test: impl FnOnce(&mut VirtualDom, AnalysisContext, Uuid, Uuid, Uuid)) {
    let saved_context = Rc::new(RefCell::new(None));
    let mut dom = VirtualDom::new_with_props(root, Props(saved_context.clone()));
    dom.rebuild(&mut NoOpMutations);
    let mut context = saved_context.borrow().unwrap();
    let session_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let snapshot = test_snapshot(session_id, run_id, RunStage::Ready);
    let criterion_id = snapshot.inputs.exercise.checklist.criteria[0].id;
    context.state.write().apply_snapshot(snapshot);
    test(&mut dom, context, session_id, run_id, criterion_id);
}

#[test]
fn correction_waits_for_another_criterion_or_overall_review_write() {
    for pending_overall in [false, true] {
        with_run(|dom, mut context, session_id, run_id, criterion_id| {
            {
                let mut page = context.state.write();
                let entry = page
                    .sessions
                    .get_mut(&session_id)
                    .unwrap()
                    .runs
                    .get_mut(&run_id)
                    .unwrap();
                if pending_overall {
                    entry.assessment_draft.saving = true;
                } else {
                    entry.corrections.entry(Uuid::new_v4()).or_default().saving = true;
                }
                let draft = entry.corrections.entry(criterion_id).or_default();
                draft.reason = "Waiting for the previous write".into();
                draft.outside_information = true;
            }
            dom.in_runtime(|| {
                analysis::save_correction(context, session_id, run_id, criterion_id, None)
            });
            let page = context.state.read();
            let draft = &page.sessions[&session_id].runs[&run_id].corrections[&criterion_id];
            assert!(!draft.saving, "the second write must not be admitted");
            assert_eq!(draft.reason, "Waiting for the previous write");
            assert!(draft.error.is_none());
        });
    }
}

#[test]
fn review_mark_waits_for_a_pending_correction_write() {
    with_run(|dom, mut context, session_id, run_id, criterion_id| {
        context
            .state
            .write()
            .sessions
            .get_mut(&session_id)
            .unwrap()
            .runs
            .get_mut(&run_id)
            .unwrap()
            .corrections
            .entry(criterion_id)
            .or_default()
            .saving = true;
        dom.in_runtime(|| analysis::mark_reviewed(context, session_id, run_id, criterion_id));
        assert!(
            !context.state.read().sessions[&session_id].runs[&run_id]
                .assessment_draft
                .saving
        );
    });
}

#[test]
fn overall_assessment_waits_for_a_pending_correction_write() {
    with_run(|dom, mut context, session_id, run_id, criterion_id| {
        context
            .state
            .write()
            .sessions
            .get_mut(&session_id)
            .unwrap()
            .runs
            .get_mut(&run_id)
            .unwrap()
            .corrections
            .entry(criterion_id)
            .or_default()
            .saving = true;
        dom.in_runtime(|| analysis::save_assessment(context, session_id, run_id));
        assert!(
            !context.state.read().sessions[&session_id].runs[&run_id]
                .assessment_draft
                .saving
        );
    });
}

#[test]
fn overall_assessment_waits_only_for_processing_of_the_same_run() {
    enum Work {
        Preparing,
        OtherRun,
        SameRun,
    }
    for work in [Work::Preparing, Work::OtherRun, Work::SameRun] {
        with_run(|dom, mut context, session_id, run_id, _| {
            let (active_run, should_wait) = match work {
                Work::Preparing => (None, false),
                Work::OtherRun => (Some(Uuid::new_v4()), false),
                Work::SameRun => (Some(run_id), true),
            };
            context.state.write().active = Some(ActiveRun {
                token: Uuid::new_v4(),
                session_id,
                run_id: active_run,
            });
            dom.in_runtime(|| analysis::save_assessment(context, session_id, run_id));
            assert_eq!(
                context.state.read().sessions[&session_id].runs[&run_id]
                    .assessment_draft
                    .saving,
                !should_wait,
                "only processing of this run can block its overall decision"
            );
        });
    }
}

use super::*;
use backend::recording::{RecorderSettings, test_support};
use dioxus::dioxus_core::NoOpMutations;
use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

type Contexts = (AnalysisContext, Signal<OperatorState>);

#[derive(Clone)]
struct Props {
    initial: Rc<RefCell<Option<OperatorState>>>,
    contexts: Rc<RefCell<Option<Contexts>>>,
}

fn root(props: Props) -> Element {
    super::super::use_analysis_context();
    let operator = use_hook(|| {
        Signal::new_in_scope(props.initial.borrow_mut().take().unwrap(), ScopeId::ROOT)
    });
    *props.contexts.borrow_mut() = Some((use_context(), operator));
    rsx! { div { "Run admission test" } }
}

fn with_contexts(test: impl FnOnce(&mut VirtualDom, Contexts)) {
    let directory = tempfile::tempdir().unwrap();
    let (runtime, recorder, _events) = test_support::spawn(
        RecorderSettings {
            io_timeout: Duration::from_secs(1),
            retry_delay: Duration::from_secs(1),
            stop_timeout: Duration::from_secs(1),
        },
        PathBuf::from("unused-test-ffmpeg"),
        PathBuf::from("unused-test-ffprobe"),
    )
    .unwrap();
    let operator = OperatorState::new(
        crate::test_settings(Vec::new(), None, 5, 0),
        directory.path().join("sessions"),
        recorder,
    )
    .unwrap();
    let contexts = Rc::new(RefCell::new(None));
    let mut dom = VirtualDom::new_with_props(
        root,
        Props {
            initial: Rc::new(RefCell::new(Some(operator))),
            contexts: contexts.clone(),
        },
    );
    dom.rebuild(&mut NoOpMutations);
    let contexts = contexts.borrow().unwrap();
    test(&mut dom, contexts);
    drop(dom);
    runtime.shutdown().unwrap();
}

#[test]
fn borrowed_signals_do_not_leave_recording_blocked_after_preparation_ends() {
    with_contexts(|dom, (context, operator)| {
        let activity =
            dom.in_runtime(|| RunActivity::begin(context, operator, Uuid::new_v4(), None));
        dom.in_runtime(|| {
            let page = context.state.read();
            let recording = operator.read();
            drop(activity);
            assert!(page.active.is_some());
            assert!(recording.running_analysis_id.is_some());
        });
        dom.render_immediate(&mut NoOpMutations);
        assert!(context.state.read().active.is_none());
        assert!(operator.read().running_analysis_id.is_none());
    });
}

#[test]
fn deferred_cleanup_does_not_release_a_new_preparation_for_the_same_recording() {
    with_contexts(|dom, (context, operator)| {
        let session = Uuid::new_v4();
        let next = dom.in_runtime(|| {
            let old = RunActivity::begin(context, operator, session, None);
            let page = context.state.read();
            let recording = operator.read();
            drop(old);
            drop(page);
            drop(recording);
            RunActivity::begin(context, operator, session, None)
        });
        dom.render_immediate(&mut NoOpMutations);
        assert_eq!(
            context.state.read().active.unwrap().token,
            next.identity.token
        );
        assert_eq!(
            operator.read().running_analysis_id,
            Some(next.identity.token)
        );
        dom.in_runtime(|| drop(next));
    });
}

fn select_completed_recording(mut operator: Signal<OperatorState>) -> (Uuid, PathBuf) {
    let directory = operator.read().session_root.join("surviving-copy");
    std::fs::create_dir_all(&directory).unwrap();
    let mut controller = backend::session::SessionController::create(
        directory.join("events.jsonl"),
        vec![backend::session::SessionCamera {
            id: 1,
            name: "Fixture camera".into(),
            enabled: true,
            initial_monitoring_profile_id: 1,
        }],
        crate::test_monitoring_profiles(),
    )
    .unwrap();
    controller
        .apply(backend::session::OperatorAction::EndSession)
        .unwrap();
    backend::session::mark_recording_complete(&directory).unwrap();
    operator.write().refresh_sessions().unwrap();
    let session_id = operator.read().selected_session_id.unwrap();
    (session_id, directory)
}

#[test]
fn changed_recording_folder_cannot_admit_work_from_its_old_cached_history() {
    with_contexts(|dom, (mut context, operator)| {
        let (session_id, _) = select_completed_recording(operator);
        let run_id = Uuid::new_v4();
        dom.in_runtime(|| {
            let mut page = context.state.write();
            page.apply_snapshot(super::super::test_snapshot(
                session_id,
                run_id,
                backend::analysis::RunStage::Ready,
            ));
            let session = page.sessions.get_mut(&session_id).unwrap();
            session.directory = "/removed-original".into();
            session.loaded = true;
            session
                .runs
                .get_mut(&run_id)
                .unwrap()
                .assessment_draft
                .saving = true;
        });

        for run in [None, Some(run_id)] {
            assert!(selected_directory(context, operator, session_id, run).is_err());
        }
        assert_eq!(
            context.state.read().sessions[&session_id].directory,
            PathBuf::from("/removed-original")
        );
        assert!(context.state.read().active.is_none());
        assert!(operator.read().running_analysis_id.is_none());
    });
}

#[test]
fn history_loading_blocks_new_and_existing_run_work() {
    with_contexts(|dom, (mut context, operator)| {
        let (session_id, directory) = select_completed_recording(operator);
        dom.in_runtime(|| {
            context
                .state
                .write()
                .begin_history_load(session_id, directory.clone(), false)
                .unwrap();
        });
        for run in [None, Some(Uuid::new_v4())] {
            assert!(selected_directory(context, operator, session_id, run).is_err());
        }
    });
}

#[test]
fn existing_run_work_requires_an_available_cached_run() {
    with_contexts(|dom, (mut context, operator)| {
        let (session_id, directory) = select_completed_recording(operator);
        let run_id = Uuid::new_v4();
        assert_eq!(
            selected_directory(context, operator, session_id, None).unwrap(),
            directory
        );
        assert!(selected_directory(context, operator, session_id, Some(run_id)).is_err());
        dom.in_runtime(|| {
            let mut page = context.state.write();
            page.apply_snapshot(super::super::test_snapshot(
                session_id,
                run_id,
                backend::analysis::RunStage::Ready,
            ));
            page.sessions.get_mut(&session_id).unwrap().directory = directory.clone();
        });
        assert_eq!(
            selected_directory(context, operator, session_id, Some(run_id)).unwrap(),
            directory
        );
        context
            .state
            .write()
            .sessions
            .get_mut(&session_id)
            .unwrap()
            .runs
            .get_mut(&run_id)
            .unwrap()
            .available = false;
        assert!(selected_directory(context, operator, session_id, Some(run_id)).is_err());
        assert_eq!(
            selected_directory(context, operator, session_id, None).unwrap(),
            directory
        );
    });
}

#[test]
fn pending_review_decision_blocks_only_its_existing_runs_model_actions() {
    with_contexts(|dom, (mut context, operator)| {
        let (session_id, directory) = select_completed_recording(operator);
        let run_id = Uuid::new_v4();
        let other_run = Uuid::new_v4();
        dom.in_runtime(|| {
            let mut page = context.state.write();
            for id in [run_id, other_run] {
                page.apply_snapshot(super::super::test_snapshot(
                    session_id,
                    id,
                    backend::analysis::RunStage::Ready,
                ));
            }
            let session = page.sessions.get_mut(&session_id).unwrap();
            session.directory = directory.clone();
            session
                .runs
                .get_mut(&run_id)
                .unwrap()
                .assessment_draft
                .saving = true;
        });
        assert!(selected_directory(context, operator, session_id, Some(run_id)).is_err());
        for run in [None, Some(other_run)] {
            assert_eq!(
                selected_directory(context, operator, session_id, run).unwrap(),
                directory
            );
        }
        context
            .state
            .write()
            .sessions
            .get_mut(&session_id)
            .unwrap()
            .runs
            .get_mut(&run_id)
            .unwrap()
            .assessment_draft
            .saving = false;
        assert_eq!(
            selected_directory(context, operator, session_id, Some(run_id)).unwrap(),
            directory
        );
    });
}

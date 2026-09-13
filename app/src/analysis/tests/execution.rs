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

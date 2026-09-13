//! Root-owned admission for formatting and analysis, sharing one model memory allowance.
use dioxus::prelude::*;
use std::cell::Cell;
use uuid::Uuid;

/// A user-visible operation currently holding the application's inference allowance.
#[derive(Clone, PartialEq, Eq)]
pub struct InferenceActivity {
    pub token: Uuid,
    pub label: String,
}

/// Prevents unrelated formatter and analysis tasks from oversubscribing the same model budget.
#[derive(Clone, Copy)]
pub struct InferenceGate {
    pub active: Signal<Option<InferenceActivity>>,
    owner: Signal<Cell<Option<Uuid>>>,
}
impl InferenceGate {
    /// Reserves inference until the returned lease is dropped, including on errors.
    pub fn acquire(mut self, label: impl Into<String>) -> Result<InferenceLease, String> {
        let mut active = self.active.write();
        if self.owner.read().get().is_some() {
            return Err(format!(
                "Wait for {} to finish",
                active
                    .as_ref()
                    .map(|a| a.label.as_str())
                    .unwrap_or("model work")
            ));
        }
        let token = Uuid::new_v4();
        self.owner.read().set(Some(token));
        *active = Some(InferenceActivity {
            token,
            label: label.into(),
        });
        drop(active);
        Ok(InferenceLease { gate: self, token })
    }
}
/// Task-lifetime reservation. Its token prevents a stale task from clearing a newer operation.
pub struct InferenceLease {
    gate: InferenceGate,
    token: Uuid,
}
impl Drop for InferenceLease {
    fn drop(&mut self) {
        // Admission is independent of a UI display borrow, so release cannot get lost.
        if let Ok(owner) = self.gate.owner.try_read() {
            if owner.get() == Some(self.token) {
                owner.set(None);
            }
        }
        if let Ok(mut active) = self.gate.active.try_write() {
            if active.as_ref().is_some_and(|a| a.token == self.token) {
                *active = None;
            }
        } else {
            let mut gate = self.gate;
            let token = self.token;
            // Dioxus polls a new root task after the current callback releases its display borrow.
            dioxus::dioxus_core::spawn_forever(async move {
                if let Ok(mut active) = gate.active.try_write() {
                    if active.as_ref().is_some_and(|a| a.token == token) {
                        *active = None;
                    }
                }
            });
        }
    }
}
/// Installs the admission gate in the ready application root.
pub fn use_inference_gate() {
    let active = use_hook(|| Signal::new_in_scope(None, ScopeId::ROOT));
    let owner = use_hook(|| Signal::new_in_scope(Cell::new(None), ScopeId::ROOT));
    use_context_provider(|| InferenceGate { active, owner });
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::dioxus_core::NoOpMutations;
    use std::{cell::RefCell, rc::Rc};
    fn root(holder: Rc<RefCell<Option<InferenceGate>>>) -> Element {
        use_inference_gate();
        *holder.borrow_mut() = Some(use_context::<InferenceGate>());
        rsx! {
            div { "Inference gate test" }
        }
    }
    #[test]
    fn dropping_a_borrowed_display_releases_admission_without_clearing_a_newer_owner() {
        let holder = Rc::new(RefCell::new(None));
        let mut dom = VirtualDom::new_with_props(root, holder.clone());
        dom.rebuild(&mut NoOpMutations);
        let gate = holder.borrow().unwrap();
        let next = dom.in_runtime(|| {
            let first = gate.acquire("first").unwrap();
            let display = gate.active.read();
            drop(first);
            assert_eq!(gate.owner.read().get(), None);
            drop(display);
            gate.acquire("second").unwrap()
        });
        dom.render_immediate(&mut NoOpMutations);
        assert_eq!(gate.active.read().as_ref().unwrap().label, "second");
        dom.in_runtime(|| drop(next));
        assert!(gate.active.read().is_none());
    }
    #[test]
    fn concurrent_operations_cannot_claim_one_allowance() {
        let holder = Rc::new(RefCell::new(None));
        let mut dom = VirtualDom::new_with_props(root, holder.clone());
        dom.rebuild(&mut NoOpMutations);
        let gate = holder.borrow().unwrap();
        dom.in_runtime(|| {
            let first = gate.acquire("analysis").unwrap();
            assert!(gate.acquire("formatting").is_err());
            drop(first);
            assert!(gate.acquire("formatting").is_ok());
        });
    }
}

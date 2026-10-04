//! Persistent exercise editing and manual reference preparation, independent of recording selection.

mod checklist;
mod editor;
mod formatting;
mod preparation;
mod references;
mod state;
mod tasks;

pub use editor::ExerciseEditor;
#[cfg(test)]
pub use state::ExercisePageState;
pub use state::{ExerciseContext, use_exercise_context};

#[cfg(test)]
mod tests;

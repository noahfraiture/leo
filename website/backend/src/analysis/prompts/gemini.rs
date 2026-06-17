//! Gemini prompt construction.

/// Keep this function as the single edit point for Gemini prompt shaping so
/// future provider-specific instructions do not get scattered through the
/// request builder.
pub fn user_prompt(prompt: &str) -> String {
    format!("{prompt}\n\nReturn plain text, not Markdown.")
}

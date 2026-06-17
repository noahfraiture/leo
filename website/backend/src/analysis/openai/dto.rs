use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct OpenAiResponse {
    #[serde(default)]
    output_text: Option<String>,
    #[serde(default)]
    output: Vec<OpenAiOutputItem>,
}

impl OpenAiResponse {
    pub(super) fn text(self) -> Option<String> {
        let text = self
            .output_text
            .into_iter()
            .chain(
                self.output
                    .into_iter()
                    .flat_map(|item| item.content)
                    .filter_map(|content| content.text),
            )
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");

        if text.is_empty() { None } else { Some(text) }
    }
}

#[derive(Deserialize)]
struct OpenAiOutputItem {
    #[serde(default)]
    content: Vec<OpenAiContent>,
}

#[derive(Deserialize)]
struct OpenAiContent {
    text: Option<String>,
}

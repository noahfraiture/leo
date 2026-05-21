use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use axum::response::{IntoResponse, Response};

#[derive(Clone, Default)]
pub struct AppMetrics {
    counters: Arc<Mutex<BTreeMap<MetricKey, u64>>>,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct MetricKey {
    name: &'static str,
    labels: Vec<(&'static str, String)>,
}

impl AppMetrics {
    pub fn increment(&self, name: &'static str, labels: &[(&'static str, &str)]) {
        let key = MetricKey {
            name,
            labels: labels
                .iter()
                .map(|(key, value)| (*key, (*value).to_owned()))
                .collect(),
        };
        let mut counters = self
            .counters
            .lock()
            .expect("metrics mutex should not be poisoned");
        *counters.entry(key).or_insert(0) += 1;
    }

    pub fn render_prometheus(&self) -> String {
        let counters = self
            .counters
            .lock()
            .expect("metrics mutex should not be poisoned");
        let mut help = HashMap::from([
            (
                "leo_analysis_submissions_total",
                "Analysis jobs submitted by provider.",
            ),
            (
                "leo_analysis_jobs_total",
                "Analysis jobs by terminal or processing result.",
            ),
            (
                "leo_upload_sessions_total",
                "Chunked upload sessions by result.",
            ),
            (
                "leo_upload_chunks_total",
                "Chunked upload chunks by result.",
            ),
            (
                "leo_canary_runs_total",
                "Synthetic analysis canary runs by result.",
            ),
        ]);
        let mut lines = Vec::new();
        let mut emitted_metadata = Vec::new();

        for (key, value) in counters.iter() {
            if !emitted_metadata.contains(&key.name) {
                let description = help.remove(key.name).unwrap_or("Leo application counter.");
                lines.push(format!("# HELP {} {}", key.name, description));
                lines.push(format!("# TYPE {} counter", key.name));
                emitted_metadata.push(key.name);
            }

            lines.push(format!(
                "{}{} {}",
                key.name,
                render_labels(&key.labels),
                value
            ));
        }

        if lines.is_empty() {
            lines.push("# HELP leo_info Leo website metrics endpoint.".to_owned());
            lines.push("# TYPE leo_info gauge".to_owned());
            lines.push("leo_info 1".to_owned());
        }

        lines.push(String::new());
        lines.join("\n")
    }
}

pub async fn serve_metrics(
    axum::extract::State(state): axum::extract::State<crate::http::router::AppState>,
) -> Response {
    (
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        state.metrics().render_prometheus(),
    )
        .into_response()
}

fn render_labels(labels: &[(&'static str, String)]) -> String {
    if labels.is_empty() {
        return String::new();
    }

    let rendered = labels
        .iter()
        .map(|(key, value)| format!("{key}=\"{}\"", escape_label_value(value)))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{rendered}}}")
}

fn escape_label_value(value: &str) -> String {
    value
        .replace('\\', r"\\")
        .replace('\n', r"\n")
        .replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::AppMetrics;

    #[test]
    fn render_prometheus_escapes_label_values() {
        let metrics = AppMetrics::default();

        metrics.increment("leo_analysis_jobs_total", &[("provider", "open\"ai")]);

        assert!(
            metrics
                .render_prometheus()
                .contains(r#"leo_analysis_jobs_total{provider="open\"ai"} 1"#)
        );
    }
}

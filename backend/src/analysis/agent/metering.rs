//! Opt-in evaluation accounting; absent from production builds without test support.

use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rig_core::completion::{CompletionResponse, Usage};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::OpenAiConfig;
use crate::profiles::AnalysisProfile;

/// Journals request reservations and actual provider usage for one explicitly named run.
pub struct Metering {
    journal: PathBuf,
    run: String,
    paid: bool,
    ceiling: f64,
}

/// One durable reservation, retained conservatively if execution is interrupted.
pub struct Measurement<'a> {
    meter: &'a Metering,
    id: Uuid,
    started: Instant,
}

impl Metering {
    /// Enables metering only when an evaluation launcher supplies an absolute journal path.
    pub fn from_environment(
        profile: &AnalysisProfile,
        config: &OpenAiConfig,
    ) -> io::Result<Option<Self>> {
        let Some(path) = std::env::var_os("LEO_EVALUATION_JOURNAL") else {
            return Ok(None);
        };
        let journal = PathBuf::from(path);
        if !journal.is_absolute() {
            return Err(io::Error::other("evaluation journal must be absolute"));
        }
        let paid = match config.base_url.as_deref() {
            None => true,
            Some(base) => {
                let url = url::Url::parse(base).map_err(io::Error::other)?;
                match url.host_str() {
                    Some("127.0.0.1" | "[::1]" | "localhost") => false,
                    _ => return Err(io::Error::other("evaluation endpoint is not approved")),
                }
            }
        };
        if paid
            && (profile.model != "gpt-5.6-luna"
                || std::env::var("LEO_RUN_PAID_OPENAI_TEST").as_deref() != Ok("1"))
        {
            return Err(io::Error::other(
                "paid evaluation requires approved Luna opt-in",
            ));
        }
        let ceiling = std::env::var("LEO_EVALUATION_LIMIT_USD")
            .unwrap_or_else(|_| "100".into())
            .parse::<f64>()
            .map_err(io::Error::other)?;
        if !ceiling.is_finite() || !(0.0..=100.0).contains(&ceiling) {
            return Err(io::Error::other(
                "evaluation limit must be between zero and 100 USD",
            ));
        }
        Ok(Some(Self {
            journal,
            run: std::env::var("LEO_EVALUATION_RUN")
                .map_err(|_| io::Error::other("evaluation run identifier is required"))?,
            paid,
            ceiling,
        }))
    }

    /// Reserves before sending. USD 1 exceeds Luna's documented maximum standard request cost.
    pub fn start(&self) -> io::Result<Measurement<'_>> {
        let _lock = JournalLock::acquire(&self.journal)?;
        let reserved = if self.paid { 1.0 } else { 0.0 };
        if accounted_cost(&self.journal)? + reserved > self.ceiling {
            return Err(io::Error::other(
                "campaign budget cannot cover the next request",
            ));
        }
        let id = Uuid::new_v4();
        append(
            &self.journal,
            &json!({
                "event": "reserved", "id": id, "run": self.run,
                "paid": self.paid, "accounted_usd": reserved,
                "utc_ms": SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis()),
            }),
        )?;
        Ok(Measurement {
            meter: self,
            id,
            started: Instant::now(),
        })
    }
}

impl Measurement<'_> {
    /// Settles known usage; missing usage or failed transport keeps the full reservation.
    pub fn finish<T: Serialize>(
        self,
        response: Option<&CompletionResponse<T>>,
        parse_succeeded: Option<bool>,
    ) -> io::Result<()> {
        let usage = response.map(|r| r.usage).filter(Usage::has_values);
        let raw = response.and_then(|r| serde_json::to_value(&r.raw_response).ok());
        let measured = if self.meter.paid {
            usage.as_ref().map(luna_cost)
        } else {
            Some(0.0)
        };
        let _lock = JournalLock::acquire(&self.meter.journal)?;
        append(
            &self.meter.journal,
            &json!({
                "event": "finished", "id": self.id, "run": self.meter.run,
                "paid": self.meter.paid, "elapsed_ms": self.started.elapsed().as_millis(),
                "utc_ms": SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis()),
                "usage": usage, "measured_usd": measured,
                "accounted_usd": measured.unwrap_or(1.0),
                "response_id": raw.as_ref().and_then(|r| r.get("id")),
                "status": raw.as_ref().and_then(|r| r.get("status")),
                "service_tier": raw.as_ref().and_then(|r| r.get("service_tier")),
                "incomplete_details": raw.as_ref().and_then(|r| r.get("incomplete_details")),
                "transport_ok": response.is_some(),
                "parse_succeeded": parse_succeeded,
            }),
        )
    }
}

// Standard Luna prices verified 2026-09-05. Inputs over 272K use long-context rates.
fn luna_cost(usage: &Usage) -> f64 {
    let (input, cached, writes, output) = if usage.input_tokens > 272_000 {
        (0.40, 0.04, 0.50, 1.80)
    } else {
        (0.20, 0.02, 0.25, 1.20)
    };
    let ordinary = usage
        .input_tokens
        .saturating_sub(usage.cached_input_tokens)
        .saturating_sub(usage.cache_creation_input_tokens);
    (ordinary as f64 * input
        + usage.cached_input_tokens as f64 * cached
        + usage.cache_creation_input_tokens as f64 * writes
        + usage.output_tokens as f64 * output)
        / 1_000_000.0
}

fn accounted_cost(path: &Path) -> io::Result<f64> {
    let contents = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0.0),
        Err(e) => return Err(e),
    };
    let mut requests = HashMap::new();
    for line in contents.lines() {
        let row: Value = serde_json::from_str(line).map_err(io::Error::other)?;
        let id = row["id"]
            .as_str()
            .ok_or_else(|| io::Error::other("invalid journal id"))?;
        let cost = row["accounted_usd"]
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .ok_or_else(|| io::Error::other("invalid journal cost"))?;
        requests.insert(id.to_owned(), cost);
    }
    Ok(requests.values().sum())
}

fn append(path: &Path, value: &Value) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()
}

struct JournalLock(PathBuf);

impl JournalLock {
    fn acquire(journal: &Path) -> io::Result<Self> {
        let path = journal.with_extension("lock");
        let started = Instant::now();
        loop {
            match File::options().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self(path)),
                Err(error)
                    if error.kind() == io::ErrorKind::AlreadyExists
                        && started.elapsed() < Duration::from_secs(5) =>
                {
                    // Another worker may be syncing a reservation. Never remove its lock;
                    // bounded waiting also fails closed after an interrupted lock owner.
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for JournalLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_core::{OneOrMany, completion::AssistantContent};

    #[test]
    fn concurrent_writer_waits_for_the_current_reservation_to_sync() {
        let temp = tempfile::tempdir().unwrap();
        let journal = temp.path().join("requests.jsonl");
        let lock = JournalLock::acquire(&journal).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let _next = JournalLock::acquire(&journal).unwrap();
                tx.send(()).unwrap();
            });
            assert!(rx.recv_timeout(Duration::from_millis(40)).is_err());
            drop(lock);
            rx.recv_timeout(Duration::from_secs(1)).unwrap();
        });
    }

    #[test]
    fn malformed_paid_output_preserves_usage_and_records_parse_failure() {
        let temp = tempfile::tempdir().unwrap();
        let meter = Metering {
            journal: temp.path().join("requests.jsonl"),
            run: "malformed-output".into(),
            paid: true,
            ceiling: 1.0,
        };
        let response = CompletionResponse {
            choice: OneOrMany::one(AssistantContent::text("not JSON")),
            usage: Usage {
                input_tokens: 1000,
                output_tokens: 100,
                ..Usage::default()
            },
            raw_response: json!({"id":"test-response", "status":"completed"}),
            message_id: None,
        };
        meter
            .start()
            .unwrap()
            .finish(Some(&response), Some(false))
            .unwrap();
        let text = fs::read_to_string(&meter.journal).unwrap();
        let settled: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        assert_eq!(settled["parse_succeeded"], false);
        assert_eq!(settled["transport_ok"], true);
        assert_eq!(settled["usage"]["input_tokens"], 1000);
        assert!((accounted_cost(&meter.journal).unwrap() - 0.00032).abs() < 1e-9);
    }

    #[test]
    fn interrupted_request_keeps_reservation_and_budget_blocks_dispatch() {
        let temp = tempfile::tempdir().unwrap();
        let meter = Metering {
            journal: temp.path().join("requests.jsonl"),
            run: "test".into(),
            paid: true,
            ceiling: 1.0,
        };
        {
            let _measurement = meter.start().unwrap();
        }
        assert_eq!(accounted_cost(&meter.journal).unwrap(), 1.0);
        assert!(meter.start().is_err());
    }

    #[test]
    fn settling_usage_releases_only_the_known_unused_reservation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("requests.jsonl");
        append(&path, &json!({"id":"a", "accounted_usd":1.0})).unwrap();
        append(&path, &json!({"id":"b", "accounted_usd":1.0})).unwrap();
        append(&path, &json!({"id":"a", "accounted_usd":0.01})).unwrap();
        assert_eq!(accounted_cost(&path).unwrap(), 1.01);
        fs::write(&path, "broken\n").unwrap();
        assert!(accounted_cost(&path).is_err());
    }

    #[test]
    fn pricing_counts_cached_tokens_once_and_uses_long_context_rates() {
        let mut usage = Usage {
            input_tokens: 100_000,
            cached_input_tokens: 20_000,
            cache_creation_input_tokens: 10_000,
            output_tokens: 10_000,
            ..Usage::default()
        };
        assert!((luna_cost(&usage) - 0.0289).abs() < 1e-9);
        usage.input_tokens = 300_000;
        assert!((luna_cost(&usage) - 0.1318).abs() < 1e-9);
    }
}

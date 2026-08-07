//! Raw ACP traffic log — per-conversation debug artifact (ticket E2-11-4).
//!
//! Port of the reference `session/raw-log.ts`: an append-only,
//! fixture-compatible log of raw ACP traffic recorded at the runtime
//! boundary BEFORE the reducer normalizes it. Exported as pretty JSON via
//! [`RawAcpLog::export_json`].

use serde::Serialize;

/// Log metadata carried into every export.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawAcpLogMeta {
    pub conversation_id: String,
    pub provider_id: String,
    pub acp_session_id: String,
    pub created_at: String,
}

/// One raw ACP event observed by the session.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RawAcpEvent {
    SessionUpdate {
        session_id: String,
        update: serde_json::Value,
    },
    Prompt {
        session_id: String,
        content: serde_json::Value,
    },
    PromptResult {
        session_id: String,
        stop_reason: Option<String>,
    },
    PermissionRequest {
        session_id: String,
        request: serde_json::Value,
    },
    PermissionResolved {
        session_id: String,
        request_id: String,
        option_id: String,
    },
}

/// One log entry with a sequence number and timestamp.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawAcpLogEntry {
    pub seq: u64,
    pub ts: u64,
    pub event: RawAcpEvent,
}

/// Export shape: meta (with generation time) + every retained entry.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawAcpLogExport {
    pub meta: RawAcpLogExportMeta,
    pub events: Vec<RawAcpLogEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawAcpLogExportMeta {
    pub conversation_id: String,
    pub provider_id: String,
    pub acp_session_id: String,
    pub created_at: String,
    pub generated_at: String,
}

const DEFAULT_MAX_ENTRIES: usize = 50_000;

/// Append-only log of raw ACP traffic (reference `RawAcpLog`).
pub struct RawAcpLog {
    meta: RawAcpLogMeta,
    entries: Vec<RawAcpLogEntry>,
    seq: u64,
    max_entries: usize,
}

impl RawAcpLog {
    pub fn new(meta: RawAcpLogMeta) -> Self {
        Self::with_max_entries(meta, DEFAULT_MAX_ENTRIES)
    }

    pub fn with_max_entries(meta: RawAcpLogMeta, max_entries: usize) -> Self {
        Self {
            meta,
            entries: Vec::new(),
            seq: 0,
            max_entries,
        }
    }

    /// The session id may change after `session/load` fallback → new.
    pub fn set_acp_session_id(&mut self, session_id: &str) {
        self.meta.acp_session_id = session_id.to_string();
    }

    /// Record one event with a monotonic seq and the current epoch-ms.
    /// Oldest entries drop once `max_entries` is exceeded.
    pub fn record(&mut self, event: RawAcpEvent) {
        self.record_at(event, now_ms());
    }

    /// `record` with an explicit timestamp (tests).
    pub fn record_at(&mut self, event: RawAcpEvent, ts: u64) {
        self.entries.push(RawAcpLogEntry {
            seq: self.seq,
            ts,
            event,
        });
        self.seq += 1;
        if self.entries.len() > self.max_entries {
            self.entries.remove(0);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn snapshot(&self) -> RawAcpLogExport {
        RawAcpLogExport {
            meta: RawAcpLogExportMeta {
                conversation_id: self.meta.conversation_id.clone(),
                provider_id: self.meta.provider_id.clone(),
                acp_session_id: self.meta.acp_session_id.clone(),
                created_at: self.meta.created_at.clone(),
                generated_at: iso_now(),
            },
            events: self.entries.clone(),
        }
    }

    /// Pretty-printed JSON export (reference `exportJson`).
    pub fn export_json(&self) -> String {
        serde_json::to_string_pretty(&self.snapshot()).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "RawAcpLog: export serialization failed");
            "{}".to_string()
        })
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// ISO-8601 UTC timestamp without pulling chrono (second precision).
pub fn iso_now() -> String {
    let secs = now_ms() / 1000;
    // Civil-from-days arithmetic (Howard Hinnant's algorithm).
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as i64;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if mo <= 2 { y + 1 } else { y };
    format!("{year:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> RawAcpLogMeta {
        RawAcpLogMeta {
            conversation_id: "conv-1".into(),
            provider_id: "claude".into(),
            acp_session_id: "sess-1".into(),
            created_at: "2026-08-05T00:00:00Z".into(),
        }
    }

    #[test]
    fn records_and_exports_json() {
        let mut log = RawAcpLog::new(meta());
        log.record_at(
            RawAcpEvent::Prompt {
                session_id: "sess-1".into(),
                content: serde_json::json!([{ "type": "text", "text": "hi" }]),
            },
            1000,
        );
        log.record_at(
            RawAcpEvent::SessionUpdate {
                session_id: "sess-1".into(),
                update: serde_json::json!({ "sessionUpdate": "agent_message_chunk" }),
            },
            1001,
        );
        assert_eq!(log.len(), 2);

        let export: serde_json::Value = serde_json::from_str(&log.export_json()).unwrap();
        assert_eq!(export["meta"]["conversationId"], "conv-1");
        assert_eq!(export["events"][0]["seq"], 0);
        assert_eq!(export["events"][0]["event"]["kind"], "prompt");
        assert_eq!(export["events"][1]["seq"], 1);
        assert!(export["meta"]["generatedAt"].is_string());
    }

    #[test]
    fn evicts_oldest_beyond_the_cap() {
        let mut log = RawAcpLog::with_max_entries(meta(), 2);
        for i in 0..5 {
            log.record_at(
                RawAcpEvent::PromptResult {
                    session_id: "sess-1".into(),
                    stop_reason: Some(format!("r{i}")),
                },
                i,
            );
        }
        assert_eq!(log.len(), 2);
        let snapshot = log.snapshot();
        // The two newest survive, with their original seqs.
        assert_eq!(snapshot.events[0].seq, 3);
        assert_eq!(snapshot.events[1].seq, 4);
    }

    #[test]
    fn iso_now_shapes_like_iso8601() {
        let stamp = iso_now();
        assert_eq!(stamp.len(), 20);
        assert!(stamp.ends_with('Z'));
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[10..11], "T");
    }
}

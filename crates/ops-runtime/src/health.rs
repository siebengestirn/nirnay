//! Operational health metrics and the gate they impose on new risk.
//!
//! Monitoring is not decorative: when a safety condition fails, new exposure
//! must be denied. This module produces the verdict; the runtime wires it to
//! the risk authority at the integration stage.

/// Thresholds for the paper/shadow runtime. Every field is mandatory.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RuntimeThresholds {
    pub max_feed_age_ns: u64,
    pub max_journal_append_latency_ns: u64,
    pub max_journal_queue_depth: u64,
    pub min_disk_available_bytes: u64,
    pub max_unknown_orders: u32,
    pub max_reconciliation_differences: u32,
    pub max_sequence_gaps: u32,
}

/// A point-in-time operational snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthSnapshot {
    pub feed_age_ns: u64,
    pub sequence_gaps: u32,
    pub reconnect_count: u32,
    pub journal_append_latency_ns: u64,
    pub journal_queue_depth: u64,
    pub disk_available_bytes: u64,
    pub open_orders: u32,
    pub unknown_orders: u32,
    pub reconciliation_differences: u32,
    pub kill_switch_engaged: bool,
    pub account_snapshot_available: bool,
}

/// Verdict on whether new risk-increasing decisions may proceed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthVerdict {
    pub safe_for_new_risk: bool,
    pub reasons: Vec<String>,
}

impl HealthSnapshot {
    /// Evaluate the snapshot against the operating thresholds.
    pub fn evaluate(&self, thresholds: &RuntimeThresholds) -> HealthVerdict {
        let mut reasons = Vec::new();
        if self.feed_age_ns > thresholds.max_feed_age_ns {
            reasons.push("feed is stale".to_owned());
        }
        if self.sequence_gaps > thresholds.max_sequence_gaps {
            reasons.push("sequence gaps are unresolved".to_owned());
        }
        if self.journal_append_latency_ns > thresholds.max_journal_append_latency_ns {
            reasons.push("journal append latency exceeded".to_owned());
        }
        if self.journal_queue_depth > thresholds.max_journal_queue_depth {
            reasons.push("journal queue backlog exceeded".to_owned());
        }
        if self.disk_available_bytes < thresholds.min_disk_available_bytes {
            reasons.push("disk capacity below minimum".to_owned());
        }
        if self.unknown_orders > thresholds.max_unknown_orders {
            reasons.push("UNKNOWN orders exceed the limit".to_owned());
        }
        if self.reconciliation_differences > thresholds.max_reconciliation_differences {
            reasons.push("account reconciliation differs".to_owned());
        }
        if self.kill_switch_engaged {
            reasons.push("kill switch engaged".to_owned());
        }
        if !self.account_snapshot_available {
            reasons.push("account snapshot unavailable".to_owned());
        }
        HealthVerdict {
            safe_for_new_risk: reasons.is_empty(),
            reasons,
        }
    }

    /// Prometheus-style metric text. No external metrics dependency.
    #[must_use]
    pub fn metrics_text(&self) -> String {
        format!(
            "nirnay_feed_age_ns {}\n\
             nirnay_sequence_gaps {}\n\
             nirnay_reconnect_count {}\n\
             nirnay_journal_append_latency_ns {}\n\
             nirnay_journal_queue_depth {}\n\
             nirnay_disk_available_bytes {}\n\
             nirnay_open_orders {}\n\
             nirnay_unknown_orders {}\n\
             nirnay_reconciliation_differences {}\n\
             nirnay_kill_switch_engaged {}\n\
             nirnay_account_snapshot_available {}\n",
            self.feed_age_ns,
            self.sequence_gaps,
            self.reconnect_count,
            self.journal_append_latency_ns,
            self.journal_queue_depth,
            self.disk_available_bytes,
            self.open_orders,
            self.unknown_orders,
            self.reconciliation_differences,
            u8::from(self.kill_switch_engaged),
            u8::from(self.account_snapshot_available),
        )
    }
}

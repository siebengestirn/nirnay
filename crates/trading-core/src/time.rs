//! Event and trading time.
//!
//! Nirnay encoded trade dates as `YYYYMMDD` integers. The canonical platform
//! date is `jiff::civil::Date`; the port keeps Nirnay's separation of source,
//! receive, availability and record clocks.

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use crate::DomainError;

/// Calendar trading date. This is the canonical `jiff::civil::Date`; the
/// exchange calendar decides whether it is a session.
pub type TradeDate = Date;

/// UTC wall-clock instant in nanoseconds. This is a timestamp, not a date.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UtcNanos(i64);

impl UtcNanos {
    #[must_use]
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// Run-local monotonic time used for availability and expiry checks. It is
/// not convertible to or from a UTC timestamp.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LogicalNanos(u64);

impl LogicalNanos {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn elapsed_since(self, earlier: Self) -> Result<u64, DomainError> {
        self.0.checked_sub(earlier.0).ok_or_else(|| {
            DomainError::InvalidEventTime("logical clock moved backwards".to_owned())
        })
    }
}

/// Four distinct times carried by every event.
///
/// `source_time_utc` is the venue/source clock and is only meaningful when the
/// envelope also carries a source identity and clock domain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventTimes {
    /// Timestamp from the venue/source clock. The source clock domain is
    /// recorded separately on the envelope.
    pub source_time_utc: Option<UtcNanos>,
    /// UTC wall-clock time when the adapter received the message.
    pub received_time_utc: UtcNanos,
    /// Run-local monotonic time when the event became visible to strategy code.
    pub available_time: LogicalNanos,
    /// UTC wall-clock time when the event envelope was recorded.
    pub recorded_time_utc: UtcNanos,
}

#[cfg(test)]
mod tests {
    use super::{LogicalNanos, TradeDate};

    #[test]
    fn logical_clock_rejects_regression() {
        let later = LogicalNanos::new(10);
        let earlier = LogicalNanos::new(3);
        assert_eq!(later.elapsed_since(earlier).unwrap_or(0), 7);
        assert!(earlier.elapsed_since(later).is_err());
    }

    #[test]
    fn trade_date_is_a_calendar_date() {
        let date: TradeDate = TradeDate::new(2026, 10, 28).unwrap_or_default();
        assert_eq!(date.to_string(), "2026-10-28");
    }
}

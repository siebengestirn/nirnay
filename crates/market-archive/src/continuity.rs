use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation {
    Initial,
    InOrder,
    Gap { missing: u64 },
    Duplicate,
    OutOfOrder { highest_seen: u64 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamQuality {
    pub observed: u64,
    pub first_sequence: Option<u64>,
    pub highest_sequence: Option<u64>,
    pub gap_events: u64,
    pub missing_total: u64,
    pub duplicates: u64,
    pub out_of_order: u64,
}

impl StreamQuality {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.gap_events == 0 && self.duplicates == 0 && self.out_of_order == 0
    }

    /// Validate the counters before they are accepted into a sealed manifest.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        let non_in_order = self
            .gap_events
            .saturating_add(self.duplicates)
            .saturating_add(self.out_of_order);
        let counters_fit = non_in_order <= self.observed.saturating_sub(1);
        let gap_fields_agree = (self.gap_events == 0) == (self.missing_total == 0);
        if self.observed == 0 {
            counters_fit
                && gap_fields_agree
                && self.first_sequence.is_none()
                && self.highest_sequence.is_none()
                && self.duplicates == 0
                && self.out_of_order == 0
        } else {
            counters_fit
                && gap_fields_agree
                && matches!(
                    (self.first_sequence, self.highest_sequence),
                    (Some(first), Some(highest)) if first <= highest
                )
        }
    }
}

/// Arrival-order sequence tracker for one provider/instrument stream.
/// Sequence semantics must be confirmed for the adapter before enabling it.
#[derive(Clone, Debug, Default)]
pub struct ContinuityTracker {
    quality: StreamQuality,
}

impl ContinuityTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe(&mut self, sequence: u64) -> Observation {
        let quality = &mut self.quality;
        quality.observed = quality.observed.saturating_add(1);

        let Some(highest) = quality.highest_sequence else {
            quality.first_sequence = Some(sequence);
            quality.highest_sequence = Some(sequence);
            return Observation::Initial;
        };

        if sequence == highest {
            quality.duplicates = quality.duplicates.saturating_add(1);
            return Observation::Duplicate;
        }
        if sequence < highest {
            quality.out_of_order = quality.out_of_order.saturating_add(1);
            return Observation::OutOfOrder {
                highest_seen: highest,
            };
        }

        quality.highest_sequence = Some(sequence);
        let missing = sequence - highest - 1;
        if missing == 0 {
            Observation::InOrder
        } else {
            quality.gap_events = quality.gap_events.saturating_add(1);
            quality.missing_total = quality.missing_total.saturating_add(missing);
            Observation::Gap { missing }
        }
    }

    #[must_use]
    pub fn quality(&self) -> &StreamQuality {
        &self.quality
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_and_gap_streams_are_distinguished() {
        let mut tracker = ContinuityTracker::new();
        assert_eq!(tracker.observe(10), Observation::Initial);
        assert_eq!(tracker.observe(11), Observation::InOrder);
        assert_eq!(tracker.observe(15), Observation::Gap { missing: 3 });
        assert_eq!(tracker.quality().missing_total, 3);
        assert!(!tracker.quality().is_clean());
    }

    #[test]
    fn duplicate_and_out_of_order_are_not_hidden() {
        let mut tracker = ContinuityTracker::new();
        tracker.observe(1);
        tracker.observe(2);
        assert_eq!(tracker.observe(2), Observation::Duplicate);
        assert_eq!(
            tracker.observe(1),
            Observation::OutOfOrder { highest_seen: 2 }
        );
        assert_eq!(tracker.quality().duplicates, 1);
        assert_eq!(tracker.quality().out_of_order, 1);
    }
}

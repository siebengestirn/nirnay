//! Bitemporal provider-to-instrument mapping history.
//!
//! Vendor symbols/tokens are not canonical instrument identities. This
//! history answers a mapping query on both valid time and system knowledge time.

use jiff::{Timestamp, civil::Date};
use serde::{Deserialize, Serialize};

use crate::{InstrumentId, Segment, SourceProvenance, Venue};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderListingMapping {
    pub mapping_id: String,
    pub provider: String,
    pub provider_instrument_id: String,
    pub instrument_id: InstrumentId,
    pub venue: Venue,
    pub segment: Segment,
    pub venue_symbol: String,
    pub valid_from: Date,
    pub valid_to: Option<Date>,
    pub recorded_at: Timestamp,
    pub provenance: SourceProvenance,
}

impl ProviderListingMapping {
    pub fn validate(&self) -> Result<(), anyhow::Error> {
        anyhow::ensure!(!self.mapping_id.trim().is_empty(), "mapping_id is required");
        anyhow::ensure!(!self.provider.trim().is_empty(), "provider is required");
        anyhow::ensure!(
            !self.provider_instrument_id.trim().is_empty(),
            "provider instrument id is required"
        );
        anyhow::ensure!(
            !self.venue_symbol.trim().is_empty(),
            "venue symbol is required"
        );
        anyhow::ensure!(
            self.valid_to.is_none_or(|end| end >= self.valid_from),
            "mapping validity interval is inverted"
        );
        self.provenance.validate()?;
        anyhow::ensure!(
            self.recorded_at >= self.provenance.retrieved_at,
            "mapping knowledge time predates source retrieval"
        );
        anyhow::ensure!(
            self.provenance.venue == self.venue && self.provenance.segment == self.segment,
            "mapping source venue/segment does not match the mapping contract"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProviderListingHistory {
    mappings: Vec<ProviderListingMapping>,
}

impl ProviderListingHistory {
    pub fn append(&mut self, mapping: ProviderListingMapping) -> Result<(), anyhow::Error> {
        mapping.validate()?;
        anyhow::ensure!(
            !self
                .mappings
                .iter()
                .any(|old| old.mapping_id == mapping.mapping_id),
            "duplicate provider listing mapping id {}",
            mapping.mapping_id
        );
        self.mappings.push(mapping);
        self.mappings.sort_by(|a, b| {
            (
                a.provider.as_str(),
                a.provider_instrument_id.as_str(),
                a.valid_from,
                a.recorded_at,
            )
                .cmp(&(
                    b.provider.as_str(),
                    b.provider_instrument_id.as_str(),
                    b.valid_from,
                    b.recorded_at,
                ))
        });
        Ok(())
    }

    #[must_use]
    pub fn mappings(&self) -> &[ProviderListingMapping] {
        &self.mappings
    }

    pub fn resolve(
        &self,
        provider: &str,
        provider_instrument_id: &str,
        venue: Venue,
        valid_on: Date,
        known_at: Timestamp,
    ) -> Result<&ProviderListingMapping, anyhow::Error> {
        let mut candidates = self.mappings.iter().filter(|mapping| {
            mapping.provider == provider
                && mapping.provider_instrument_id == provider_instrument_id
                && mapping.venue == venue
                && mapping.valid_from <= valid_on
                && mapping.valid_to.is_none_or(|end| valid_on <= end)
                && mapping.recorded_at <= known_at
        });
        let first = candidates.next().ok_or_else(|| anyhow::anyhow!(
            "no point-in-time mapping for {provider}:{provider_instrument_id} on {valid_on} known at {known_at}"))?;
        anyhow::ensure!(
            candidates.next().is_none(),
            "ambiguous point-in-time mapping for {provider}:{provider_instrument_id} on {valid_on}"
        );
        Ok(first)
    }
}

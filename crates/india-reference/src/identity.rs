//! Internal instrument identity resolution.
//!
//! An `InstrumentId` is an opaque platform identity. ISIN, ticker, scrip code
//! and provider identity are aliases used for matching. No identity is ever
//! derived from ISIN or symbol text.

use std::collections::{
    BTreeMap, //
    BTreeSet,
};

use thiserror::Error;
use trading_core::{
    InstrumentId, //
    Isin,
    Venue,
};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("symbol {venue:?}:{symbol} is already assigned to another instrument")]
    SymbolCollision { venue: Venue, symbol: String },
    #[error("ISIN {isin} is already assigned to another instrument")]
    IsinCollision { isin: String },
    #[error("instrument identity prefix is empty")]
    EmptyPrefix,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdentityRegistry {
    next: u64,
    prefix: String,
    isin_owners: BTreeMap<String, InstrumentId>,
    symbol_owners: BTreeMap<(Venue, String), InstrumentId>,
    aliases: BTreeMap<InstrumentId, BTreeSet<String>>,
}

impl IdentityRegistry {
    /// Create a registry with an operator-owned opaque ID prefix.
    pub fn new(prefix: impl Into<String>) -> Result<Self, IdentityError> {
        let prefix = prefix.into();
        if prefix.trim().is_empty() {
            return Err(IdentityError::EmptyPrefix);
        }
        Ok(Self {
            prefix,
            ..Self::default()
        })
    }

    fn allocate(&mut self) -> Result<InstrumentId, IdentityError> {
        loop {
            let sequence = self.next;
            self.next = self.next.checked_add(1).ok_or(IdentityError::EmptyPrefix)?;
            let text = format!("{}{sequence:08x}", self.prefix);
            if let Ok(id) = InstrumentId::new(&text) {
                self.aliases.entry(id.clone()).or_default();
                return Ok(id);
            }
        }
    }

    /// Resolve an ISIN and venue symbol to one stable platform identity.
    ///
    /// Conflicting aliases fail closed. A new symbol for a known ISIN extends
    /// that instrument's aliases; a known symbol with a new ISIN is rejected.
    pub fn resolve(
        &mut self,
        isin: &Isin,
        venue: Venue,
        symbol: &str,
    ) -> Result<InstrumentId, IdentityError> {
        let isin_key = isin.as_str().to_owned();
        let symbol_key = (venue, symbol.to_owned());
        match (
            self.isin_owners.get(&isin_key).cloned(),
            self.symbol_owners.get(&symbol_key).cloned(),
        ) {
            (Some(by_isin), Some(by_symbol)) => {
                if by_isin != by_symbol {
                    return Err(IdentityError::SymbolCollision {
                        venue,
                        symbol: symbol.to_owned(),
                    });
                }
                Ok(by_isin)
            }
            (Some(id), None) => {
                self.symbol_owners.insert(symbol_key.clone(), id.clone());
                self.aliases
                    .entry(id.clone())
                    .or_default()
                    .insert(format!("{venue:?}:{symbol}"));
                Ok(id)
            }
            (None, Some(id)) => {
                let owners_isin = self
                    .aliases
                    .get(&id)
                    .and_then(|aliases| aliases.iter().find(|alias| alias.starts_with("ISIN:")))
                    .map(|alias| alias.strip_prefix("ISIN:").unwrap_or_default())
                    .unwrap_or_default();
                if !owners_isin.is_empty() && owners_isin != isin_key {
                    return Err(IdentityError::IsinCollision { isin: isin_key });
                }
                self.isin_owners.insert(isin_key.clone(), id.clone());
                self.aliases
                    .entry(id.clone())
                    .or_default()
                    .insert(format!("ISIN:{isin_key}"));
                Ok(id)
            }
            (None, None) => {
                let id = self.allocate()?;
                self.isin_owners.insert(isin_key.clone(), id.clone());
                self.symbol_owners.insert(symbol_key.clone(), id.clone());
                let aliases = self.aliases.entry(id.clone()).or_default();
                aliases.insert(format!("ISIN:{isin_key}"));
                aliases.insert(format!("{venue:?}:{symbol}"));
                Ok(id)
            }
        }
    }

    /// Number of allocated identities.
    #[must_use]
    pub fn len(&self) -> usize {
        self.aliases.len()
    }

    /// Whether no identity has been allocated.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.aliases.is_empty()
    }
}

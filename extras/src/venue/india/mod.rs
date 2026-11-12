//! NSE/BSE reference-master ingestion (capability `INDIA-REF-INGEST`, task
//! P1-03).
//!
//! Ingestion is the bridge between raw exchange source files and the canonical
//! India domain in `india-domain` (`crates/india-domain`). The domain owns identity,
//! listings and validation; this module owns acquisition provenance, venue
//! parsers, identity resolution, reconciliation, bitemporal snapshots,
//! calendars, tick schedules, corporate-action events and the raw-source
//! archive.
//!
//! # Invariants
//!
//! * Raw source bytes are never rewritten by ingestion. Every source is
//!   retained write-once in the [`archive`] and described by
//!   [`provenance`: source id, venue, segment, retrieval time, SHA-256 and
//!   source format version.
//! * NSE and BSE masters have separate parsers ([`nse`], [`bse`]). A parser
//!   rejects malformed rows; it never invents instruments or silently drops a
//!   record it cannot classify.
//! * `SERIES` semantics come from the exchange: `BE` is a settlement
//!   regime (trade-for-trade), not a suspension (see [`nse`]).
//! * Instrument identity ([`india_domain::InstrumentId`]) is preserved across
//!   symbol changes. Resolution matches on ISIN first and never merges two
//!   instruments onto one freshly-seen symbol (see [`resolution`]).
//! * Listing state changes close and open effective intervals; each open and
//!   close is appended as a [`resolution::ListingStateRecord`] with its own
//!   knowledge time. History is never overwritten in place (see
//!   [`resolution`], [`store`]).
//! * Tick sizes are exchange-assigned data, ingested per revision date (see
//!   [`tick_schedule`]) — never a hard-coded constant. NSE assigns them
//!   monthly per instrument from the previous month's last available close;
//!   the assignment record carries the reference close, its dataset and
//!   hash, the schedule revision and hash, and the calendar revision and
//!   hash. The close must resolve to a retained archive object recorded
//!   under that dataset and be the last trading session before the month;
//!   the holiday file itself must be retained in the archive under the
//!   calendar dataset, so the rule is reproducible rather than merely
//!   well-formed. BSE publishes the tick in decimal rupees (`FLOAT[3,2]`)
//!   and it is converted to exact paise by integer arithmetic.
//! * Settlement eligibility comes from the exchange group, not an
//!   assumption: BSE trade-to-trade groups (`T`, `MT`, `TS`, `IT`, `R`,
//!   `P`, `Y`, `XT` and the `Z`/`ZP`/`ZY` variants) carry no netting and are
//!   delivery-only. An unclassified group fails ingestion.
//! * A [`store::ReferenceStore`] keeps every published revision immutably;
//!   [`store::ReferenceSnapshot::reference`] answers `Reference(t_valid,
//!   t_recorded)` across both time axes.
//! * No record claims knowledge earlier than its source retrieval: any
//!   bitemporal record with `recorded_at < retrieved_at` is rejected (see
//!   [`corporate_actions`], [`resolution::reconcile`]).
//!
//! This code is retained as reference material; it is not an active capability
//! claim. Its scope is
//! NSE CM cash and BSE equity masters, including the BSE tick/settlement and
//! venue-eligibility corrections. F&O masters are a separate capability
//! (`INDIA-FNO-INGEST`, proposed), and durable on-disk snapshot storage is
//! P1-04.

pub mod archive;
pub mod backup;
pub mod durable;

pub mod bse;
pub mod calendar;
pub mod corporate_actions;
mod csv;
pub mod eod_close;
pub mod nse;
pub mod provenance;
pub mod resolution;
pub mod store;
pub mod tick_schedule;

pub use archive::{ArchiveBusy, ArchiveVerification, ArchivedSource, SourceArchive};
pub use backup::{
    BACKUP_FORMAT_VERSION, BackupManifest, BackupResult, RestoreVerification,
    backup as backup_store, restore as restore_backup,
};
pub use bse::{
    BseCashRow, BseGroupClass, BseSecurityType, classify_bse_group, parse_bse_master,
    parse_rupees_to_paise,
};
pub use calendar::{
    BuiltCalendar, build_calendar, classify_purpose, is_trading_weekday, parse_holiday_csv,
    parse_nse_holiday_csv,
};
pub use corporate_actions::{CorporateActionLedger, CorporateActionRecord};
pub use durable::{
    DurableReferenceStore, PAYLOAD_SCHEMA_VERSION, PayloadEnvelope, PublishOutcome,
    STORE_SCHEMA_VERSION,
};
pub use eod_close::{EodCloseRow, parse_eod_close_csv};
pub use nse::{NseCashRow, parse_nse_cm_master};
pub use provenance::{SourceProvenance, provenance_for_source};
pub use resolution::{
    InstrumentChange, ListingStateRecord, NormalizedListing, ReconciledUniverse, Reconciliation,
    bse_carry, nse_carry, reconcile,
};
pub use store::{ReferenceSnapshot, ReferenceStore, latest_states_at, snapshot_hash_of};
pub use tick_schedule::{
    ExchangeTickSchedule, ReferenceClose, TickScheduleBook, nse_tick_schedule_2025_04_15,
    parse_tick_schedule_csv,
};

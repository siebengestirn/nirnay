//! Cross-source book comparison without merging liquidity.
//!
//! Upstox and FYERS are independent data products. Their displayed quantities
//! are never summed and their books are never combined into a synthetic
//! consolidated book. This module only reports differences.

use serde::{
    Deserialize, //
    Serialize,
};

use crate::mbp::{
    BookPhase, //
    MbpBook,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBookView {
    pub source_id: String,
    pub best_bid_paise: Option<i64>,
    pub best_ask_paise: Option<i64>,
    pub bid_levels: usize,
    pub ask_levels: usize,
    pub phase: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookComparison {
    pub left: SourceBookView,
    pub right: SourceBookView,
    pub differences: Vec<String>,
    pub matched: bool,
}

fn view(book: &MbpBook) -> SourceBookView {
    let snapshot = book.to_snapshot();
    SourceBookView {
        source_id: book.source_id().to_owned(),
        best_bid_paise: book.best_bid().map(|level| level.price.raw()),
        best_ask_paise: book.best_ask().map(|level| level.price.raw()),
        bid_levels: snapshot.bids.len(),
        ask_levels: snapshot.asks.len(),
        phase: format!("{:?}", book.phase()),
    }
}

/// Compare two per-source books. No merged book is produced; a mismatch is an
/// explicit incident signal, not something silently corrected.
pub fn compare_books(left: &MbpBook, right: &MbpBook) -> BookComparison {
    let left_view = view(left);
    let right_view = view(right);
    let mut differences = Vec::new();

    if left_view.source_id == right_view.source_id {
        differences.push("both books declare the same source".to_owned());
    }
    if left_view.best_bid_paise != right_view.best_bid_paise {
        differences.push("best bid differs".to_owned());
    }
    if left_view.best_ask_paise != right_view.best_ask_paise {
        differences.push("best ask differs".to_owned());
    }
    if left_view.bid_levels != right_view.bid_levels {
        differences.push("bid depth differs".to_owned());
    }
    if left_view.ask_levels != right_view.ask_levels {
        differences.push("ask depth differs".to_owned());
    }
    if left_view.phase != right_view.phase {
        differences.push("book quality state differs".to_owned());
    }

    let matched = differences.is_empty();
    BookComparison {
        left: left_view,
        right: right_view,
        differences,
        matched,
    }
}

#[must_use]
pub fn is_live(book: &MbpBook) -> bool {
    book.phase() == BookPhase::LiveUpdates
}

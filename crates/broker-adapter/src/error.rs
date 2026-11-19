//! Adapter failures. A transport error never authorizes a retry by itself.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AdapterError {
    #[error("adapter is not authenticated")]
    NotAuthenticated,
    #[error("request does not bind the complete typed order terms")]
    IncompleteRequest,
    #[error("broker order identity was not returned")]
    MissingBrokerOrderId,
    #[error("unknown broker order `{0}`")]
    UnknownBrokerOrder(String),
    #[error("simulator configuration is invalid: {0}")]
    InvalidScenario(String),
    #[error("transport failure on `{operation}`")]
    Transport { operation: String },
}

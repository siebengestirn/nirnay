class KiteOpsError(RuntimeError):
    """An operator-facing Kite API or control error."""


class KiteRejected(KiteOpsError):
    """Kite returned an explicit error response for a request."""


class KiteUnknownOutcome(KiteOpsError):
    """A write may have reached Kite; reconciliation is required."""


class RiskRejected(KiteOpsError):
    """A manual order failed a local pre-trade control."""

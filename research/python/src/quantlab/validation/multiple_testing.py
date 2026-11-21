"""Package-local re-export of the flat multiple-comparison helper.

No behavior is duplicated: the tested implementation remains in
``quantlab.multiple_testing``.
"""
from __future__ import annotations

from ..multiple_testing import holm_adjust

__all__ = ["holm_adjust"]

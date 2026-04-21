"""StarBreaker runtime package.

Phase 7 migration in progress. This package is currently a thin shim over
``_legacy.py`` (the original monolithic ``runtime.py``). Subsequent sub-phases
will move logic into topically-named modules while this ``__init__`` continues
to re-export the full public surface for backward compatibility.
"""

from __future__ import annotations

from . import _legacy


def __getattr__(name: str):
    # Delegates any attribute access (public and private) to the legacy module
    # so that both ``from .runtime import PROP_PALETTE_ID`` and
    # ``from .runtime import _float_public_param`` keep working while the
    # split is in flight.
    try:
        return getattr(_legacy, name)
    except AttributeError as exc:
        raise AttributeError(
            f"module 'starbreaker_addon.runtime' has no attribute {name!r}"
        ) from exc


def __dir__() -> list[str]:
    return sorted(set(list(globals().keys()) + dir(_legacy)))

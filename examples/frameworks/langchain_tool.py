"""LangChain integration example for skrun executable skills.

This file is intentionally dependency-light: it exposes the callable shape that
can be passed to a LangChain tool wrapper without requiring LangChain during
skrun's own tests.
"""

from __future__ import annotations

from typing import Any

import skrun


def regex_finder_tool(arguments: dict[str, Any]) -> dict[str, Any]:
    """Call an installed skrun skill from a LangChain tool body."""

    return skrun.skill("regex-finder").call(arguments)


__all__ = ["regex_finder_tool"]

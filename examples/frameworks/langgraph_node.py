"""LangGraph node integration example for skrun executable skills."""

from __future__ import annotations

from typing import Any

import skrun


def regex_finder_node(state: dict[str, Any]) -> dict[str, Any]:
    """Call a skrun skill and return a LangGraph-compatible state patch."""

    result = skrun.skill("regex-finder").call(
        {
            "pattern": state["pattern"],
            "path": state.get("path", "."),
        }
    )
    return {"regex_finder_result": result}


__all__ = ["regex_finder_node"]

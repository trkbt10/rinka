#!/usr/bin/env python3
"""Verify the activity panel through the AT-SPI accessibility API."""

from __future__ import annotations

import argparse
import json
import time
from pathlib import Path
from typing import Iterable

import pyatspi


def children(node: object) -> Iterable[object]:
    try:
        for index in range(node.childCount):
            child = node.getChildAtIndex(index)
            if child is not None:
                yield child
    except (AttributeError, LookupError, RuntimeError):
        return


def walk(node: object) -> Iterable[object]:
    yield node
    for child in children(node):
        yield from walk(child)


def name(node: object) -> str:
    try:
        return node.name or ""
    except (AttributeError, LookupError, RuntimeError):
        return ""


def role_name(node: object) -> str:
    try:
        return node.getRoleName()
    except (AttributeError, LookupError, RuntimeError):
        return "defunct"


def has_state(node: object, state: int) -> bool:
    try:
        return node.getState().contains(state)
    except (AttributeError, LookupError, RuntimeError):
        return False


def geometry(node: object) -> dict[str, int]:
    extents = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
    return {
        "x": extents.x,
        "y": extents.y,
        "width": extents.width,
        "height": extents.height,
    }


def find_window(title: str, timeout: float) -> object:
    deadline = time.monotonic() + timeout
    desktop = pyatspi.Registry.getDesktop(0)
    while time.monotonic() < deadline:
        for node in walk(desktop):
            if role_name(node) not in {"dialog", "frame", "panel", "window"}:
                continue
            if name(node) == title or any(name(child) == "Stop" for child in walk(node)):
                return node
        time.sleep(0.1)
    named = [
        {"role": role_name(node), "name": name(node)}
        for node in walk(desktop)
        if name(node)
    ]
    raise RuntimeError(f"AT-SPI window not found: {title}; named nodes={named}")


def required_node(window: object, accessible_name: str, role: str) -> object:
    for node in walk(window):
        if name(node) == accessible_name and role_name(node) == role:
            return node
    raise RuntimeError(f"AT-SPI node not found: {accessible_name!r} ({role})")


def node_snapshot(node: object) -> dict[str, object]:
    return {
        "name": name(node),
        "role": role_name(node),
        "showing": has_state(node, pyatspi.STATE_SHOWING),
        "visible": has_state(node, pyatspi.STATE_VISIBLE),
        "sensitive": has_state(node, pyatspi.STATE_SENSITIVE),
        "geometry": geometry(node),
    }


def assert_contained(
    window_geometry: dict[str, int],
    node_geometry: dict[str, int],
    label: str,
) -> None:
    if node_geometry["width"] <= 0 or node_geometry["height"] <= 0:
        raise RuntimeError(f"activity panel node has zero size: {label}={node_geometry}")
    window_right = window_geometry["x"] + window_geometry["width"]
    window_bottom = window_geometry["y"] + window_geometry["height"]
    node_right = node_geometry["x"] + node_geometry["width"]
    node_bottom = node_geometry["y"] + node_geometry["height"]
    if (
        node_geometry["x"] < window_geometry["x"]
        or node_geometry["y"] < window_geometry["y"]
        or node_right > window_right
        or node_bottom > window_bottom
    ):
        raise RuntimeError(
            "activity panel node extends beyond the window: "
            f"{label}={node_geometry} window={window_geometry}"
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--title", default="Connection Activity")
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()

    window = find_window(arguments.title, 15.0)
    required = {
        "heading": required_node(window, "Refreshing Remote Project", "label"),
        "progress": required_node(window, "Directory refresh 58 percent", "progress bar"),
        "detail": required_node(window, "Reading directory metadata", "label"),
        "stop": required_node(window, "Stop", "push button"),
    }
    snapshots = {key: node_snapshot(node) for key, node in required.items()}
    window_snapshot = node_snapshot(window)
    window_geometry = window_snapshot["geometry"]
    if window_geometry["width"] <= 0 or window_geometry["height"] <= 0:
        raise RuntimeError(f"activity panel window has zero size: {window_geometry}")
    for key, snapshot in snapshots.items():
        if not snapshot["showing"] or not snapshot["visible"]:
            raise RuntimeError(f"activity panel node is not visible: {key}={snapshot}")
        assert_contained(window_geometry, snapshot["geometry"], key)
    if not snapshots["stop"]["sensitive"]:
        raise RuntimeError(f"Stop action is not sensitive: {snapshots['stop']}")

    result = {
        "window": window_snapshot,
        "required": snapshots,
        "result": "PASS",
    }
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Inspect and exercise the running Explorer through the AT-SPI accessibility API."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

import pyatspi


@dataclass(frozen=True)
class Geometry:
    x: int
    y: int
    width: int
    height: int

    def as_dict(self) -> dict[str, int]:
        return {
            "x": self.x,
            "y": self.y,
            "width": self.width,
            "height": self.height,
        }


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


def role_name(node: object) -> str:
    try:
        return node.getRoleName()
    except (AttributeError, LookupError, RuntimeError):
        return "defunct"


def name(node: object) -> str:
    try:
        return node.name or ""
    except (AttributeError, LookupError, RuntimeError):
        return ""


def description(node: object) -> str:
    try:
        return node.description or ""
    except (AttributeError, LookupError, RuntimeError):
        return ""


def has_state(node: object, state: int) -> bool:
    try:
        return node.getState().contains(state)
    except (AttributeError, LookupError, RuntimeError):
        return False


def geometry(node: object) -> Geometry:
    extents = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
    return Geometry(extents.x, extents.y, extents.width, extents.height)


def find_window(title: str, timeout: float) -> object:
    deadline = time.monotonic() + timeout
    desktop = pyatspi.Registry.getDesktop(0)
    while time.monotonic() < deadline:
        for node in walk(desktop):
            if name(node) == title and role_name(node) in {"frame", "window"}:
                return node
        time.sleep(0.1)
    raise RuntimeError(f"AT-SPI window not found: {title}")


def find_button(window: object, button_name: str) -> object:
    for node in walk(window):
        if name(node) == button_name and role_name(node) in {
            "push button",
            "toggle button",
        }:
            return node
    raise RuntimeError(f"AT-SPI button not found: {button_name}")


def invoke(node: object) -> None:
    actions = node.queryAction()
    if actions.nActions < 1 or not actions.doAction(0):
        raise RuntimeError(f"AT-SPI action failed: {name(node)}")


def named_node_snapshot(window: object, accessible_name: str) -> list[dict[str, object]]:
    snapshots = []
    for node in walk(window):
        if name(node) != accessible_name:
            continue
        try:
            node_geometry = geometry(node).as_dict()
        except (AttributeError, LookupError, NotImplementedError, RuntimeError):
            node_geometry = None
        snapshots.append(
            {
                "role": role_name(node),
                "showing": has_state(node, pyatspi.STATE_SHOWING),
                "visible": has_state(node, pyatspi.STATE_VISIBLE),
                "geometry": node_geometry,
            }
        )
    return snapshots


def assert_fixed_frame(expected: Geometry, actual: Geometry, operation: str) -> None:
    if actual != expected:
        raise RuntimeError(
            f"window frame changed during {operation}: "
            f"expected={expected.as_dict()} actual={actual.as_dict()}"
        )


def capture(window_id: str, destination: Path) -> str:
    destination.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["import", "-window", window_id, str(destination)],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return hashlib.sha256(destination.read_bytes()).hexdigest()


def exercise_pane(
    window: object,
    window_id: str,
    button_name: str,
    marker_name: str,
    expected_frame: Geometry,
    cycles: int,
    capture_directory: Path,
) -> list[dict[str, object]]:
    button = find_button(window, button_name)
    samples: list[dict[str, object]] = []
    for cycle in range(1, cycles + 1):
        stem = f"{button_name.lower()}-cycle-{cycle}"
        before_path = capture_directory / f"{stem}-before.png"
        collapsed_path = capture_directory / f"{stem}-collapsed.png"
        expanded_path = capture_directory / f"{stem}-expanded.png"
        before_hash = capture(window_id, before_path)
        before_markers = named_node_snapshot(window, marker_name)

        invoke(button)
        time.sleep(0.8)
        collapsed_frame = geometry(window)
        assert_fixed_frame(expected_frame, collapsed_frame, f"{button_name} collapse {cycle}")
        collapsed_hash = capture(window_id, collapsed_path)
        collapsed_markers = named_node_snapshot(window, marker_name)
        if collapsed_hash == before_hash:
            raise RuntimeError(f"pane image did not change after {button_name} collapse {cycle}")

        invoke(button)
        time.sleep(0.8)
        expanded_frame = geometry(window)
        assert_fixed_frame(expected_frame, expanded_frame, f"{button_name} expand {cycle}")
        expanded_hash = capture(window_id, expanded_path)
        expanded_markers = named_node_snapshot(window, marker_name)
        if expanded_hash == collapsed_hash:
            raise RuntimeError(f"pane image did not change after {button_name} expand {cycle}")
        samples.append(
            {
                "cycle": cycle,
                "beforeImage": str(before_path),
                "beforeSha256": before_hash,
                "beforeMarkers": before_markers,
                "collapsedFrame": collapsed_frame.as_dict(),
                "collapsedImage": str(collapsed_path),
                "collapsedSha256": collapsed_hash,
                "collapsedMarkers": collapsed_markers,
                "expandedFrame": expanded_frame.as_dict(),
                "expandedImage": str(expanded_path),
                "expandedSha256": expanded_hash,
                "expandedMarkers": expanded_markers,
            }
        )
    return samples


def structural_snapshot(window: object) -> dict[str, object]:
    nodes = list(walk(window))
    table_cells = [node for node in nodes if role_name(node) == "table cell"]
    table_cell_names = sorted({name(node) for node in table_cells})
    expected_cells = {
        "Cargo.toml",
        "Today, 10:42",
        "2.4 KB",
        "TOML document",
    }
    missing_cells = sorted(expected_cells.difference(table_cell_names))
    if missing_cells:
        related_nodes = [
            {"role": role_name(node), "name": name(node)}
            for node in nodes
            if any(
                token in name(node)
                for token in ("Cargo", "Today", "2.4", "TOML", "Name", "Date", "Size", "Kind")
            )
        ]
        raise RuntimeError(
            f"missing expected AT-SPI cells: {missing_cells}; "
            f"related nodes: {related_nodes}"
        )
    header_rows = [
        name(node)
        for node in nodes
        if role_name(node) == "table row" and name(node) == "Name Date Modified Size Kind"
    ]
    if not header_rows:
        raise RuntimeError("native table header row is missing from AT-SPI")

    interactive_roles = {
        "check box",
        "combo box",
        "entry",
        "menu",
        "menu item",
        "password text",
        "push button",
        "radio button",
        "search box",
        "switch",
        "toggle button",
    }
    interactive = []
    for node in nodes:
        node_role = role_name(node)
        if node_role not in interactive_roles:
            continue
        interactive.append(
            {
                "role": node_role,
                "name": name(node),
                "showing": has_state(node, pyatspi.STATE_SHOWING),
                "visible": has_state(node, pyatspi.STATE_VISIBLE),
                "enabled": has_state(node, pyatspi.STATE_ENABLED),
                "sensitive": has_state(node, pyatspi.STATE_SENSITIVE),
                "focusable": has_state(node, pyatspi.STATE_FOCUSABLE),
            }
        )
    required_sensitive_controls = {
        "Sidebar",
        "Inspector",
        "Back",
        "New Folder",
        "Search files",
        "Show hidden files",
        "Open in Editor",
    }
    missing_sensitive_controls = sorted(
        control_name
        for control_name in required_sensitive_controls
        if not any(
            control["name"] == control_name and control["sensitive"]
            for control in interactive
        )
    )
    if missing_sensitive_controls:
        raise RuntimeError(
            "expected sensitive controls are missing from AT-SPI: "
            f"{missing_sensitive_controls}"
        )
    return {
        "nodeCount": len(nodes),
        "tableCellNames": table_cell_names,
        "tableCells": [
            {"name": name(node), "description": description(node)} for node in table_cells
        ],
        "tableHeaderRows": header_rows,
        "interactive": interactive,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--title", default="Rinka Explorer")
    parser.add_argument("--cycles", type=int, default=3)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--capture-directory", type=Path)
    parser.add_argument("--skip-pane-cycles", action="store_true")
    arguments = parser.parse_args()

    window = find_window(arguments.title, 15.0)
    window_id = subprocess.check_output(
        ["xdotool", "search", "--onlyvisible", "--name", f"^{arguments.title}$"],
        text=True,
    ).splitlines()[0]
    initial_frame = geometry(window)
    result: dict[str, object] = {
        "window": arguments.title,
        "initialFrame": initial_frame.as_dict(),
        "structure": structural_snapshot(window),
    }
    if not arguments.skip_pane_cycles:
        capture_directory = arguments.capture_directory or (
            arguments.output.parent / "pane-cycles"
        )
        result["sidebarCycles"] = exercise_pane(
            window,
            window_id,
            "Sidebar",
            "Show hidden files",
            initial_frame,
            arguments.cycles,
            capture_directory,
        )
        result["inspectorCycles"] = exercise_pane(
            window,
            window_id,
            "Inspector",
            "Open Cargo.toml in editor",
            initial_frame,
            arguments.cycles,
            capture_directory,
        )
    result["result"] = "PASS"
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

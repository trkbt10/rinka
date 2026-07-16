.DEFAULT_GOAL := help

.PHONY: help install typecheck lint test build dev fixture surface macos-demo macos-bundle macos-accelerator-probe macos-clipboard-probe macos-dialog-probe macos-scene-matrix macos-transition-matrix macos-transition-visual-matrix macos-visual-matrix gtk-demo gtk-scene-matrix gtk-wayland-smoke gtk-gnome-shell-smoke windows-bootstrap windows-demo windows-scene-matrix macos-drag-drop-probe macos-text-input-probe macos-menu-bar-probe

help:
	@printf '%s\n' 'install typecheck lint test build dev fixture surface macos-demo macos-bundle macos-accelerator-probe macos-clipboard-probe macos-dialog-probe macos-scene-matrix macos-transition-matrix macos-transition-visual-matrix macos-visual-matrix gtk-demo gtk-scene-matrix gtk-wayland-smoke gtk-gnome-shell-smoke windows-bootstrap windows-demo windows-scene-matrix macos-drag-drop-probe macos-text-input-probe macos-menu-bar-probe'

install:
	@cargo fetch

typecheck:
	@cargo check --workspace --all-targets

lint:
	@cargo fmt --all -- --check
	@cargo clippy --workspace --all-targets -- -D warnings

test:
	@cargo test --workspace

build:
	@cargo build --workspace --all-targets

dev:
	@cargo run -p rinka-explorer

fixture: surface

surface:
	@cargo run -p rinka-explorer -- --extract-surface

macos-demo:
	@cargo run -p rinka-explorer

macos-bundle:
	@scripts/bundle-macos.sh

macos-accelerator-probe:
	@scripts/appkit-accelerator-probe.sh

macos-clipboard-probe:
	@scripts/appkit-clipboard-probe.sh
macos-dialog-probe:
	@scripts/appkit-dialog-probe.sh

macos-drag-drop-probe:
	@scripts/appkit-drag-drop-probe.sh

macos-text-input-probe:
	@scripts/appkit-text-input-probe.sh

macos-menu-bar-probe:
	@scripts/appkit-menu-bar-probe.sh

macos-scene-matrix:
	@scripts/appkit-scene-matrix.sh

macos-transition-matrix:
	@scripts/appkit-transition-matrix.sh

macos-transition-visual-matrix:
	@scripts/appkit-transition-visual-matrix.sh

macos-visual-matrix:
	@scripts/appkit-visual-matrix.sh

gtk-demo:
	@cargo run -p rinka-explorer

gtk-scene-matrix:
	@scripts/gtk-scene-matrix.sh

gtk-wayland-smoke:
	@scripts/gtk-wayland-smoke.sh

gtk-gnome-shell-smoke:
	@scripts/gtk-gnome-shell-smoke.sh

windows-bootstrap:
	@powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/windows-bootstrap.ps1

windows-demo:
	@cargo run -p rinka-explorer

windows-scene-matrix:
	@powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/windows-scene-matrix.ps1

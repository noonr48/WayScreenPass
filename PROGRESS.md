# WayScreenPass — Progress Tracker

**Updated**: 2026-04-14

## Current Project State

WayScreenPass is now oriented around a single supported runtime model: a dedicated headless Wayland session remoted over Tailscale. The server CLI, setup flow, systemd unit, and status output all assume that model, and the previous mixed-mode portal/live-desktop posture is no longer exposed as the supported path.

## Validation Snapshot

- `cargo test --workspace` — **26 passed**
- `cargo build --release` — **passed**

## Component Snapshot

| Area | Status | Notes |
|------|--------|-------|
| Headless session runtime | ✅ | Dedicated Sway session, session metadata, app launch |
| Video streaming | ✅ | `grim` capture + persistent `x264` encoder |
| Input injection | ✅ | Wayland virtual keyboard + pointer scoped to the headless session |
| Clipboard sync | ✅ | `wl-clipboard` wired into the headless session environment |
| Client and tray | ✅ | Existing client/tray continue to build and test cleanly |
| Setup and status UX | ✅ | Headless-only guidance and corrected service startup |
| Packaging assets | ✅ | Service unit fixed, socket unit removed, deps aligned with headless mode |
| Live runtime smoke test | ⚠️ Recommended | Still worth verifying on the target machine/compositor build |

## What Changed in This Refactor

- Replaced the old headless stub input path with a real Wayland virtual input backend.
- Moved headless input ownership onto a dedicated thread so Wayland/XKB objects stay out of shared async state.
- Removed server-side portal/live-desktop mode branching from the exposed runtime path.
- Corrected the packaged systemd service to run `remote-desktop-server start`.
- Removed the unsupported socket activation unit from deployment assets.
- Updated top-level docs to describe the actual supported architecture.

## Remaining Practical Risk

The codebase now validates cleanly at build/test time, but compositor/runtime behavior still depends on the target machine having the expected wlroots/Sway virtual input support and the required tools (`sway`, `grim`, `wl-clipboard`, `x264`) installed.

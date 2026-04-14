# WayScreenPass — Implementation Summary

**Updated**: 2026-04-14  
**Status**: Headless-only server flow implemented and validated at the workspace build/test level.

## Summary

WayScreenPass now treats the dedicated headless session as the only supported server mode. The server starts a headless Sway session, captures it with `grim`, encodes it through a persistent `x264` process, and injects keyboard/pointer events back into that same session through Wayland virtual input protocols.

Portal/current-session code may still exist in-tree, but it is no longer the supported runtime path exposed by the server CLI, setup flow, or packaged service.

## Key Changes

| Change | Result |
|--------|--------|
| Headless input backend replaced | Keyboard and pointer events now target the headless Wayland session instead of a stub path |
| Dedicated input thread added | Non-`Send` Wayland/XKB objects stay owned by one thread while the async server sends commands over a channel |
| Server runtime simplified | Monitor selection and streaming now assume the dedicated headless output |
| Setup/systemd flow corrected | Generated and packaged services run `remote-desktop-server start` |
| Deployment assets cleaned up | Unsupported socket activation unit removed |
| Docs corrected | README/progress summary now match the actual product surface |

## Runtime Architecture

```text
Client input/events
        |
        v
remote-desktop client -> TCP protocol over Tailscale -> remote-desktop-server
                                                     |-> headless session capture (`grim`)
                                                     |-> H.264 encode (`x264`)
                                                     |-> clipboard sync (`wl-clipboard`)
                                                     `-> input command channel -> Wayland virtual keyboard/pointer
```

## Validation

- `cargo test --workspace`
- `cargo build --release`

Both commands pass after the headless-only refactor.

## Recommended Next Check

Run a live smoke test on the target machine:

1. Start `remote-desktop-server start`
2. Connect with `remote-desktop`
3. Verify keyboard input, pointer input, clipboard sync, and `remote-desktop-server launch`

That is the remaining best end-to-end confirmation because the last environment-specific variable is the compositor/runtime stack on the actual host.

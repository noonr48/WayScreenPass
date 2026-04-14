#!/bin/bash
# Connect to framework via VNC (headless, no portal needed)

VNC_HOST="100.74.168.24:5900"

# Run vncviewer with cert warnings suppressed
vncviewer "$VNC_HOST" \
  --no-xcursor \
  --alert=none \
  --acceptClipboard \
  --sendClipboard \
  --sendPrimary \
  --fullScreen \
  "$@"

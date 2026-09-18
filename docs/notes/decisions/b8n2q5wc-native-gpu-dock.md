---
id: b8n2q5wc
date: 2026-09-18
kind: adr
status: accepted-direction
tags: [architecture, gpu, shell]
---
# Native GPU dock

The operator selected Provlita, an independent Xilem/Vello dock inspired by
Gershwin, alongside Lom and Bemenu. Keep the first UI to pinned application tiles.
Reuse the supported non-Winit embedding and exact GPU grant path. Sophia owns
admission, catalog actions and presentation authority, without a Vello dependency.

A Gershwin Workspace fork would bring file-manager/GNUstep integration beyond the
required dock. A GTK dock would reopen the display-backend problem. CPU Cairo
rendering does not meet the requested GPU direction. Build a small native client
using original assets and a generic protocol contract instead.

Consequences: third-component budgets/reservations and persistent catalog actions
need Sophia work before the dock can run. GPU readback/upload remains the current
path; neither zero-copy nor a hard VRAM quota is claimed.

See [architecture](../../../ARCHITECTURE.md) and [plan](../plans/r7k3m2va-native-dock-critical-path.md).

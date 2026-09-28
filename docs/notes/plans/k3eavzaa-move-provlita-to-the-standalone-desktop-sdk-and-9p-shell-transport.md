---
id: k3eavzaa
date: 2026-09-27
kind: plan
tags: [plan, milestone, 9p]
---
# Move Provlita to the standalone desktop SDK and 9P shell transport

## Scope and exit

The operator asked for Provlita's adoption to finish, as the last application
item of Sophia's t263 (native C and Rust desktop SDKs). Move the dock's
production shell connection from Sophia's in-tree client to the standalone Rust
desktop SDK, using only 9P with no IPC dependency or fallback. Preserve protocol
revision 8, the persistent-catalog capability mask, rendering and
presentation-based action authority.

The SDK owns file transport, object fetching, submission custody and retry.
Provlita observes every admitted unit's custody within the SDK's bounded ticket
history; submission acceptance does not imply presentation. Refused or
ambiguous work ends the service without replay.

Exit: a published SDK pin, the full offline gate, file-wire lifecycle, refusal
and budget tests with negative controls, and the retained service assertions
moved to 9P. Live promotion is separate: a packaged profile selects the 9P dock,
then the operator verifies presentation and actions.

## Task details

## t007

Use `sophia-shell-protocol` and `sophia-shell-client` from
`sophia-org/sophia-desktop-sdk-rs` at published revision
`ea9cf651d37649fc1be889b3cfef6c1729b055f8`, the same revision Lom pins. The
lockfile drops `sophia-protocol` from the Sophia repository. The Lom GPU helper
stays at `ad349869`: `crates/shell-gpu` is unchanged between that revision and
Lom `53d3a921` and has no Sophia dependency.

`--serve` requires a nonempty `SOPHIA_SHELL_9P_SOCKET` and refuses
`SOPHIA_SHELL_SOCKET`, even when empty or set beside the file endpoint. It prints
`provlita_shell_transport` before GPU admission. Idle turns sleep until the SDK's
wake deadline, at most 4 ms.

A candidate is measured by its native file encoding
(`encode_catalog_content_candidate`) against the smaller of the limits and the
permit's byte budget, replacing the IPC frame lengths. Upload chunk sizing still
derives from `max_frame_payload` and `max_chunk_bytes`, as Lom does. Record-level
budgets without IPC framing are Sophia's t268.

The SDK does not export the application-catalog capability bit (`1 << 5`). Provlita
defines it locally, citing its Sophia source.

## Implementation and validation, 2026-09-27

The service fixture speaks the 9P file contract over a Unix socket. It adapts
Lom's scripted wire, which exercises partial upload-slot writes, multi-read
objects, and acknowledgement of Submitted before the transaction reopens. The
catalog is published as the `catalog` object with its identities. Candidates
arrive as one `CatalogCandidate` record; activations as `CatalogActivate`, each
answered with an outcome event. The two retained scenarios still pass:
two-output presentation with exact clicks, and a withheld release with slot
reuse. New tests cover a refused submission, a candidate over the permit budget,
and the serve entry's environment refusals.

The full `tools/check.sh` passed offline and locked: 12 tooling tests, the layout
audit, formatting, 21 Rust tests, and Clippy with warnings denied. It ran at
nice 19 in a sandbox with devices hidden, network disabled and display variables
unset. The first run failed only on formatting.

The negative controls were run the same way:
- Ignoring custody refusals failed only the refusal test.
- Removing the byte-budget check failed only the budget test.

The source was restored after each.

Evidence: `~/.local/state/sophia/development-evidence/provlita-9p-only/`
(`full-gate.log`, `full-gate-run1-fmt.log`, `custody-mutant.log`,
`budget-mutant.log`).

These are scripted peer tests. They make no claim about Session policy, the
production export, GPU execution or native presentation, and the current desktop
was not modified.

## Production-export verification, moved from Sophia t263 (2026-09-27)

Sophia closed t263 with the SDK releases `sophia-desktop-sdk-rs` v0.1.0
(`ea9cf651`, the revision this repository pins) and `sophia-desktop-sdk-c`
v0.1.0. At the operator's direction, Provlita's check against a production
Session export moved to this task. That check is external, like the Lom and
Bemenu live runs, and belongs in the niltempus integration repository, not in
Sophia or this repository's ordinary checks.

It must run the dock through protected launch against a real persistent-catalog
export. It must show a catalog read, allocation, upload through the slots,
candidate presentation, one exact activation with its outcome, and retirement
and release. It must also confirm the transport record and a refused
`SOPHIA_SHELL_SOCKET`. Later live promotion adds the dock to a packaged profile,
followed by the operator's attended check. t007 closes after both.

## Connections

- [Architecture](../../../ARCHITECTURE.md)
- [Native dock critical path](r7k3m2va-native-dock-critical-path.md)

## Graceful stop, 2026-09-27

Live reload stops components with SIGTERM, and a component started as a bwrap
`--as-pid-1` namespace init ignores that signal unless it installs a handler.
`--serve` now installs SIGTERM/SIGINT handlers first; the semantics and limits are
in ARCHITECTURE.md. `tests/stop.rs` sends real signals to the binary held in
negotiation by a silent listener: SIGINT to a direct child, SIGTERM to the dock
as namespace init, and a control showing a handler-less namespace init ignores
the same SIGTERM. `tests/service.rs` raises SIGTERM in-process with a launch
outcome held by the fixture, then checks the report, the disconnect and that no
activation is replayed.

## Negotiated upload chunks (2026-09-28)

The SDK pin is now `c1323401b7e336606408499b13097d1270a2319d`. Resource
descriptions can cross the file wire under reduced chunk limits; the content
owner still checks their exact granted layout. Provlita uses the canonical
`max_chunk_bytes` rule, with scripted-peer tests at row boundaries. Its Lom
GPU-library pin is unchanged.

The full offline `tools/check.sh` passed 26 Rust tests, tooling tests, layout,
formatting and strict clippy. Earlier mutation controls are recorded in
`ipc-removal-inventory/t268-rust-consumers.md`; the final pin gate is
`ipc-retirement/t268-provlita-c132340-check.log`. The private Cargo cache
was explicitly seeded from the published SDK's local source. The gate had no
network, display or devices. Production-export and native acceptance remain
separate; no installed component changed.

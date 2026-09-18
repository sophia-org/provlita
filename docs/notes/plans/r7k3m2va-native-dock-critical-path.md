---
id: r7k3m2va
date: 2026-09-18
kind: plan
tags: [critical-path, gpu, shell]
---
# Native dock critical path

Scope: pinned-application GPU dock alongside Lom and Bemenu. Scaffold creation
is complete; implementation and acceptance below remain open. Coordinate file
ownership before cross-repository edits; do not import isolated input work.

## t001

Define bounded third-component admission, budgets and per-output reservations.

Coordinate Session/runtime ownership. Replace the current two-role/two-active-epoch assumptions with bounded inventories. Define per-component budget reservations under aggregate caps, no authority inheritance, exact restart identities and bar/dock edge-conflict rules. Exit: valid three-client admission and budget/reservation/neighbor negatives through production owners.

### Admission-owner foundation (2026-09-18)

Sophia checkpoint `9f1da60e` (local, unpublished) adds this foundation.
Six real-socket transport controls and fourteen runtime library controls pass;
strict focused Clippy and source layout pass. Removing aggregate reservation
checking makes the held-consumer control fail; restored controls pass. No full
canonical or hardware run is claimed. Evidence is retained in Sophia under
`.artifacts/provlita-admission-capacity`.

The shared Sophia content registry now has an explicit bounded active-owner
capacity (one through three); existing construction still defaults to two.
Capacity does not increase the 64 MiB aggregate reservation ceiling or authorize
any role. No configuration or live Session caller enables the third owner yet.

A device-hidden private-socket control admits three actual transports with
24/20/20 MiB reservations, retains a disconnected peer's real pixel lease,
refuses replacement while those bytes remain charged, and lets both neighbors
upload. Dropping the exact retained consumer permits a fresh epoch; final
collection reaches quiescence. This is storage/transport evidence, not GPU,
process-supervision, edge reservation or native dock acceptance.

Remaining t001 work: role-specific capability derivation, fixed inventory across
Session/process scheduling, explicit production budget partition, and exact
per-output bar/dock reservation conflict and retirement handling. Persistent
catalog launch remains t002. Do not configure Provlita as a bar to bypass these
requirements. The project binary remains a scaffold.

## t002

Implement generic persistent presented catalog actions.

Design/version persistent catalog selection and launch events. Reuse actual catalog and execution owners, preserve transient Bemenu and descriptor behavior, and test independent encoding/decoding. Exit: exact presented selection executes once; stale/revoked/wrong-target/replayed events and exhausted response credit refuse without effects.

## t003

Build the retained Xilem view and bounded GPU adapter.

Pin Lom-compatible Xilem/Masonry/Vello and GPU versions. Reuse only cohesive generic adapters, with affected Lom regressions. Retain per-output host/widget identity, exact DRM selection, bounded waits and source leases. Exit: view fixtures and device-hidden scheduler/admission controls pass; actual GPU evidence remains separate.

## t004

Integrate KDL pins, catalog identity and native dock service.

Implement bounded KDL parsing and pins resolving to authorized catalog IDs. Connect retained view updates, persistent allocation/presentation/input and lazy resource generations. Exit: real private-socket contract controls, unavailable entries, two-output progress and exact cleanup; proposed example becomes tested syntax.

## t005

Prove three-component isolation, progress and reclamation.

Run all three production ownership paths with simulated native completion where needed, real held byte consumers, independent crash/restart and resource pressure. Exit: recorded numeric high-water bounds, no replay, no neighbor revocation and final exact reclamation; contained canonical and affected projects pass. No native acceptance inferred.

## t006

Accept both-monitor coexistence and measured workload.

Freeze signed clean sources, binaries, profile and rollback. Predeclare workload counts/resource and latency thresholds. Operator verifies dock on both outputs beside Lom/Bemenu, launches, focus behavior, independent recovery and clean shutdown. Exit: matching logs plus operator observations. Installation/promotion is a separate decision.

## Connections

[Architecture](../../../ARCHITECTURE.md), [decision](../decisions/b8n2q5wc-native-gpu-dock.md),
[task queue](../../../todo.md). Task state lives only in the queue.

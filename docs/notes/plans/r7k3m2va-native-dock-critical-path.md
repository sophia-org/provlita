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

## Implementation checkpoint: inventory, wire and configuration (2026-09-18)

The next Sophia slice implements the bounded three-role inventory, seals role
registration before connection reservations begin, and partitions the existing
64 MiB cap as bar 24 MiB, launcher 20 MiB and dock 20 MiB. Explicit reservations
bind a component to a distinct edge and logical thickness; allocation scales it
once. The current profile applies these edges to all outputs, conservatively
refusing shared-edge configurations. Retained bar bands are keyed by connection
owner rather than selected from the first bar. This is not yet three-client
native service: Dock startup and negotiation remain explicitly refused until
its persistent service is implemented.

Revision-8 codec work adds persistent catalog candidate/activation records and
stable registered/desktop identities. The legacy catalog encoding is unchanged;
the new complete transaction requires a bijection between slots and identities.
Missing, duplicated, stale or cross-transaction identity records refuse. No
transient opening or workspace-indicator action is fabricated for a dock.

Device-hidden evidence in Sophia's `.artifacts/provlita-admission-capacity`:
`checkpoint-gates` runs 789 Rust controls with zero failures, 20 ignored, strict
affected Clippy and no warning lines; `catalog-independent` checks five golden
records and 1,644 mutations against the independent C reader. The identity
publication controls use actual Session catalog construction and encoders,
not process launch, native presentation or a negotiated revision-8 service.
Source-layout checking also passes. These are scoped gates, not canonical or
native acceptance. Earlier setup/compile failures remain separately retained.

Provlita now implements bounded KDL parsing and `check --config PATH`.
The example resolves syntactically to a 172 by 64 logical-pixel three-pin strip;
no catalog authorization is implied by parsing it. Device-hidden project checks
pass four Rust controls, twelve tooling controls, formatting, layout and strict
Clippy (`.artifacts/provlita-config-check/stable-identities` in Sophia).

Open critical path remains: per-connection catalog publication/execution,
reserved once-only persistent action outcomes, complete candidate/input binding,
role negotiation and service construction, retained GPU view and scheduling,
three-client lifecycle workload, `lom-test dock`, then attended acceptance.
None of t001–t006 is closed by this checkpoint. No hardware, installation,
live endpoint access or pending Sophia implementation publication occurred.

### Shared execution owner (2026-09-18)

Sophia's signed local checkpoint `00dea343` carries an explicit transient or
persistent cause through the existing bounded launch queue, worker verification,
execution attempt and child attribution. Persistent requests do not fabricate a
menu opening or keyboard lease. Exact entry/command ownership, grant revocation
and once-only execution per admission use the same production queue as Bemenu.
Replay after an admission has retired still requires the forthcoming presented
action ledger; queue de-duplication alone is not that authorization boundary.

The execution boundary additionally requires the connection's actual role
capability and profile. A real transient connection with the same grant cannot
execute a persistent cause. Persistent negotiation remains refused, so this
checkpoint enables no dock process or launch path yet.

Device-hidden final checks: 498 passing Rust controls, zero failures, 15 ignored;
strict affected Clippy and layout pass. Evidence is
`.artifacts/persistent-catalog-queue-v4` in Sophia. The three persistent queue
controls supply authorized events and prove queue custody, exact command/grant,
duplicate dispatch, peer-specific revocation and capacity refusal. They do not
prove presentation authorization or a revision-8 socket roundtrip. Existing
transient execution tests use private sockets and short-lived test processes;
the new wrong-role control refuses before spawn. Earlier compile and lint
failures remain in separate artifact directories. No hardware or native session
was run and no Sophia implementation commit was pushed.

### Retained Xilem host (2026-09-18)

The dock now uses the pinned non-Winit Xilem/Masonry embedding. Xilem callbacks
update its application state directly; there is no additional TEA dispatcher.
Catalog observations reconcile the retained widget tree. Stable configured IDs
resolve to current slots; unavailable entries produce no intent. One bounded
pending intent keeps its original catalog generation and slot across updates.
This intent is not a Sophia Action or launch authorization.

The device-hidden project gate passes, including two real Xilem controls for
widget identity, changed pixels, exact tile bounds, atomic catalog refusal and
callback origin. CPU rasterization is test-only. The supplied ButtonPress control
exercises Xilem message dispatch, not physical pointer input or shell authority.
Hit rectangles use transformed border boxes, not descendant paint bounds.
The earlier layout failures and final passing bounds-check.log are retained in
Sophia's .artifacts/provlita-xilem-host directory.

GPU rendering, native protocol dispatch, exact presented-action authorization,
three-component coexistence and attended acceptance remain open. The host alone
does not make lom-test dock ready. Tasks t001 through t006 remain open.

### Independent catalog owners and persistent storage (2026-09-18)

Sophia `607d0b1b` keeps publications per exact connected grant and routes the
single catalog worker through its actual pending/dispatch owner. The two-peer
control uses real sockets and a real `/bin/true` child; catalog/presentation
authority remains supplied. Final device-hidden Session checks pass 477 tests,
15 ignored, strict Clippy and layout. A compiled wrong-owner guard mutant fails.
The first two archive runs tested the baseline by mistake; they are explicitly
excluded from candidate evidence. Corrected evidence is in Sophia's
`.artifacts/dock-catalog-owners-v3`.

Persistent candidate storage subsequently carries the catalog generation through
actual assembly and resource-bearing renderer bundles, with revalidation before
render transfer. Six new controls plus 32 existing candidate controls pass, and
a compiled generation-check mutant fails. This is supplied completion/store
evidence only (`.artifacts/dock-candidate-binding`), not enabled dock transport.
The next integration is revision-8 socket service, exact presented action
authorization and outcome custody. GPU service and the physical wrapper remain
after that; no task is closed by these foundations.

Sophia candidate storage is signed at `68d9da00`; response custody is signed at
`f5d7913e`. Typed activation intake reserves aggregate control credit, retains the
first exact outcome across refusal and transfers it once to the partial-write
FIFO. Seventeen device-hidden runtime controls and the combined 477 Session
controls pass, with 15 Session tests ignored. A compiled credit-omission mutant
fails. These response fixtures supply negotiated state and simulate FIFO drain;
the complete dock handshake/action/launch chain remains open. Post-mutation
positive checks use a fresh dedicated target after one shared-target retry reused
the mutant binary; that retry is explicitly non-evidence. See Sophia's
`.artifacts/dock-catalog-response`. No native run, deployment or push occurred.

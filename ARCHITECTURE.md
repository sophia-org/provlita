# Provlita architecture

Status: accepted direction, implementation pending. First target: a small,
independent GPU dock beside Lom and Bemenu on generic Linux distributions.
No custom kernel changes. Sophia remains rendering-library agnostic.

## Experience and scope

Use original Gershwin-inspired square tiles, clear borders, restrained colors,
labels and pressed feedback. Initially show three pinned applications in a
bottom-centered strip on selected outputs. Reserve enough edge space to avoid
covering application windows. Dock presses do not acquire typing focus.
Auto-hide, hover magnification, previews, running-window tracking, drag-and-drop,
trash services and file-manager integration are outside the first milestone.

## Model, view and effects

Model state consists of stable pin IDs, authorized catalog identities/revisions,
output/allocation identities, layout, and exact outstanding actions. Use typed
events and deterministic updates; keep I/O and rendering in explicit adapters.
Use Xilem's intended reactive reconciliation rather than an additional competing
UI framework. Retain one bounded Masonry host per grant/output/allocation/scale
identity; never reconstruct the widget tree for every rendered frame.

Start from Lom's pinned Xilem commit b81d8d7a631849def6eeab282561439b963862e5,
using xilem_masonry for the documented non-Winit embedding, Masonry and
masonry_imaging with Vello, wgpu 28/Vulkan and exact DRM-device selection. These
are planned dependencies, deliberately not fetched by this scaffold. Freeze an
explicit compatible set when rendering is introduced; do not silently track main.

Reuse reviewed generic shell lifecycle and render-admission seams. Extract only
cohesive shared pieces from Lom when both clients exercise the same owner and
controls. Provlita's UI/model must not depend on Lom's binary or workspace view.

## Presentation and GPU boundary

Session grants GPU access explicitly, default denied, independently of content
permission. Retain and validate the admitted render fd; require unique non-CPU
Vulkan adapter selection with VK_EXT_physical_device_drm matching dev_t. Reuse
the bounded read-only sysfs discovery contract. No PCI/CPU fallback, ambient
DISPLAY, Wayland endpoint, private display server or broad host sysfs access.

Vello renders to an offscreen GPU target. The existing supported path performs
bounded readback followed by content upload over shell IPC: it is GPU-rendered,
not zero-copy. Treat upload and actual native presentation as separate facts.
Use bounded GPU jobs/readback waits and per-output dirty scheduling. Unchanged
outputs do not submit. Keep pending presentation, input binding and resource
release independent, with aggregate byte/record/slot limits and held-consumer
progress. Direct device access does not imply a hard aggregate VRAM quota; retain
that limitation rather than claiming one-job scheduling enforces it.

## Sophia prerequisites

At baseline 79c11c85 Sophia explicitly admits only bar and application-launcher
roles, with a two-active-epoch ceiling. Three processes do not fit that contract
by configuration alone. Add a bounded component inventory, individually derived
capabilities and independent process/grant identities. A role is operator-facing
selection, not permission implied by the executable or socket.

Rebalance explicit grant budgets under the existing aggregate limits before
admitting a third client. Do not simply copy two large GPU/content budgets into
three slots. Coordinate edge reservations per output so the bar and dock coexist;
release only the retiring owner's reservation and preserve topology/scale identity.

Current launcher activation names a transient opening and its event. A persistent
dock needs a generic presented catalog-action contract. Reuse Session's catalog,
policy checks and execution queue; do not fabricate an opening or treat workspace
indicator actions as application launch authority. Validate exact grant, catalog
revision, presented target and event; reserve response custody before irreversible
admission; retries may send a response but may not execute a second launch.
Version required wire extensions and retain old client negotiation behavior.

Both Rust and non-Rust clients must be able to implement this contract. Keep
protocol types independent of Xilem/Vello and update independent decoding and
negative controls alongside new records.

## Configuration and assets

KDL selects pins by stable catalog identity, output selection and appearance.
No arbitrary command strings. Unknown catalog entries are visibly unavailable.
Use bounded bundled original icons/vector tiles initially; no recursive host
icon-theme discovery, broad home access or unbounded image decoding. The example
KDL is parsed and tested. Pin names use `registered:<name>` or
`desktop:<desktop-file-id>`; catalog resolution and native serving remain pending.

## Validation and acceptance

Headless tests cover model updates, exact presented clicks, duplicate/stale
refusal, resource pressure, two-output progress, reconnection and cleanup with
all three component owners. Keep protocol tests independent of rendering mocks.
The affected existing Lom/Bemenu tests remain required after common extraction.

Before an attended run, freeze source/build identities and run contained canonical
checks. The physical matrix verifies placement and launch on both monitors,
independent restart, unaffected peer components, and final owner quiescence.
Define measured resource and latency workload bounds before collecting results;
do not borrow a different workload's percentiles or call fixtures native acceptance.

## Sources and decisions

- [Xilem non-Winit embedding](https://github.com/linebender/xilem/blob/b81d8d7a631849def6eeab282561439b963862e5/xilem_masonry/README.md)
- [Gershwin Workspace](https://github.com/gershwin-desktop/gershwin-workspace): inspiration only.
- [Native independent GPU dock decision](docs/notes/decisions/b8n2q5wc-native-gpu-dock.md)
- [Critical path](docs/notes/plans/r7k3m2va-native-dock-critical-path.md)

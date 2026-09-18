# Provlita — Προβλήτα

A small native Sophia GPU dock, inspired by Gershwin's square application tiles.
Provlita means pier or quay in Greek.

**Status: bounded configuration parser implemented; native dock pending.**
`provlita check --config examples/minimal/config.kdl` validates the configuration
without resolving the catalog, rendering or contacting a session.
The intended stack is Xilem/Masonry + Vello through Sophia's shell IPC; no GTK,
X11, Wayland endpoint or private display server.

The first milestone is pinned application tiles on both monitors alongside Lom
and Bemenu, with exact launch authorization and independent cleanup. Running-app
tracking, previews, drag-and-drop and auto-hide are deferred.

- [Architecture](ARCHITECTURE.md)
- [Critical path](todo.md) and [task details](docs/notes/plans/r7k3m2va-native-dock-critical-path.md)
- [Style guide](docs/style-guide.md) and [work tracking](docs/work-tracking.md)
- [Notes](docs/notes/README.md)
- [Proposed KDL](examples/minimal/config.kdl) (tested configuration syntax)

## Development

The pinned Rust toolchain, Python 3 and Git are sufficient for the scaffold.
Use a disk-backed Cargo target directory. No extra GUI packages are needed yet.

```sh
sh tools/check.sh
cargo run -- check --config examples/minimal/config.kdl
```

The current gate runs only device-free tooling and scaffold tests. Future GPU
or native tests must be explicitly separated from it. On Sophia's development
host, run executable checks in the existing device-hidden isolation wrapper.

BSD-3-Clause. Gershwin is design inspiration; no Gershwin code or artwork is
included. Shared layout tooling originated in Lom and retains its license.

# Source and ownership style

Adopt Sophia/Lom's data-oriented ownership and source-layout conventions.
Separate model transitions, protocol I/O, rendering, configuration and adapters
when they have different owners. Prefer explicit IDs/generations, bounded storage
and typed outcomes. No hidden global ownership, unbounded waits, discarded errors
or newly broadened permissions to make a test pass.

Production and test sources at 800 lines require cohesion review. Production
sources over 1,000 lines fail the gate. Test files are reviewed at 800 lines but
have no automatic hard cap. Do not hide implementation in tests or split functions
arbitrarily to evade the limit. No accepted-debt exceptions exist in this project.

The inherited audit inspects Git-tracked and unignored source, including tooling
and shaders; source symlinks fail. External/generated top-level vendor,
third_party, target and .artifacts directories are excluded, and must not contain
Provlita-owned implementation. Tests exercise threshold boundaries, tracked
ignored files, read failures and symlink refusal. cargo test invokes the actual
audit too. Keep test bodies outside src and public APIs shaped for production.

Run sh tools/check.sh. It includes tooling regressions, actual layout audit,
formatting, tests and strict Clippy. Keep compiler warnings at zero. Evidence
must identify source and distinguish CPU fixtures, GPU rendering and native
presentation; none substitutes for another. Never weaken a gate to pass it.

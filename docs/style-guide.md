# Source and ownership style

Adopt Sophia/Lom's data-oriented ownership and source-layout conventions.
Separate model transitions, protocol I/O, rendering, configuration and adapters
when they have different owners. Prefer explicit IDs/generations, bounded storage
and typed outcomes. No hidden global ownership, unbounded waits, discarded errors
or newly broadened permissions to make a test pass.

## Repository tooling

Use Rust with an `xtask` entry point for new maintained test harnesses, isolation
orchestration, protocol validation, evidence collection and release tooling.
Keep identities typed, failures explicit and subprocess work bounded. This is
the direction for new tools; the current `sh tools/check.sh` remains the gate.

Use shell for short launchers and simple command sequences. Reserve Python for
disposable analysis and experiments. Prefer direct patches for source edits;
script repetitive transformations only when justified and inspect the diff.
Independent non-Rust protocol checks remain valuable interoperability evidence.

Migrate recurring scripts incrementally as their owning domain changes; do not
rewrite working tools before finishing the dock. Preserve device-hidden execution,
fail-closed prerequisites, timeouts, evidence formats and negative controls.
Prove equivalent behavior before replacing a gate. No migration may introduce
hardware autodetection or implicit live-session access into ordinary checks.

## Source layout and validation

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

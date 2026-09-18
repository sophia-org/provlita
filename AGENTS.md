# Working on Provlita

Read ARCHITECTURE.md and docs/style-guide.md before editing implementation.

- Xilem owns view reconciliation; deterministic model state and effect adapters
  have separate owners. Preserve real resource custody through completion.
- Keep test bodies outside src; do not widen production visibility for tests.
- Review source cohesion at 800 lines; production over 1,000 lines fails.
- Run sh tools/check.sh for source/tooling changes. Keep warnings at zero.
- GPU and native acceptance are separate from device-free tests. Do not add
  hardware autodetection, live sockets or GPU initialization to ordinary checks.
- Coordinate Sophia/Lom shared-file ownership before editing those repositories.
- Use master, signed commits, todo.txt notation and zk notes. Scope evidence to
  the exact source and distinguish fixtures from native/operator results.
- Never present a stub, proposed wire extension or simulated result as working
  native support. No CPU fallback may masquerade as the requested GPU path.

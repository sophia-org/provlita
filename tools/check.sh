#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
export PYTHONDONTWRITEBYTECODE=1
python3 -B -m unittest discover -s tools/tests -p 'test_*.py'
python3 -B tools/audit_source_layout.py
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-D warnings"
export RUSTDOCFLAGS="${RUSTDOCFLAGS:+$RUSTDOCFLAGS }-D warnings"
cargo fmt --all -- --check
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings

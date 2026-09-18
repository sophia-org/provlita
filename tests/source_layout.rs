//! Keep the real repository length check in ordinary Cargo test runs too.

use std::process::Command;

#[test]
fn repository_sources_fit_the_layout_budget() {
    let output = Command::new("python3")
        .args(["-B", "tools/audit_source_layout.py"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("source-layout checks require Python 3 and Git");
    assert!(
        output.status.success(),
        "source-layout audit failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

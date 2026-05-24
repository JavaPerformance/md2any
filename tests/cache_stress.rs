//! Integration: remote-image cache + retry stress harness.
//!
//! `#[ignore]`'d by default — runs by `cargo test -- --ignored`. Shells out
//! to `tests/integration/cache_stress.sh`, which lifecycles a Python HTTP
//! test server and drives the compiled binary against synthetic markdown.
//!
//! Skip semantics:
//!   - The shell script exits 77 (autoconf convention for "skip") when
//!     Python 3, curl, or the compiled binary is missing. We treat that
//!     as a *pass* with a printed reason rather than a failure so CI
//!     environments without the prerequisites don't block.
//!   - Any non-zero, non-77 exit means a real assertion failed.

use std::path::PathBuf;
use std::process::Command;

#[test]
#[ignore = "runs a local HTTP server + spawns md2any; use `cargo test -- --ignored`"]
fn remote_image_cache_stress() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script = manifest_dir.join("tests/integration/cache_stress.sh");
    let bin = manifest_dir.join("target/release/md2any");

    assert!(
        script.exists(),
        "harness missing: {} — run from project root",
        script.display()
    );
    assert!(
        bin.exists(),
        "release binary missing: {} — run `cargo build --release` first",
        bin.display()
    );

    let status = Command::new("bash")
        .arg(&script)
        .env("MD2ANY_BIN", &bin)
        .current_dir(&manifest_dir)
        .status()
        .expect("spawn bash harness");

    match status.code() {
        Some(0) => {}
        Some(77) => {
            // Prerequisites missing (Python 3 / curl / binary). Treat as
            // skipped so CI without the toolchain doesn't block.
            eprintln!("(skipped: prerequisites missing — harness reported exit 77)");
        }
        Some(code) => panic!("cache stress harness failed with exit {}", code),
        None => panic!("cache stress harness terminated by signal"),
    }
}

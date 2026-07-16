//! Byte-for-byte determinism tests for the `generate_catalog` binary.

use std::path::Path;
use std::process::Command;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn run_generator(input: &Path, output: &Path) {
    let status = Command::new(env!("CARGO_BIN_EXE_generate_catalog"))
        .arg("--input")
        .arg(input)
        .arg("--output")
        .arg(output)
        .status()
        .expect("generator runs");
    assert!(status.success(), "generator exited with {status}");
}

#[test]
fn fixture_generation_is_byte_for_byte_deterministic() {
    let input = fixture("models-dev-small.json");
    let one = tempfile::NamedTempFile::new().unwrap();
    let two = tempfile::NamedTempFile::new().unwrap();
    run_generator(&input, one.path());
    run_generator(&input, two.path());
    assert_eq!(
        std::fs::read(one.path()).unwrap(),
        std::fs::read(two.path()).unwrap()
    );
    let text = std::fs::read_to_string(one.path()).unwrap();
    assert!(text.ends_with('\n'));
    assert!(!text.ends_with("\n\n"));
}

#[test]
fn check_mode_accepts_current_output_and_rejects_stale_output() {
    let input = fixture("models-dev-small.json");
    let out = tempfile::NamedTempFile::new().unwrap();
    run_generator(&input, out.path());

    let check = Command::new(env!("CARGO_BIN_EXE_generate_catalog"))
        .args(["--input"])
        .arg(&input)
        .arg("--output")
        .arg(out.path())
        .arg("--check")
        .status()
        .expect("generator runs");
    assert!(check.success(), "check of current output must pass");

    std::fs::write(out.path(), b"stale").unwrap();
    let stale = Command::new(env!("CARGO_BIN_EXE_generate_catalog"))
        .args(["--input"])
        .arg(&input)
        .arg("--output")
        .arg(out.path())
        .arg("--check")
        .status()
        .expect("generator runs");
    assert!(!stale.success(), "check of stale output must fail");
    assert_eq!(
        std::fs::read(out.path()).unwrap(),
        b"stale",
        "--check must not write"
    );
}

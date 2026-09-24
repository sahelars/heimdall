//! Shared helpers for driving the real `heimdall` binary.
//!
//! Each integration test binary compiles this module separately and uses only
//! part of it, so unused helpers are expected here rather than dead code.
#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Output;

use serde_json::Value;

pub fn binary() -> PathBuf {
    assert_cmd::cargo::cargo_bin("heimdall")
}

/// Where the forked binaries keep Heimdall's application data.
///
/// These are real `heimdall` processes, so they resolve the production data
/// directory — the developer's actual application-data folder — unless told
/// otherwise. `HEIMDALL_DATA_DIR` is what tells them otherwise. Every vault
/// here is a fresh temp directory and per-vault files are named by a hash of
/// the vault path, so one shared directory cannot collide.
pub fn data_dir() -> PathBuf {
    std::env::temp_dir().join("heimdall-test-data")
}

/// A `heimdall` command that will not write into the real application data.
pub fn command() -> std::process::Command {
    let mut command = std::process::Command::new(binary());
    command.env("HEIMDALL_DATA_DIR", data_dir());
    command
}

/// Create a scaffolded vault and return its directory and path.
pub fn new_vault() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap().to_string();

    let output = command()
        .args(["create", "demo", "--root", &root])
        .output()
        .unwrap();
    assert!(output.status.success(), "create failed: {output:?}");

    let vault = format!("{root}/demo");
    (dir, vault)
}

pub fn run(args: &[&str]) -> Output {
    command().args(args).output().unwrap()
}

pub fn run_with_stdin(args: &[&str], stdin: &[u8]) -> Output {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = command()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

/// Parse stdout as the JSON envelope, asserting there is nothing else there.
pub fn envelope(output: &Output) -> Value {
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout must be UTF-8");
    serde_json::from_str(&stdout).unwrap_or_else(|err| panic!("stdout is not one JSON object: {err}\n{stdout}"))
}

pub fn data(output: &Output) -> Value {
    let envelope = envelope(output);
    assert_eq!(envelope["ok"], true, "expected success: {envelope}");
    envelope["data"].clone()
}

pub fn error_code(output: &Output) -> String {
    let envelope = envelope(output);
    assert_eq!(envelope["ok"], false, "expected failure: {envelope}");
    envelope["error"]["code"].as_str().unwrap().to_string()
}

/// The revision currently stored for one note.
pub fn revision_of(vault: &str, path: &str) -> String {
    let output = run(&["read", path, "--vault", vault]);
    data(&output)["document"]["revision"].as_str().unwrap().to_string()
}

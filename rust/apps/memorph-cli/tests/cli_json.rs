use serde_json::Value;
use std::{
    fs,
    net::TcpListener,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn json_mode_covers_version_tui_and_server_commands() {
    let binary = env!("CARGO_BIN_EXE_memorph");

    let version = Command::new(binary)
        .args(["--json", "--version"])
        .output()
        .unwrap();
    assert!(version.status.success());
    assert!(version.stderr.is_empty());
    let version: Value = serde_json::from_slice(&version.stdout).unwrap();
    assert_eq!(version["ok"], true);
    assert_eq!(version["result"]["version"], env!("CARGO_PKG_VERSION"));

    for args in [["--json", "tui"].as_slice(), ["--json"].as_slice()] {
        let output = Command::new(binary).args(args).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stderr.is_empty());
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["ok"], false);
        assert!(error["error"]
            .as_str()
            .unwrap()
            .contains("TUI requires an interactive terminal"));
    }

    for (command, interface) in [("web", "web"), ("api", "api")] {
        let port = TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let home = tempfile::tempdir().unwrap();
        let stdout = tempfile::NamedTempFile::new().unwrap();
        let port_arg = port.to_string();
        let mut process = Command::new(binary);
        process.args(["--json", command, "--port", &port_arg]);
        if command == "web" {
            process.arg("--no-open");
        }
        let mut child = process
            .env("HOME", home.path())
            .stdout(Stdio::from(stdout.reopen().unwrap()))
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        let line = loop {
            let output = fs::read_to_string(stdout.path()).unwrap();
            if output.ends_with('\n') {
                break output;
            }
            if let Some(status) = child.try_wait().unwrap() {
                let output = child.wait_with_output().unwrap();
                panic!(
                    "{command} exited before JSON startup ({status}): {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            assert!(Instant::now() < deadline, "{command} startup timed out");
            thread::sleep(Duration::from_millis(25));
        };

        child.kill().unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.stderr.is_empty());
        assert_eq!(
            line.lines().count(),
            1,
            "{command} emitted invalid startup text"
        );
        let started: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(started["ok"], true);
        assert_eq!(started["result"]["interface"], interface);
        assert_eq!(started["result"]["url"], format!("http://127.0.0.1:{port}"));
    }
}

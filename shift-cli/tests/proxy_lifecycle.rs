#![cfg(unix)]

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

struct Disposable(Child);
impl Drop for Disposable {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn legacy_pid_file_does_not_signal_an_unrelated_process() {
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join(".shift");
    fs::create_dir(&state).unwrap();
    let mut victim = Disposable(
        Command::new("sleep")
            .arg("30")
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let pid = victim.0.id().to_string();
    fs::write(state.join("proxy.pid"), &pid).unwrap();

    let result = Command::new(env!("CARGO_BIN_EXE_shift-ai"))
        .env("HOME", home.path())
        .args(["proxy", "stop", "--quiet"])
        .output()
        .unwrap();

    assert!(
        victim.0.try_wait().unwrap().is_none(),
        "stop signaled a process named only by a legacy PID file"
    );
    assert!(
        !result.status.success(),
        "unverifiable state must produce an actionable error"
    );
    assert_eq!(fs::read_to_string(state.join("proxy.pid")).unwrap(), pid);
}

fn cli(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_shift-ai"))
        .env("HOME", home)
        .args(args)
        .output()
        .unwrap()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Managed {
    home: tempfile::TempDir,
    port: u16,
}

impl Managed {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
            port: free_port(),
        }
    }
    fn ensure(&self) -> Output {
        cli(
            self.home.path(),
            &[
                "proxy",
                "ensure",
                "--port",
                &self.port.to_string(),
                "--quiet",
            ],
        )
    }
    fn record(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.home.path().join(".shift/proxy.pid")).unwrap())
            .unwrap()
    }
}

impl Drop for Managed {
    fn drop(&mut self) {
        let _ = cli(self.home.path(), &["proxy", "stop", "--quiet"]);
    }
}

#[test]
fn genuine_daemon_start_ensure_status_stop() {
    let proxy = Managed::new();
    let started = proxy.ensure();
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let record = proxy.record();
    assert_eq!(record["format_version"], 1);
    assert!(record["identity"]["pid"].as_u64().unwrap() > 1);
    assert!(!record["identity"]["started"].as_str().unwrap().is_empty());
    assert_eq!(record["port"], proxy.port);
    assert_eq!(
        fs::metadata(proxy.home.path().join(".shift/proxy.pid"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(proxy.ensure().status.success());
    assert_eq!(proxy.record(), record);
    let status = cli(
        proxy.home.path(),
        &["proxy", "status", "--port", &proxy.port.to_string()],
    );
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout)
        .contains(&format!("pid {}", record["identity"]["pid"])));
    assert!(cli(proxy.home.path(), &["proxy", "stop", "--quiet"])
        .status
        .success());
    assert!(!proxy.home.path().join(".shift/proxy.pid").exists());
    assert!(proxy.home.path().join(".shift/proxy.lock").exists());
    assert!(TcpListener::bind(("127.0.0.1", proxy.port)).is_ok());
}

#[test]
fn concurrent_ensure_commands_share_one_identity_record() {
    let proxy = Managed::new();
    let children: Vec<_> = (0..6)
        .map(|_| {
            Command::new(env!("CARGO_BIN_EXE_shift-ai"))
                .env("HOME", proxy.home.path())
                .args([
                    "proxy",
                    "ensure",
                    "--port",
                    &proxy.port.to_string(),
                    "--quiet",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in children {
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let original = proxy.record();
    for _ in 0..3 {
        assert!(proxy.ensure().status.success());
        assert_eq!(proxy.record(), original);
    }
    let body = ureq::get(format!("http://127.0.0.1:{}/health", proxy.port))
        .call()
        .unwrap()
        .into_body()
        .read_to_string()
        .unwrap();
    let health: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(health["pid"], original["identity"]["pid"]);
}

#[test]
fn starting_another_port_does_not_orphan_managed_daemon() {
    let proxy = Managed::new();
    assert!(proxy.ensure().status.success());
    let record = proxy.record();
    let result = cli(
        proxy.home.path(),
        &[
            "proxy",
            "start",
            "--port",
            &free_port().to_string(),
            "--quiet",
        ],
    );
    assert!(!result.status.success());
    assert_eq!(proxy.record(), record);
}

#[test]
fn failed_startup_leaves_no_ownership_record() {
    let home = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let result = cli(
        home.path(),
        &["proxy", "start", "--port", &port.to_string(), "--quiet"],
    );
    assert!(!result.status.success());
    assert!(!home.path().join(".shift/proxy.pid").exists());
    assert!(TcpListener::bind(("127.0.0.1", port)).is_err());
}

#[test]
fn fifo_state_cannot_block_lifecycle_commands_or_hold_the_lock() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::time::{Duration, Instant};

    fn bounded(home: &Path, args: &[&str]) -> Option<Output> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_shift-ai"))
            .env("HOME", home)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if child.try_wait().unwrap().is_some() {
                return Some(child.wait_with_output().unwrap());
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    let proxy = Managed::new();
    assert!(proxy.ensure().status.success());
    let path = proxy.home.path().join(".shift/proxy.pid");
    let original = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    let cpath = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    let ensured = bounded(
        proxy.home.path(),
        &[
            "proxy",
            "ensure",
            "--port",
            &proxy.port.to_string(),
            "--quiet",
        ],
    );
    let stopped = bounded(proxy.home.path(), &["proxy", "stop", "--quiet"]);
    // Restore the known identity before any assertion so fixture cleanup stays safe.
    fs::remove_file(&path).unwrap();
    fs::write(&path, original).unwrap();
    assert!(ensured
        .expect("healthy ensure blocked on FIFO state")
        .status
        .success());
    assert!(!stopped
        .expect("stop blocked on FIFO state")
        .status
        .success());
    assert!(
        proxy.ensure().status.success(),
        "failed read retained the lifecycle lock"
    );
}

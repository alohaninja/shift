//! Native proxy lifecycle. All cooperating commands serialize through a stable
//! lock file; proxy.pid contains a versioned process-identity record, not just a PID.

mod process;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEFAULT_PORT: u16 = 8787;
const DEFAULT_MODE: &str = "balanced";
const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(100);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const HEALTH_SERVICE_ID: &str = "@shift-preflight/runtime proxy";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    format_version: u8,
    identity: process::Identity,
    port: u16,
}

impl Record {
    fn validate(&self) -> Result<()> {
        process::validate_pid(self.identity.pid)?;
        if self.format_version != 1
            || self.port == 0
            || self.identity.started.is_empty()
            || !self.identity.executable.is_absolute()
        {
            bail!("invalid or unsupported proxy identity record; no signal sent");
        }
        Ok(())
    }
}

/// Keep this file open for the entire operation. Never unlink proxy.lock: that
/// would let another command lock a different inode while this one is active.
struct Manager {
    dir: PathBuf,
    _lock: File,
}

impl Manager {
    fn open() -> Result<Self> {
        let home = std::env::var_os("HOME").context("HOME not set")?;
        Self::at(PathBuf::from(home).join(".shift"))
    }

    fn at(dir: PathBuf) -> Result<Self> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)?;
        if !fs::symlink_metadata(&dir)?.is_dir() {
            bail!("proxy state directory must not be a symlink");
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(dir.join("proxy.lock"))
            .context("cannot open proxy lifecycle lock")?;
        let deadline = Instant::now() + LOCK_TIMEOUT;
        loop {
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                break;
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::WouldBlock {
                return Err(error).context("cannot lock proxy lifecycle");
            }
            if Instant::now() >= deadline {
                bail!("another proxy lifecycle operation is in progress; try again");
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Ok(Self { dir, _lock: lock })
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("proxy.pid")
    }

    fn read(&self) -> Result<Option<Record>> {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.state_path())
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("cannot read proxy state; no signal sent"),
        };
        if !file.metadata()?.is_file() {
            bail!("proxy state must be a regular file")
        }
        let mut contents = String::new();
        file.take(4097).read_to_string(&mut contents)?;
        if contents.len() > 4096 {
            bail!("proxy state is too large; no signal sent")
        }
        if contents.trim().parse::<u32>().is_ok() {
            bail!("legacy PID-only proxy state cannot prove ownership; no signal sent. Stop the old daemon via its service manager or after manually verifying its identity, then remove {}", self.state_path().display());
        }
        let record: Record = serde_json::from_str(&contents)
            .context("invalid proxy identity record; no signal sent")?;
        record.validate()?;
        Ok(Some(record))
    }

    fn write(&self, record: &Record) -> Result<()> {
        record.validate()?;
        let mut file = tempfile::Builder::new()
            .prefix(".proxy-state-")
            .tempfile_in(&self.dir)?;
        serde_json::to_writer(&mut file, record)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(self.state_path())
            .context("cannot publish proxy identity")?;
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        match fs::remove_file(self.state_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("cannot remove stale proxy state"),
        }
    }

    fn start(&self, port: u16, mode: &str, quiet: bool) -> Result<()> {
        if port == 0 {
            bail!("proxy port must be nonzero")
        }
        let _: shift_preflight::DriveMode = mode.parse().map_err(|e: String| anyhow::anyhow!(e))?;
        let record = self.read();
        // A live recorded daemon on another port must not lose its ownership record.
        if let Ok(Some(record)) = &record {
            if process::inspect(record.identity.pid)?.as_ref() == Some(&record.identity)
                && record.port != port
            {
                bail!("a managed proxy is already running on port {}; stop it before starting another", record.port);
            }
        }
        // Foreground/LaunchAgent instances and legacy installations may be healthy
        // without a usable ownership record. Reuse them, but never adopt their PID.
        if health(port).is_some() {
            if !quiet {
                eprintln!("[shift] proxy already running on port {port}");
            }
            return Ok(());
        }
        if let Some(record) = record? {
            if process::inspect(record.identity.pid)?.as_ref() == Some(&record.identity) {
                bail!("recorded proxy PID {} is still running but unhealthy; refusing to overwrite its state", record.identity.pid);
            }
            self.clear()?;
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.dir.join("proxy.log"))?;
        let executable = std::env::current_exe().context("cannot locate shift-ai executable")?;
        let mut child = Command::new(&executable)
            .args([
                "proxy",
                "start",
                "--foreground",
                "--port",
                &port.to_string(),
                "--mode",
                mode,
                "--quiet",
            ])
            .stdout(log.try_clone()?)
            .stderr(log)
            .stdin(Stdio::null())
            .spawn()?;
        let result = (|| -> Result<()> {
            let identity = process::inspect(child.id())?
                .context("proxy exited before its identity could be recorded")?;
            if identity.executable != fs::canonicalize(&executable)? {
                bail!("spawned process executable does not match shift-ai");
            }
            self.write(&Record {
                format_version: 1,
                identity,
                port,
            })?;
            let deadline = Instant::now() + STARTUP_TIMEOUT;
            loop {
                if let Some(status) = child.try_wait()? {
                    bail!(
                        "proxy exited during startup ({status}); see {}",
                        self.dir.join("proxy.log").display()
                    )
                }
                // The listener must belong to THIS child, not a competing process.
                if health(port).and_then(|h| h.pid) == Some(child.id())
                    && child.try_wait()?.is_none()
                {
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    bail!(
                        "proxy did not become healthy; see {}",
                        self.dir.join("proxy.log").display()
                    )
                }
                std::thread::sleep(HEALTH_POLL_INTERVAL);
            }
        })();
        if let Err(error) = result {
            // This is our unreaped Child, so its PID cannot have been recycled.
            if child.try_wait()?.is_none() {
                child
                    .kill()
                    .context("startup failed and child termination failed; state retained")?;
                child
                    .wait()
                    .context("cannot confirm failed startup child exited; state retained")?;
            }
            self.clear()?;
            return Err(error);
        }
        if !quiet {
            eprintln!(
                "[shift] proxy started on port {port} (pid {}, mode: {mode})",
                child.id()
            );
        }
        Ok(())
    }

    fn stop(&self, quiet: bool) -> Result<()> {
        let Some(record) = self.read()? else {
            if !quiet {
                eprintln!(
                    "[shift] no managed proxy state (foreground services use their own supervisor)"
                );
            }
            return Ok(());
        };
        let Some(handle) = process::Handle::open(&record.identity)? else {
            self.clear()?;
            if !quiet {
                eprintln!("[shift] removed stale proxy state; no matching daemon to signal");
            }
            return Ok(());
        };
        handle.signal(libc::SIGTERM)?;
        if !wait_for_exit(&handle, Duration::from_secs(3))? {
            // signal() rechecks birth/executable identity before escalation too.
            handle.signal(libc::SIGKILL)?;
            if !wait_for_exit(&handle, Duration::from_secs(1))? {
                bail!("proxy termination could not be confirmed; ownership record retained");
            }
        }
        self.clear()?;
        if !quiet {
            eprintln!("[shift] proxy stopped (pid {})", record.identity.pid);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Health {
    service: String,
    pid: Option<u32>,
}

fn health(port: u16) -> Option<Health> {
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(HEALTH_TIMEOUT))
            .build(),
    );
    let mut response = agent
        .get(&format!("http://localhost:{port}/health"))
        .call()
        .ok()?;
    if response.status().as_u16() != 200 {
        return None;
    }
    let body: Health = serde_json::from_str(&response.body_mut().read_to_string().ok()?).ok()?;
    (body.service == HEALTH_SERVICE_ID).then_some(body)
}

fn wait_for_exit(handle: &process::Handle, timeout: Duration) -> Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        if !handle.matches()? {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(HEALTH_POLL_INTERVAL);
    }
}

pub fn start(port: Option<u16>, mode: Option<&str>, quiet: bool) -> Result<()> {
    Manager::open()?.start(
        port.unwrap_or(DEFAULT_PORT),
        mode.unwrap_or(DEFAULT_MODE),
        quiet,
    )
}

pub fn ensure(port: Option<u16>, mode: Option<&str>, quiet: bool) -> Result<()> {
    start(port, mode, quiet)
}

pub fn stop(quiet: bool) -> Result<()> {
    Manager::open()?.stop(quiet)
}

pub fn status(port: Option<u16>) -> Result<()> {
    let port = port.unwrap_or(DEFAULT_PORT);
    let manager = Manager::open()?;
    let record = manager.read();
    if let Some(health) = health(port) {
        match record {
            Ok(Some(record))
                if record.port == port
                    && health.pid == Some(record.identity.pid)
                    && process::inspect(record.identity.pid)?.as_ref()
                        == Some(&record.identity) =>
            {
                println!("running (pid {}, port {port})", record.identity.pid);
            }
            _ => println!("running (port {port}, ownership unverified)"),
        }
        return Ok(());
    }
    if let Some(record) = record? {
        if process::inspect(record.identity.pid)?.as_ref() == Some(&record.identity) {
            println!(
                "running (pid {}, port {}, unhealthy)",
                record.identity.pid, record.port
            );
        } else {
            manager.clear()?;
            println!("stopped (stale identity record)");
        }
    } else {
        println!("stopped");
    }
    Ok(())
}

/// Foreground instances remain owned by their LaunchAgent/systemd supervisor.
pub fn run_foreground(port: u16, mode: &str, verbose: bool) -> Result<()> {
    let drive_mode: shift_preflight::DriveMode =
        mode.parse().map_err(|e: String| anyhow::anyhow!(e))?;
    let config = shift_proxy::ProxyConfig {
        port,
        mode: drive_mode,
        verbose,
        providers: shift_proxy::state::ProviderUrls::default(),
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;
    rt.block_on(shift_proxy::start_server(config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Child;

    struct Victim(Child);
    impl Victim {
        fn spawn() -> Self {
            Self(Command::new("sleep").arg("30").spawn().unwrap())
        }
        fn identity(&self) -> process::Identity {
            process::inspect(self.0.id()).unwrap().unwrap()
        }
    }
    impl Drop for Victim {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn record(identity: process::Identity) -> Record {
        Record {
            format_version: 1,
            identity,
            port: 12345,
        }
    }

    #[test]
    fn stale_birth_time_or_executable_never_signals_victim() {
        for change_birth in [true, false] {
            let home = tempfile::tempdir().unwrap();
            let manager = Manager::at(home.path().join("state")).unwrap();
            let mut victim = Victim::spawn();
            let mut identity = victim.identity();
            if change_birth {
                identity.started.push_str(":stale");
            } else {
                identity.executable = PathBuf::from("/a/different/executable");
            }
            manager.write(&record(identity)).unwrap();
            manager.stop(true).unwrap();
            assert!(victim.0.try_wait().unwrap().is_none());
            assert!(!manager.state_path().exists());
        }
    }

    #[test]
    fn failed_signal_is_reported_and_does_not_kill_process() {
        let mut victim = Victim::spawn();
        let handle = process::Handle::open(&victim.identity()).unwrap().unwrap();
        assert!(handle.signal(-1).is_err());
        assert!(victim.0.try_wait().unwrap().is_none());
    }

    #[test]
    fn failed_stop_keeps_record() {
        let home = tempfile::tempdir().unwrap();
        let manager = Manager::at(home.path().join("state")).unwrap();
        manager
            .write(&record(
                process::inspect(std::process::id()).unwrap().unwrap(),
            ))
            .unwrap();
        assert!(manager.stop(true).is_err()); // Self-signaling is always refused.
        assert!(manager.state_path().exists());
    }

    #[test]
    fn verified_process_ignoring_term_is_killed_and_confirmed_gone() {
        use std::os::unix::process::ExitStatusExt;
        let home = tempfile::tempdir().unwrap();
        let manager = Manager::at(home.path().join("state")).unwrap();
        let mut victim = Victim(
            Command::new("sh")
                .args(["-c", "trap '' TERM; exec sleep 30"])
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if victim.identity().executable.file_name().unwrap() == "sleep" {
                break;
            }
            assert!(Instant::now() < deadline, "fixture did not exec sleep");
            std::thread::sleep(Duration::from_millis(10));
        }
        manager.write(&record(victim.identity())).unwrap();
        manager.stop(true).unwrap();
        assert_eq!(victim.0.wait().unwrap().signal(), Some(libc::SIGKILL));
        assert!(!manager.state_path().exists());
    }

    #[test]
    fn executable_change_after_term_prevents_kill_escalation() {
        let home = tempfile::tempdir().unwrap();
        let manager = Manager::at(home.path().join("state")).unwrap();
        let ready = home.path().join("ready");
        let mut victim = Victim(
            Command::new("sh")
                .args([
                    "-c",
                    "trap 'exec sleep 30' TERM; printf ready > \"$1\"; while :; do :; done",
                    "fixture",
                ])
                .arg(&ready)
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "fixture did not install signal handler"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        manager.write(&record(victim.identity())).unwrap();
        manager.stop(true).unwrap();
        assert!(
            victim.0.try_wait().unwrap().is_none(),
            "replacement executable was killed"
        );
        assert_eq!(victim.identity().executable.file_name().unwrap(), "sleep");
    }

    #[test]
    fn special_pids_and_malformed_records_fail_closed() {
        let home = tempfile::tempdir().unwrap();
        let manager = Manager::at(home.path().join("state")).unwrap();
        let identity = process::inspect(std::process::id()).unwrap().unwrap();
        for pid in [0, 1, i32::MAX as u32 + 1, u32::MAX] {
            let mut invalid = record(identity.clone());
            invalid.identity.pid = pid;
            let bytes = serde_json::to_vec(&invalid).unwrap();
            fs::write(manager.state_path(), &bytes).unwrap();
            assert!(manager.stop(true).is_err());
            assert_eq!(fs::read(manager.state_path()).unwrap(), bytes);
        }
        for invalid in ["{broken".to_string(), "x".repeat(4097)] {
            fs::write(manager.state_path(), &invalid).unwrap();
            assert!(manager.stop(true).is_err());
            assert_eq!(fs::read_to_string(manager.state_path()).unwrap(), invalid);
        }
    }

    #[test]
    fn symlinked_state_is_not_read_or_overwritten() {
        let home = tempfile::tempdir().unwrap();
        let manager = Manager::at(home.path().join("state")).unwrap();
        let outside = home.path().join("outside");
        fs::write(&outside, "keep").unwrap();
        std::os::unix::fs::symlink(&outside, manager.state_path()).unwrap();
        assert!(manager.stop(true).is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "keep");
    }
}

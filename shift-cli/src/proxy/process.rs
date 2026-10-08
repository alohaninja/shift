//! Process-instance identity, rather than PID existence, authorizes signals.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    pub pid: u32,
    pub started: String,
    pub executable: PathBuf,
    pub uid: u32,
}

pub(super) fn validate_pid(pid: u32) -> Result<libc::pid_t> {
    if pid <= 1 || pid > i32::MAX as u32 {
        bail!("refusing unsafe PID {pid}");
    }
    Ok(pid as libc::pid_t)
}

fn disappeared(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::ESRCH | libc::ENOENT))
}

#[cfg(target_os = "macos")]
fn bsd_info(pid: libc::pid_t) -> Result<Option<libc::proc_bsdinfo>> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut _,
            size,
        )
    };
    if read != size {
        let error = io::Error::last_os_error();
        if disappeared(&error) {
            return Ok(None);
        }
        bail!("cannot inspect PID {pid}: {error} (read {read}/{size} bytes)");
    }
    if info.pbi_status == libc::SZOMB {
        return Ok(None);
    }
    Ok(Some(info))
}

#[cfg(target_os = "macos")]
pub(super) fn inspect(pid: u32) -> Result<Option<Identity>> {
    use std::ffi::{CStr, OsStr};
    use std::os::unix::ffi::OsStrExt;

    let native_pid = validate_pid(pid)?;
    let Some(before) = bsd_info(native_pid)? else {
        return Ok(None);
    };
    let mut path = vec![0 as libc::c_char; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let read =
        unsafe { libc::proc_pidpath(native_pid, path.as_mut_ptr().cast(), path.len() as u32) };
    if read <= 0 {
        let error = io::Error::last_os_error();
        if disappeared(&error) {
            return Ok(None);
        }
        return Err(error).context("cannot inspect daemon executable");
    }
    let Some(after) = bsd_info(native_pid)? else {
        return Ok(None);
    };
    if (
        before.pbi_start_tvsec,
        before.pbi_start_tvusec,
        before.pbi_uid,
    ) != (after.pbi_start_tvsec, after.pbi_start_tvusec, after.pbi_uid)
    {
        bail!("process identity changed while inspecting PID {pid}");
    }
    let bytes = unsafe { CStr::from_ptr(path.as_ptr()) }.to_bytes();
    Ok(Some(Identity {
        pid,
        started: format!("{}:{}", after.pbi_start_tvsec, after.pbi_start_tvusec),
        executable: PathBuf::from(OsStr::from_bytes(bytes)),
        uid: after.pbi_uid,
    }))
}

// Kept platform-independent so the tricky /proc parsing is tested on macOS too.
#[cfg(any(target_os = "linux", test))]
fn stat_start(stat: &str) -> Result<Option<&str>> {
    // comm may itself contain spaces, parentheses, and newlines.
    let (_, fields) = stat.rsplit_once(") ").context("invalid /proc stat")?;
    let fields: Vec<_> = fields.split_whitespace().collect();
    if matches!(fields.first(), Some(&"Z" | &"X")) {
        return Ok(None);
    }
    let start = *fields.get(19).context("missing /proc start time")?;
    start.parse::<u64>().context("invalid /proc start time")?;
    Ok(Some(start))
}

#[cfg(target_os = "linux")]
pub(super) fn inspect(pid: u32) -> Result<Option<Identity>> {
    use std::ffi::OsStr;
    use std::fs;
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};

    validate_pid(pid)?;
    let dir = PathBuf::from(format!("/proc/{pid}"));
    let read_stat = || -> Result<Option<String>> {
        match fs::read_to_string(dir.join("stat")) {
            Ok(stat) => Ok(Some(stat)),
            Err(error) if disappeared(&error) => Ok(None),
            Err(error) => Err(error).context("cannot read process identity"),
        }
    };
    let Some(before) = read_stat()? else {
        return Ok(None);
    };
    let Some(start) = stat_start(&before)? else {
        return Ok(None);
    };
    let executable = match fs::read_link(dir.join("exe")) {
        Ok(path) => path,
        Err(error) if disappeared(&error) => return Ok(None),
        Err(error) => return Err(error).context("cannot inspect daemon executable"),
    };
    let uid = match fs::metadata(&dir) {
        Ok(meta) => meta.uid(),
        Err(error) if disappeared(&error) => return Ok(None),
        Err(error) => return Err(error).context("cannot inspect daemon owner"),
    };
    let Some(after) = read_stat()? else {
        return Ok(None);
    };
    if stat_start(&after)? != Some(start) {
        bail!("process identity changed while inspecting PID {pid}");
    }
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    let bytes = executable.as_os_str().as_bytes();
    // Package managers may replace the binary while its old instance still runs.
    let executable = PathBuf::from(OsStr::from_bytes(
        bytes.strip_suffix(b" (deleted)").unwrap_or(bytes),
    ));
    Ok(Some(Identity {
        pid,
        started: format!("{}:{start}", boot.trim()),
        executable,
        uid,
    }))
}

pub(super) struct Handle {
    expected: Identity,
    #[cfg(target_os = "linux")]
    fd: std::os::fd::OwnedFd,
}

impl Handle {
    pub fn open(expected: &Identity) -> Result<Option<Self>> {
        let pid = validate_pid(expected.pid)?;
        if expected.pid == std::process::id() {
            bail!("refusing to signal this CLI process")
        }
        #[cfg(target_os = "linux")]
        let fd = {
            use std::os::fd::FromRawFd;
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0u32) };
            if fd < 0 {
                let error = io::Error::last_os_error();
                if disappeared(&error) {
                    return Ok(None);
                }
                return Err(error)
                    .context("cannot open process handle (Linux 5.3+ required); no signal sent");
            }
            unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as i32) }
        };
        #[cfg(not(target_os = "linux"))]
        let _ = pid;
        // Inspect AFTER opening the pidfd: a recycled PID must never authorize it.
        if inspect(expected.pid)?.as_ref() != Some(expected) {
            return Ok(None);
        }
        Ok(Some(Self {
            expected: expected.clone(),
            #[cfg(target_os = "linux")]
            fd,
        }))
    }

    pub fn matches(&self) -> Result<bool> {
        Ok(inspect(self.expected.pid)?.as_ref() == Some(&self.expected))
    }

    pub fn signal(&self, signal: libc::c_int) -> Result<()> {
        if !self.matches()? {
            return Ok(());
        }
        #[cfg(target_os = "linux")]
        let result = {
            use std::os::fd::AsRawFd;
            unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    self.fd.as_raw_fd(),
                    signal,
                    std::ptr::null::<libc::siginfo_t>(),
                    0u32,
                )
            }
        };
        // macOS has no pidfd equivalent. Revalidate immediately before each signal;
        // unlike Linux this cannot make the check-and-signal pair atomic.
        #[cfg(target_os = "macos")]
        let result =
            unsafe { libc::kill(validate_pid(self.expected.pid)?, signal) } as libc::c_long;
        if result < 0 {
            let error = io::Error::last_os_error();
            if !disappeared(&error) {
                return Err(error).context("failed to signal verified proxy");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_special_and_wrapping_pids() {
        for pid in [0, 1, i32::MAX as u32 + 1, u32::MAX] {
            assert!(validate_pid(pid).is_err());
        }
        assert_eq!(validate_pid(2).unwrap(), 2);
        assert_eq!(validate_pid(i32::MAX as u32).unwrap(), i32::MAX);
    }

    #[test]
    fn current_process_identity_is_stable() {
        let first = inspect(std::process::id()).unwrap().unwrap();
        assert_eq!(first, inspect(std::process::id()).unwrap().unwrap());
        assert_eq!(
            first.executable,
            std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap()
        );
        assert!(!first.started.is_empty());
    }

    #[test]
    fn parses_proc_start_time_after_command_with_parentheses() {
        let stat = "42 (command with ) spaces\nand (parens)) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 987654 0 0";
        assert_eq!(stat_start(stat).unwrap(), Some("987654"));
        assert_eq!(stat_start("42 (zombie) Z").unwrap(), None);
        assert!(stat_start("42 (short) S 1").is_err());
    }
}

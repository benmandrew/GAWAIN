//! The one place the crate spawns external tools.
//!
//! [`run`] starts a command in its own session, with an address-space and a
//! CPU-time limit and a parent-death signal, feeds it stdin, and captures
//! stdout and stderr. On a timeout it kills the child's whole process group,
//! since tools such as `pan` and `gcc` start children of their own. A
//! timeout or a death by signal is a [`RunError`], which a back end reports
//! as `Verdict::Unknown`, never as a verdict. See "Persistent processes,
//! parallelism and resource limits" in `docs/dev/oracle-performance.md`.

use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Resource limits for one child.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Wall-clock time before the process group is killed.
    pub timeout: Duration,
    /// `RLIMIT_AS` in bytes. It caps virtual memory, not resident memory, so
    /// leave headroom over the resident size the tool is expected to reach.
    pub memory_bytes: u64,
}

/// What a child that exited by itself wrote.
#[derive(Debug)]
pub struct Output {
    /// The exit status. It is never a death by signal.
    pub status: ExitStatus,
    /// Standard output, lossily decoded as UTF-8.
    pub stdout: String,
    /// Standard error, lossily decoded as UTF-8.
    pub stderr: String,
}

/// Why a run gave no output.
#[derive(Debug)]
pub enum RunError {
    /// The command could not be started.
    Spawn(io::Error),
    /// The child ran past [`Limits::timeout`] and its group was killed.
    TimedOut(Duration),
    /// The child died of this signal, for example `SIGXCPU` from the CPU
    /// limit or `SIGKILL` from an out-of-memory killer.
    Signalled(i32),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Spawn(e) => write!(f, "cannot start the tool: {e}"),
            RunError::TimedOut(t) => write!(f, "timed out after {t:?}"),
            RunError::Signalled(s) => write!(f, "killed by signal {s}"),
        }
    }
}

impl std::error::Error for RunError {}

/// Runs `command` with `stdin` as its standard input under `limits`.
///
/// Whatever the outcome, every process left in the child's group is killed
/// before this returns.
pub fn run(command: &mut Command, stdin: &str, limits: Limits) -> Result<Output, RunError> {
    // RLIMIT_CPU has whole-second resolution; the wall-clock timeout is the
    // real bound and the CPU limit only a backstop if this process dies.
    let cpu_secs = limits.timeout.as_secs() + 1;
    let memory = limits.memory_bytes;
    // SAFETY: the closure runs between fork and exec and calls only
    // async-signal-safe functions (setsid, setrlimit, prctl).
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            set_limit(libc::RLIMIT_AS, memory)?;
            set_limit(libc::RLIMIT_CPU, cpu_secs)?;
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(RunError::Spawn)?;
    // The child is its own session leader, so its pid is its group id.
    let group = child.id() as libc::pid_t;

    let mut input = child.stdin.take().expect("stdin is piped");
    let text = stdin.to_owned();
    // A child that exits without reading closes the pipe; that is not an
    // error here, so the write result is dropped.
    let writer = thread::spawn(move || drop(input.write_all(text.as_bytes())));
    let stdout = reader(child.stdout.take().expect("stdout is piped"));
    let stderr = reader(child.stderr.take().expect("stderr is piped"));

    let status = wait(&mut child, limits.timeout);
    // SAFETY: killpg has no memory-safety preconditions. ESRCH (no process
    // left in the group) is the common case and is ignored.
    unsafe {
        libc::killpg(group, libc::SIGKILL);
    }
    if status.is_none() {
        let _ = child.wait();
    }
    let _ = writer.join();
    let stdout = stdout.join().expect("reader thread");
    let stderr = stderr.join().expect("reader thread");

    let status = status.ok_or(RunError::TimedOut(limits.timeout))?;
    if let Some(signal) = status.signal() {
        return Err(RunError::Signalled(signal));
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn set_limit(resource: libc::__rlimit_resource_t, value: u64) -> io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    // SAFETY: `limit` is a valid rlimit for the duration of the call.
    if unsafe { libc::setrlimit(resource, &limit) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn reader(mut pipe: impl Read + Send + 'static) -> JoinHandle<String> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// Waits for the child until `timeout`, and returns `None` if it is still
/// running then. The poll interval grows from 1 ms to 20 ms, so a 30 ms tool
/// run is not rounded up much.
fn wait(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    let mut pause = Duration::from_millis(1);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        thread::sleep(pause.min(deadline - now));
        pause = (pause * 2).min(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits {
        timeout: Duration::from_secs(10),
        memory_bytes: 256 << 20,
    };

    fn sh(script: &str, stdin: &str, limits: Limits) -> Result<Output, RunError> {
        run(Command::new("sh").args(["-c", script]), stdin, limits)
    }

    fn short(timeout: Duration) -> Limits {
        Limits { timeout, ..LIMITS }
    }

    #[test]
    fn stdin_reaches_the_child() {
        let output = sh("tr a-z A-Z; echo oops >&2", "hello\n", LIMITS).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, "HELLO\n");
        assert_eq!(output.stderr, "oops\n");
    }

    #[test]
    fn a_nonzero_exit_is_output() {
        let output = sh("exit 3", "", LIMITS).unwrap();
        assert_eq!(output.status.code(), Some(3));
    }

    #[test]
    fn a_timeout_kills_a_sleep() {
        let start = Instant::now();
        let error = sh("exec sleep 30", "", short(Duration::from_millis(200))).unwrap_err();
        assert!(matches!(error, RunError::TimedOut(_)), "{error}");
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_death_by_signal_is_an_error() {
        let error = sh("kill -9 $$", "", LIMITS).unwrap_err();
        assert!(matches!(error, RunError::Signalled(9)), "{error}");
    }

    #[test]
    fn a_memory_hog_is_stopped() {
        // `tail` keeps the last line, and /dev/zero has no newline, so it
        // grows until the address-space limit stops it.
        let start = Instant::now();
        let result = sh("head -c 2000000000 /dev/zero | tail -n 1", "", LIMITS);
        match result {
            Ok(output) => assert!(!output.status.success(), "{output:?}"),
            Err(error) => assert!(matches!(error, RunError::Signalled(_)), "{error}"),
        }
        assert!(start.elapsed() < LIMITS.timeout);
    }

    #[test]
    fn no_grandchild_survives_a_timeout() {
        let dir = std::env::temp_dir().join(format!("gawain-process-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pid_file = dir.join("grandchild.pid");
        let script = format!("sleep 30 & echo $! > '{}'; wait", pid_file.display());
        let error = sh(&script, "", short(Duration::from_millis(300))).unwrap_err();
        assert!(matches!(error, RunError::TimedOut(_)), "{error}");
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        let stat = format!("/proc/{}/stat", pid.trim());
        // The orphaned sleep is reaped by its new parent, not by us, so give
        // that a moment; a zombie counts as dead.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match std::fs::read_to_string(&stat) {
                Err(_) => break,
                Ok(text) if text.contains(") Z ") => break,
                Ok(text) => {
                    assert!(Instant::now() < deadline, "grandchild alive: {text}");
                    thread::sleep(Duration::from_millis(20));
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

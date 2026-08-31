use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_STDERR_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub stdin: Vec<u8>,
    pub timeout: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
    pub cancellation: Option<Arc<AtomicBool>>,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>, args: Vec<String>, max_stdout_bytes: usize) -> Self {
        Self {
            program: program.into(),
            args,
            stdin: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
            max_stdout_bytes,
            max_stderr_bytes: MAX_STDERR_BYTES,
            cancellation: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub output_limited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerError {
    Missing,
    Spawn,
    Io,
}

pub trait CommandRunner: Send + Sync {
    fn run(&self, spec: &CommandSpec) -> Result<ToolOutput, RunnerError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DirectCommandRunner;

impl CommandRunner for DirectCommandRunner {
    fn run(&self, spec: &CommandSpec) -> Result<ToolOutput, RunnerError> {
        if spec.program.is_empty() || spec.program.chars().any(char::is_control) {
            return Err(RunnerError::Spawn);
        }
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                RunnerError::Missing
            } else {
                RunnerError::Spawn
            }
        })?;
        if let Some(mut stdin) = child.stdin.take() {
            if stdin.write_all(&spec.stdin).is_err() {
                terminate_group(&mut child);
                return Err(RunnerError::Io);
            }
            drop(stdin);
        }
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate_group(&mut child);
                return Err(RunnerError::Io);
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                terminate_group(&mut child);
                return Err(RunnerError::Io);
            }
        };
        let stdout_limit = spec.max_stdout_bytes;
        let stderr_limit = spec.max_stderr_bytes;
        let stdout_reader = thread::spawn(move || read_bounded(stdout, stdout_limit));
        let stderr_reader = thread::spawn(move || read_bounded(stderr, stderr_limit));
        let deadline = Instant::now() + spec.timeout;
        let mut timed_out = false;
        let mut cancelled = false;
        loop {
            if spec
                .cancellation
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Relaxed))
            {
                cancelled = true;
                terminate_group(&mut child);
                break;
            }
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {}
                Err(_) => {
                    terminate_group(&mut child);
                    return Err(RunnerError::Io);
                }
            }
            if Instant::now() >= deadline {
                timed_out = true;
                terminate_group(&mut child);
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let status = child.wait().map_err(|_| RunnerError::Io)?;
        let (stdout, stdout_limited) = stdout_reader.join().map_err(|_| RunnerError::Io)?;
        let (stderr, stderr_limited) = stderr_reader.join().map_err(|_| RunnerError::Io)?;
        Ok(ToolOutput {
            exit_code: status.code(),
            stdout,
            stderr,
            timed_out,
            cancelled,
            output_limited: stdout_limited || stderr_limited,
        })
    }
}

fn read_bounded<R: Read>(mut reader: R, limit: usize) -> (Vec<u8>, bool) {
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut limited = false;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                let remaining = limit.saturating_sub(output.len());
                output.extend_from_slice(&buffer[..read.min(remaining)]);
                if read > remaining {
                    limited = true;
                }
            }
            Err(_) => {
                limited = true;
                break;
            }
        }
    }
    (output, limited)
}

fn terminate_group(child: &mut Child) {
    #[cfg(unix)]
    {
        if let Ok(pid) = i32::try_from(child.id()) {
            // The child is placed in its own process group in pre_exec.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
    let _ = child.kill();
}

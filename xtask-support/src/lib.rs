use anyhow::{Context, Result, anyhow, bail};
use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::thread;

pub const BLUE: &str = "\x1b[0;34m";
pub const GREEN: &str = "\x1b[0;32m";
pub const YELLOW: &str = "\x1b[1;33m";
pub const BOLD: &str = "\x1b[1m";
pub const RESET: &str = "\x1b[0m";

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask-support has a repo root parent")
        .to_path_buf()
}

#[derive(Clone)]
pub struct TaskContext {
    pub repo_root: PathBuf,
    rtk_available: bool,
}

impl Default for TaskContext {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskContext {
    pub fn new() -> Self {
        let rtk_available = match validated_release_rtk_available() {
            Ok(available) => available,
            Err(err) => {
                eprintln!(
                    "{YELLOW}warning:{RESET} {err}. Falling back to direct command execution."
                );
                false
            }
        };
        Self {
            repo_root: repo_root(),
            rtk_available,
        }
    }

    /// Builds a context rooted at an arbitrary directory without probing for `rtk`.
    ///
    /// Used by tests that exercise repo-relative path handling inside a temporary
    /// directory instead of the real checkout.
    pub fn with_repo_root(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            rtk_available: false,
        }
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.repo_root.join(relative)
    }

    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        Command::new(program)
    }

    pub fn command_in(&self, program: impl AsRef<OsStr>, cwd: &Path) -> Command {
        let mut command = Command::new(program);
        command.current_dir(cwd);
        command
    }

    pub fn release_command(&self, program: impl AsRef<OsStr>) -> Command {
        self.release_command_impl(program, None)
    }

    pub fn release_command_in(&self, program: impl AsRef<OsStr>, cwd: &Path) -> Command {
        self.release_command_impl(program, Some(cwd))
    }

    fn release_command_impl(&self, program: impl AsRef<OsStr>, cwd: Option<&Path>) -> Command {
        let program = program.as_ref();
        let mut command = if self.rtk_available {
            let mut command = Command::new("rtk");
            command.arg(program);
            command
        } else {
            Command::new(program)
        };
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        command
    }
}

pub fn step(message: impl AsRef<str>) {
    println!("\n{BLUE}{BOLD}▶  {}{RESET}", message.as_ref());
}

pub fn ok(message: impl AsRef<str>) {
    println!("   {GREEN}✓  {}{RESET}", message.as_ref());
}

pub fn warn(message: impl AsRef<str>) {
    eprintln!("   {YELLOW}⚠  {}{RESET}", message.as_ref());
}

pub fn prefixed_step(prefix: &str, message: impl AsRef<str>) {
    println!("{prefix}{BLUE}{BOLD}▶  {}{RESET}", message.as_ref());
}

pub fn prefixed_ok(prefix: &str, message: impl AsRef<str>) {
    println!("{prefix}{GREEN}✓  {}{RESET}", message.as_ref());
}

pub fn require_command(command: &str) -> Result<()> {
    if command_available(command)? {
        Ok(())
    } else {
        bail!("{command} is required")
    }
}

pub fn command_available(command: &str) -> Result<bool> {
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {command} >/dev/null 2>&1"))
        .status()?;
    Ok(status.success())
}

pub fn validated_release_rtk_available() -> Result<bool> {
    if !command_available("rtk")? {
        return Ok(false);
    }

    let mut help_command = Command::new("rtk");
    help_command.arg("--help");
    let help = run_capture(&mut help_command).context("failed to inspect `rtk --help`")?;

    let looks_like_rust_token_killer = help.contains("filter and summarize system outputs")
        && help.contains("gain")
        && help.contains("proxy");

    if looks_like_rust_token_killer {
        Ok(true)
    } else {
        Err(anyhow!(
            "`rtk` was found on PATH, but it does not appear to be Rust Token Killer"
        ))
    }
}

pub fn run_status(command: &mut Command) -> Result<ExitStatus> {
    Ok(command.status()?)
}

pub fn run_checked(command: &mut Command) -> Result<()> {
    let debug = format!("{command:?}");
    let status = run_status(command)?;
    if !status.success() {
        bail!("command failed: {debug}");
    }
    Ok(())
}

pub fn run_capture(command: &mut Command) -> Result<String> {
    let debug = format!("{command:?}");
    let output = command.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("command failed: {debug}\n{stderr}");
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn run_streaming(command: &mut Command, prefix: &'static str) -> Result<()> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_thread = stdout.map(|stdout| pipe_reader(stdout, prefix, false));
    let stderr_thread = stderr.map(|stderr| pipe_reader_stderr(stderr, prefix));
    let status = child.wait()?;
    if let Some(thread) = stdout_thread {
        let _ = thread.join();
    }
    if let Some(thread) = stderr_thread {
        let _ = thread.join();
    }
    if !status.success() {
        bail!("command failed: {command:?}");
    }
    Ok(())
}

fn pipe_reader(stream: ChildStdout, prefix: &'static str, stderr: bool) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines().map_while(Result::ok) {
            if stderr {
                eprintln!("{prefix}{line}");
            } else {
                println!("{prefix}{line}");
            }
        }
    })
}

fn pipe_reader_stderr(stream: ChildStderr, prefix: &'static str) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines().map_while(Result::ok) {
            eprintln!("{prefix}{line}");
        }
    })
}

//! Launches operator-authored scripts.
//!
//! One runner serves every caller that executes a script: it resolves the
//! interpreter from the entry point, materializes inline content that needs a
//! file, runs the process in its own process group, enforces the timeout by
//! killing that whole group, and keeps the tail of each captured stream.

use chrono::{DateTime, Utc};
use scryer_domain::ScriptLanguage;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

/// How much of each captured stream a run keeps. The store compresses the
/// tails (zstd) so a larger window costs little at rest.
pub const OUTPUT_TAIL_BYTES: usize = 32 * 1024;

/// Directory under the materialization root that holds Go's build cache when
/// the invocation does not choose one. The service account's home directory
/// is not reliably writable, and `go run` refuses to start without a cache.
const GO_CACHE_DIR_NAME: &str = ".gocache";

/// Directory under the materialization root used as Go's temp directory when
/// neither the invocation nor the server environment chooses one.
const GO_TMP_DIR_NAME: &str = ".gotmp";

/// How long to keep reading output after the script has exited or been
/// killed. Only a background process still holding the pipe takes longer.
const DRAIN_BOUND: Duration = Duration::from_secs(5);

/// Interpreters the operator pinned. `None` falls back to the conventional
/// command name, resolved through `PATH`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InterpreterConfig {
    pub python: Option<PathBuf>,
    pub powershell: Option<PathBuf>,
    pub batch: Option<PathBuf>,
    pub go: Option<PathBuf>,
}

/// Where the script comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptSource {
    /// Script text stored with its definition.
    Inline {
        content: String,
        language: ScriptLanguage,
    },
    /// Absolute path to a script on the server.
    File { path: String },
}

/// Everything needed to launch one script run.
#[derive(Debug, Clone)]
pub struct ScriptInvocation {
    /// Script identity; names the directory inline content is materialized in.
    pub script_id: String,
    pub source: ScriptSource,
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub timeout: Duration,
    /// Whether stdout and stderr are captured; otherwise they are discarded.
    pub capture_output: bool,
    pub interpreters: InterpreterConfig,
    /// Root directory for materialized inline scripts.
    pub materialize_root: PathBuf,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptOutcome {
    /// The process exited on its own.
    Exited { code: Option<i32>, success: bool },
    /// The process outlived its timeout and its process group was killed.
    TimedOut,
    /// The process could not be started.
    SpawnFailed { reason: String },
    /// Waiting for the process failed.
    IoError { reason: String },
}

/// Result of one script run.
#[derive(Debug, Clone)]
pub struct ScriptExecution {
    pub outcome: ScriptOutcome,
    /// Last [`OUTPUT_TAIL_BYTES`] of stdout, when output was captured.
    pub stdout_tail: Option<String>,
    /// Last [`OUTPUT_TAIL_BYTES`] of stderr, when output was captured.
    pub stderr_tail: Option<String>,
    pub duration_ms: i64,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}

/// Program, arguments and extra environment that launch a script.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LaunchPlan {
    program: PathBuf,
    args: Vec<OsString>,
    env: Vec<(String, String)>,
}

/// Run a script to completion, timeout, or launch failure. Never panics on a
/// failed launch; every failure is reported through [`ScriptOutcome`].
pub async fn run_script(invocation: ScriptInvocation) -> ScriptExecution {
    let started_at = Utc::now();
    let start_instant = Instant::now();

    let plan = match plan_launch(&invocation) {
        Ok(plan) => plan,
        Err(reason) => {
            return finished(
                ScriptOutcome::SpawnFailed { reason },
                started_at,
                start_instant,
            );
        }
    };

    let mut cmd = Command::new(&plan.program);
    cmd.args(&plan.args);
    #[cfg(not(windows))]
    {
        // Create a new process group so we can kill the entire tree on timeout,
        // not just the direct child process.
        unsafe {
            cmd.pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            });
        }
    }
    for (name, value) in invocation.env.iter().chain(plan.env.iter()) {
        cmd.env(name, value);
    }
    cmd.current_dir(&invocation.cwd);

    if invocation.capture_output {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            return finished(
                ScriptOutcome::SpawnFailed {
                    reason: format!("{}: {err}", plan.program.display()),
                },
                started_at,
                start_instant,
            );
        }
    };

    if !invocation.capture_output {
        let outcome = match tokio::time::timeout(invocation.timeout, child.wait()).await {
            Ok(Ok(status)) => ScriptOutcome::Exited {
                code: status.code(),
                success: status.success(),
            },
            Ok(Err(err)) => ScriptOutcome::IoError {
                reason: err.to_string(),
            },
            Err(_elapsed) => {
                kill_process_group(&mut child).await;
                ScriptOutcome::TimedOut
            }
        };
        return finished(outcome, started_at, start_instant);
    }

    // Capture stdout/stderr (last OUTPUT_TAIL_BYTES of each).
    let stdout_drain = OutputDrain::spawn(child.stdout.take());
    let stderr_drain = OutputDrain::spawn(child.stderr.take());

    let outcome = match tokio::time::timeout(invocation.timeout, child.wait()).await {
        Ok(Ok(status)) => ScriptOutcome::Exited {
            code: status.code(),
            success: status.success(),
        },
        Ok(Err(err)) => {
            stdout_drain.abort();
            stderr_drain.abort();
            return finished(
                ScriptOutcome::IoError {
                    reason: err.to_string(),
                },
                started_at,
                start_instant,
            );
        }
        Err(_elapsed) => {
            kill_process_group(&mut child).await;
            ScriptOutcome::TimedOut
        }
    };
    let duration_ms = start_instant.elapsed().as_millis() as i64;
    let completed_at = Utc::now();
    let stdout_bytes = stdout_drain.finish().await;
    let stderr_bytes = stderr_drain.finish().await;
    ScriptExecution {
        outcome,
        stdout_tail: Some(last_bytes_utf8(&stdout_bytes, OUTPUT_TAIL_BYTES)),
        stderr_tail: Some(last_bytes_utf8(&stderr_bytes, OUTPUT_TAIL_BYTES)),
        duration_ms,
        started_at,
        completed_at,
    }
}

/// Reads one output pipe in the background, keeping only its last
/// [`OUTPUT_TAIL_BYTES`].
struct OutputDrain {
    tail: Arc<Mutex<Vec<u8>>>,
    task: tokio::task::JoinHandle<()>,
}

impl OutputDrain {
    fn spawn<R>(pipe: Option<R>) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        let tail = Arc::new(Mutex::new(Vec::new()));
        let writer = Arc::clone(&tail);
        let task = tokio::spawn(async move {
            let Some(mut pipe) = pipe else {
                return;
            };
            let mut chunk = [0_u8; 8192];
            loop {
                match pipe.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        let mut tail = writer.lock().unwrap_or_else(|err| err.into_inner());
                        tail.extend_from_slice(&chunk[..count]);
                        if tail.len() > OUTPUT_TAIL_BYTES {
                            let excess = tail.len() - OUTPUT_TAIL_BYTES;
                            tail.drain(..excess);
                        }
                    }
                }
            }
        });
        Self { tail, task }
    }

    /// Wait briefly for the pipe to close, then take what was read. A
    /// background process that inherited the pipe can hold it open long after
    /// the script itself is gone, so the read is abandoned after
    /// [`DRAIN_BOUND`] instead of waiting for it.
    async fn finish(mut self) -> Vec<u8> {
        if tokio::time::timeout(DRAIN_BOUND, &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
        }
        std::mem::take(&mut *self.tail.lock().unwrap_or_else(|err| err.into_inner()))
    }

    fn abort(self) {
        self.task.abort();
    }
}

fn finished(
    outcome: ScriptOutcome,
    started_at: DateTime<Utc>,
    start_instant: Instant,
) -> ScriptExecution {
    ScriptExecution {
        outcome,
        stdout_tail: None,
        stderr_tail: None,
        duration_ms: start_instant.elapsed().as_millis() as i64,
        started_at,
        completed_at: Utc::now(),
    }
}

async fn kill_process_group(child: &mut tokio::process::Child) {
    // Kill the entire process group (shell + children), not just the shell.
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    let _ = child.kill().await;
}

fn plan_launch(invocation: &ScriptInvocation) -> Result<LaunchPlan, String> {
    match &invocation.source {
        ScriptSource::Inline { content, language } => {
            if *language == ScriptLanguage::Shell && !content.starts_with("#!") {
                return Ok(inline_shell_plan(content));
            }
            let path = materialize_inline_script(
                &invocation.materialize_root,
                &invocation.script_id,
                content,
                *language,
            )
            .map_err(|err| format!("failed to materialize inline script: {err}"))?;
            plan_materialized(&path, content, *language, invocation)
        }
        ScriptSource::File { path } => {
            let path = validate_file_script_path(path)?;
            plan_file_script(Path::new(path), invocation)
        }
    }
}

#[cfg(windows)]
fn inline_shell_plan(script_content: &str) -> LaunchPlan {
    LaunchPlan {
        program: PathBuf::from("cmd"),
        args: vec![OsString::from("/C"), OsString::from(script_content)],
        env: Vec::new(),
    }
}

#[cfg(not(windows))]
fn inline_shell_plan(script_content: &str) -> LaunchPlan {
    LaunchPlan {
        program: PathBuf::from("sh"),
        args: vec![OsString::from("-c"), OsString::from(script_content)],
        env: Vec::new(),
    }
}

pub(crate) fn validate_file_script_path(script_content: &str) -> Result<&str, String> {
    let path = script_content.trim();
    if path.is_empty() {
        return Err("file script path is empty".to_string());
    }
    if !Path::new(path).is_absolute() {
        return Err("file script path must be absolute".to_string());
    }
    Ok(path)
}

/// Launch plan for materialized inline content. The file is always handed to
/// an interpreter and never executed itself, so the scripts directory needs no
/// execute permission and a freshly written file is never exec'd.
///
/// A shebang naming an interpreter that exists on this host wins. Otherwise
/// the language picks the interpreter, and shell falls back to `sh`, so a
/// `#!/bin/bash` script still runs on a host that only has a POSIX shell.
fn plan_materialized(
    file: &Path,
    content: &str,
    language: ScriptLanguage,
    invocation: &ScriptInvocation,
) -> Result<LaunchPlan, String> {
    if let Some((program, mut args)) = parse_shebang_text(content)
        && program.is_absolute()
        && program.is_file()
    {
        args.push(file.as_os_str().to_owned());
        return Ok(LaunchPlan {
            program,
            args,
            env: Vec::new(),
        });
    }
    match language {
        ScriptLanguage::Shell => Ok(LaunchPlan {
            program: PathBuf::from("sh"),
            args: vec![file.as_os_str().to_owned()],
            env: Vec::new(),
        }),
        ScriptLanguage::Python
        | ScriptLanguage::PowerShell
        | ScriptLanguage::Batch
        | ScriptLanguage::Go => plan_by_extension(file, language.file_extension(), invocation)?
            .ok_or_else(|| {
                format!(
                    "no interpreter is defined for .{} scripts",
                    language.file_extension()
                )
            }),
    }
}

/// Launch plan for a script file on the server. An executable file runs
/// directly, whatever its extension. Otherwise the extension picks the
/// interpreter, then a shebang does, and as a last resort the file is
/// executed directly so the OS reports why it cannot run.
fn plan_file_script(
    entrypoint: &Path,
    invocation: &ScriptInvocation,
) -> Result<LaunchPlan, String> {
    let direct = LaunchPlan {
        program: entrypoint.to_path_buf(),
        args: Vec::new(),
        env: Vec::new(),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // A file that cannot be inspected is launched directly too, so the
        // spawn reports the failure exactly as it always has.
        match std::fs::metadata(entrypoint) {
            Ok(metadata) if metadata.permissions().mode() & 0o111 == 0 => {}
            _ => return Ok(direct),
        }
    }
    if let Some(plan) =
        plan_by_extension(entrypoint, &entrypoint_extension(entrypoint), invocation)?
    {
        return Ok(plan);
    }
    #[cfg(unix)]
    {
        if let Ok(Some((program, mut args))) = parse_shebang_file(entrypoint) {
            args.push(entrypoint.as_os_str().to_owned());
            return Ok(LaunchPlan {
                program,
                args,
                env: Vec::new(),
            });
        }
    }
    Ok(direct)
}

fn entrypoint_extension(entrypoint: &Path) -> String {
    entrypoint
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Interpreter for a known script extension, or `None` for any other.
fn plan_by_extension(
    entrypoint: &Path,
    extension: &str,
    invocation: &ScriptInvocation,
) -> Result<Option<LaunchPlan>, String> {
    let interpreters = &invocation.interpreters;
    let file = entrypoint.as_os_str().to_owned();
    let plan = match extension {
        "py" => LaunchPlan {
            program: interpreters
                .python
                .clone()
                .unwrap_or_else(|| PathBuf::from("python3")),
            args: vec![file],
            env: Vec::new(),
        },
        "ps1" => LaunchPlan {
            program: interpreters
                .powershell
                .clone()
                .unwrap_or_else(|| PathBuf::from("pwsh")),
            args: vec![
                OsString::from("-NoProfile"),
                OsString::from("-NonInteractive"),
                OsString::from("-File"),
                file,
            ],
            env: Vec::new(),
        },
        // No `/S`: with it cmd strips the first and last quote of the command
        // line, which breaks a script path containing spaces.
        "bat" | "cmd" => LaunchPlan {
            program: interpreters
                .batch
                .clone()
                .or_else(|| std::env::var_os("COMSPEC").map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("cmd.exe")),
            args: vec![OsString::from("/D"), OsString::from("/C"), file],
            env: Vec::new(),
        },
        "go" => LaunchPlan {
            program: interpreters
                .go
                .clone()
                .unwrap_or_else(|| PathBuf::from("go")),
            args: vec![OsString::from("run"), file],
            env: go_environment(invocation, |name| std::env::var_os(name))?,
        },
        _ => return Ok(None),
    };
    Ok(Some(plan))
}

#[cfg(unix)]
fn parse_shebang_file(entrypoint: &Path) -> io::Result<Option<(PathBuf, Vec<OsString>)>> {
    use std::io::Read;

    let mut file = std::fs::File::open(entrypoint)?;
    let mut bytes = [0_u8; 4096];
    let count = file.read(&mut bytes)?;
    Ok(parse_shebang_text(&String::from_utf8_lossy(
        &bytes[..count],
    )))
}

/// Interpreter and arguments from a `#!` first line. Arguments are split on
/// whitespace here rather than by the kernel, so `#!/usr/bin/env bash -e`
/// passes both words.
fn parse_shebang_text(content: &str) -> Option<(PathBuf, Vec<OsString>)> {
    let line = content.lines().next()?.strip_prefix("#!")?;
    let mut words = line.split_ascii_whitespace();
    let program = words.next()?;
    Some((PathBuf::from(program), words.map(OsString::from).collect()))
}

/// Go needs a writable build cache and temp directory; the service account's
/// home is not reliably writable. A choice already made in the invocation or
/// the server's own environment is kept.
fn go_environment(
    invocation: &ScriptInvocation,
    process_env: impl Fn(&str) -> Option<OsString>,
) -> Result<Vec<(String, String)>, String> {
    let is_set = |name: &str| {
        invocation.env.iter().any(|(key, _)| key == name)
            || process_env(name).is_some_and(|value| !value.is_empty())
    };
    let mut env = Vec::new();
    for (name, dir_name, purpose) in [
        ("GOCACHE", GO_CACHE_DIR_NAME, "build cache"),
        ("GOTMPDIR", GO_TMP_DIR_NAME, "temp directory"),
    ] {
        if is_set(name) {
            continue;
        }
        let dir = invocation.materialize_root.join(dir_name);
        create_private_dir_all(&dir)
            .map_err(|err| format!("failed to create Go {purpose} {}: {err}", dir.display()))?;
        env.push((name.to_string(), dir.to_string_lossy().into_owned()));
    }
    if !is_set("GOFLAGS") {
        env.push(("GOFLAGS".to_string(), "-mod=mod".to_string()));
    }
    Ok(env)
}

/// Write inline content to `<root>/<script_id>/<hash>.<ext>` unless that file
/// already exists. Files are content-addressed, so an edited script gets a new
/// file and the previous one is left in place.
fn materialize_inline_script(
    root: &Path,
    script_id: &str,
    content: &str,
    language: ScriptLanguage,
) -> io::Result<PathBuf> {
    validate_script_id(script_id)?;
    let dir = root.join(script_id);
    create_private_dir_all(&dir)?;
    let digest = blake3::hash(content.as_bytes()).to_hex();
    let path = dir.join(format!("{digest}.{}", language.file_extension()));
    if path.try_exists()? {
        return Ok(path);
    }

    // Write under a unique temporary name and rename into place, so a
    // concurrent run never executes a partially written file.
    let temp = dir.join(format!(
        ".{digest}.{}.{}.{}.tmp",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        MATERIALIZE_SEQUENCE.fetch_add(1, Ordering::Relaxed),
    ));
    write_new_private_file(&temp, content.as_bytes())?;
    match std::fs::rename(&temp, &path) {
        Ok(()) => Ok(path),
        // A concurrent run installed the same content first.
        Err(_) if path.try_exists().unwrap_or(false) => Ok(path),
        Err(err) => Err(err),
    }
}

static MATERIALIZE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn validate_script_id(script_id: &str) -> io::Result<()> {
    let valid = !script_id.is_empty()
        && !script_id.starts_with('.')
        && script_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("script id {script_id:?} cannot name a directory"),
        ))
    }
}

fn create_private_dir_all(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

fn write_new_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o700);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o700))?;
    }
    file.sync_all()
}

/// Return the last `max_bytes` of `buf` as a trimmed UTF-8 string.
pub(crate) fn last_bytes_utf8(buf: &[u8], max_bytes: usize) -> String {
    let slice = if buf.len() > max_bytes {
        &buf[buf.len() - max_bytes..]
    } else {
        buf
    };
    String::from_utf8_lossy(slice).trim().to_string()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Generous upper bound on any single run; a correct run finishes far
    /// sooner, so reaching it means the runner hung.
    const RUN_BOUND: Duration = Duration::from_secs(60);

    /// Fake interpreter that reports how it was invoked, one line per fact.
    const FAKE_INTERPRETER: &str = "#!/bin/sh\n\
        printf 'argv0=%s\\n' \"$0\"\n\
        for a in \"$@\"; do printf 'arg=%s\\n' \"$a\"; done\n\
        printf 'GOCACHE=%s\\n' \"${GOCACHE-}\"\n\
        printf 'GOTMPDIR=%s\\n' \"${GOTMPDIR-}\"\n\
        printf 'GOFLAGS=%s\\n' \"${GOFLAGS-}\"\n";

    struct Fixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
        bin: PathBuf,
        work: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().expect("tempdir");
            let root = temp.path().join("scripts");
            let bin = temp.path().join("bin");
            let work = temp.path().join("work");
            std::fs::create_dir_all(&bin).expect("bin dir");
            std::fs::create_dir_all(&work).expect("work dir");
            Self {
                _temp: temp,
                root,
                bin,
                work,
            }
        }

        fn fake_interpreter(&self, name: &str) -> PathBuf {
            let path = self.bin.join(name);
            write_file(&path, FAKE_INTERPRETER, 0o755);
            path
        }

        fn invocation(&self, source: ScriptSource) -> ScriptInvocation {
            ScriptInvocation {
                script_id: "script-alpha".to_string(),
                source,
                env: Vec::new(),
                cwd: self.work.clone(),
                timeout: Duration::from_secs(30),
                capture_output: true,
                interpreters: InterpreterConfig::default(),
                materialize_root: self.root.clone(),
            }
        }

        fn file_invocation(&self, path: &Path) -> ScriptInvocation {
            self.invocation(ScriptSource::File {
                path: path.to_string_lossy().into_owned(),
            })
        }
    }

    fn write_file(path: &Path, content: &str, mode: u32) {
        std::fs::write(path, content).expect("write file");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
    }

    async fn run(invocation: ScriptInvocation) -> ScriptExecution {
        tokio::time::timeout(RUN_BOUND, run_script(invocation))
            .await
            .expect("script run finished within its bound")
    }

    fn stdout_lines(execution: &ScriptExecution) -> Vec<String> {
        execution
            .stdout_tail
            .as_deref()
            .expect("stdout captured")
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn arg_lines(execution: &ScriptExecution) -> Vec<String> {
        stdout_lines(execution)
            .into_iter()
            .filter(|line| line.starts_with("arg="))
            .collect()
    }

    fn assert_succeeded(execution: &ScriptExecution) {
        assert_eq!(
            execution.outcome,
            ScriptOutcome::Exited {
                code: Some(0),
                success: true
            },
            "stderr: {:?}",
            execution.stderr_tail
        );
    }

    fn inline(content: &str, language: ScriptLanguage) -> ScriptSource {
        ScriptSource::Inline {
            content: content.to_string(),
            language,
        }
    }

    fn materialized_path(root: &Path, content: &str, extension: &str) -> PathBuf {
        root.join("script-alpha").join(format!(
            "{}.{extension}",
            blake3::hash(content.as_bytes()).to_hex()
        ))
    }

    fn file_count(dir: &Path) -> usize {
        std::fs::read_dir(dir).expect("read dir").count()
    }

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    }

    #[tokio::test]
    async fn inline_shell_without_shebang_runs_through_sh_c_and_materializes_nothing() {
        let fixture = Fixture::new();
        let content = "printf '%s' \"$0\"";
        let invocation = fixture.invocation(inline(content, ScriptLanguage::Shell));

        let plan = plan_launch(&invocation).expect("plan");
        assert_eq!(plan.program, PathBuf::from("sh"));
        assert_eq!(
            plan.args,
            vec![OsString::from("-c"), OsString::from(content)]
        );
        assert!(plan.env.is_empty());

        let execution = run(invocation).await;
        assert_succeeded(&execution);
        assert_eq!(execution.stdout_tail.as_deref(), Some("sh"));
        assert!(
            !fixture.root.exists(),
            "an inline shell script without a shebang is never written to disk"
        );
    }

    #[tokio::test]
    async fn inline_shell_with_shebang_is_materialized_once_and_reused() {
        let fixture = Fixture::new();
        let content = "#!/bin/sh\necho materialized-run\n";
        let expected = materialized_path(&fixture.root, content, "sh");

        let first = run(fixture.invocation(inline(content, ScriptLanguage::Shell))).await;
        assert_succeeded(&first);
        assert_eq!(first.stdout_tail.as_deref(), Some("materialized-run"));
        assert!(expected.is_file(), "materialized at {}", expected.display());
        let script_dir = fixture.root.join("script-alpha");
        assert_eq!(mode_of(&expected), 0o700);
        assert_eq!(mode_of(&script_dir), 0o700);

        let second = run(fixture.invocation(inline(content, ScriptLanguage::Shell))).await;
        assert_succeeded(&second);
        assert_eq!(second.stdout_tail.as_deref(), Some("materialized-run"));
        let plan =
            plan_launch(&fixture.invocation(inline(content, ScriptLanguage::Shell))).expect("plan");
        assert_eq!(plan.program, PathBuf::from("/bin/sh"));
        assert_eq!(plan.args, vec![expected.as_os_str().to_owned()]);
        assert_eq!(file_count(&script_dir), 1);
    }

    #[tokio::test]
    async fn materialized_inline_scripts_never_need_execute_permission() {
        let fixture = Fixture::new();
        let content = "#!/bin/sh\necho still-runs\n";
        let expected = materialized_path(&fixture.root, content, "sh");
        assert_succeeded(&run(fixture.invocation(inline(content, ScriptLanguage::Shell))).await);

        // Simulate a noexec data directory: the reused file cannot be exec'd,
        // and the run must not care.
        std::fs::set_permissions(&expected, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        let execution = run(fixture.invocation(inline(content, ScriptLanguage::Shell))).await;
        assert_succeeded(&execution);
        assert_eq!(execution.stdout_tail.as_deref(), Some("still-runs"));
    }

    #[tokio::test]
    async fn edited_inline_content_gets_a_new_file_and_keeps_the_old_one() {
        let fixture = Fixture::new();
        let original = "#!/bin/sh\necho first-edit\n";
        let edited = "#!/bin/sh\necho second-edit\n";

        assert_succeeded(&run(fixture.invocation(inline(original, ScriptLanguage::Shell))).await);
        assert_succeeded(&run(fixture.invocation(inline(edited, ScriptLanguage::Shell))).await);

        assert!(materialized_path(&fixture.root, original, "sh").is_file());
        assert!(materialized_path(&fixture.root, edited, "sh").is_file());
        assert_eq!(file_count(&fixture.root.join("script-alpha")), 2);
    }

    #[tokio::test]
    async fn inline_shell_with_a_missing_shebang_interpreter_runs_under_sh() {
        let fixture = Fixture::new();
        let content = "#!/nonexistent/bash\necho ok\n";
        let invocation = fixture.invocation(inline(content, ScriptLanguage::Shell));

        let plan = plan_launch(&invocation).expect("plan");
        assert_eq!(plan.program, PathBuf::from("sh"));
        let execution = run(invocation).await;
        assert_succeeded(&execution);
        assert_eq!(execution.stdout_tail.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn inline_shell_with_a_resolvable_shebang_runs_under_it_with_its_arguments() {
        let fixture = Fixture::new();
        let interpreter = fixture.fake_interpreter("fake-bash");
        let content = format!("#!{} -e --flag\necho unused\n", interpreter.display());

        let execution = run(fixture.invocation(inline(&content, ScriptLanguage::Shell))).await;
        assert_succeeded(&execution);
        let expected = materialized_path(&fixture.root, &content, "sh");
        assert_eq!(
            stdout_lines(&execution)[0],
            format!("argv0={}", interpreter.display())
        );
        assert_eq!(
            arg_lines(&execution),
            vec![
                "arg=-e".to_string(),
                "arg=--flag".to_string(),
                format!("arg={}", expected.display()),
            ]
        );
    }

    #[tokio::test]
    async fn inline_shell_env_shebang_passes_the_split_arguments_to_env() {
        let fixture = Fixture::new();
        let content = "#!/usr/bin/env sh\necho \"$0\"\n";
        let invocation = fixture.invocation(inline(content, ScriptLanguage::Shell));
        let expected = materialized_path(&fixture.root, content, "sh");

        let plan = plan_launch(&invocation).expect("plan");
        assert_eq!(plan.program, PathBuf::from("/usr/bin/env"));
        assert_eq!(
            plan.args,
            vec![OsString::from("sh"), expected.as_os_str().to_owned()]
        );
        let execution = run(invocation).await;
        assert_succeeded(&execution);
        assert_eq!(
            execution.stdout_tail.as_deref(),
            Some(expected.to_string_lossy().as_ref())
        );
    }

    #[tokio::test]
    async fn inline_python_is_materialized_and_dispatched_to_the_configured_python() {
        let fixture = Fixture::new();
        let python = fixture.fake_interpreter("fake-python");
        let content = "print('synthetic')\n";
        let mut invocation = fixture.invocation(inline(content, ScriptLanguage::Python));
        invocation.interpreters.python = Some(python.clone());

        let execution = run(invocation).await;
        assert_succeeded(&execution);
        let expected = materialized_path(&fixture.root, content, "py");
        assert!(expected.is_file());
        assert_eq!(
            stdout_lines(&execution)[0],
            format!("argv0={}", python.display())
        );
        assert_eq!(
            arg_lines(&execution),
            vec![format!("arg={}", expected.display())]
        );
    }

    #[tokio::test]
    async fn inline_python_prefers_a_resolvable_shebang_over_the_python_pin() {
        let fixture = Fixture::new();
        let pinned = fixture.fake_interpreter("fake-python");
        let shebang = fixture.fake_interpreter("fake-python-shebang");
        let content = format!("#!{}\nprint('synthetic')\n", shebang.display());
        let mut invocation = fixture.invocation(inline(&content, ScriptLanguage::Python));
        invocation.interpreters.python = Some(pinned);

        let execution = run(invocation).await;
        assert_succeeded(&execution);
        assert_eq!(
            stdout_lines(&execution)[0],
            format!("argv0={}", shebang.display())
        );
        assert_eq!(
            arg_lines(&execution),
            vec![format!(
                "arg={}",
                materialized_path(&fixture.root, &content, "py").display()
            )]
        );
    }

    #[tokio::test]
    async fn non_executable_powershell_file_gets_the_noninteractive_file_arguments() {
        let fixture = Fixture::new();
        let powershell = fixture.fake_interpreter("fake-pwsh");
        let script = fixture.work.join("Job.PS1");
        write_file(&script, "Write-Output 'synthetic'\n", 0o644);
        let mut invocation = fixture.file_invocation(&script);
        invocation.interpreters.powershell = Some(powershell.clone());

        let execution = run(invocation).await;
        assert_succeeded(&execution);
        assert_eq!(
            stdout_lines(&execution)[0],
            format!("argv0={}", powershell.display())
        );
        assert_eq!(
            arg_lines(&execution),
            vec![
                "arg=-NoProfile".to_string(),
                "arg=-NonInteractive".to_string(),
                "arg=-File".to_string(),
                format!("arg={}", script.display()),
            ]
        );
    }

    #[tokio::test]
    async fn non_executable_python_file_dispatches_to_the_configured_python() {
        let fixture = Fixture::new();
        let python = fixture.fake_interpreter("fake-python");
        let script = fixture.work.join("job.py");
        write_file(&script, "print('synthetic')\n", 0o644);
        let mut invocation = fixture.file_invocation(&script);
        invocation.interpreters.python = Some(python.clone());

        let execution = run(invocation).await;
        assert_succeeded(&execution);
        assert_eq!(
            stdout_lines(&execution)[0],
            format!("argv0={}", python.display())
        );
        assert_eq!(
            arg_lines(&execution),
            vec![format!("arg={}", script.display())]
        );
    }

    #[tokio::test]
    async fn executable_python_file_is_executed_directly() {
        let fixture = Fixture::new();
        let pinned = fixture.fake_interpreter("fake-python");
        let shebang = fixture.fake_interpreter("fake-python-shebang");
        let script = fixture.work.join("job.py");
        write_file(
            &script,
            &format!("#!{}\nprint('synthetic')\n", shebang.display()),
            0o755,
        );
        let mut invocation = fixture.file_invocation(&script);
        invocation.interpreters.python = Some(pinned);

        let plan = plan_launch(&invocation).expect("plan");
        assert_eq!(plan.program, script);
        assert!(plan.args.is_empty());
        let execution = run(invocation).await;
        assert_succeeded(&execution);
        assert_eq!(
            stdout_lines(&execution)[0],
            format!("argv0={}", shebang.display())
        );
    }

    #[tokio::test]
    async fn non_executable_go_file_runs_through_go_run() {
        let fixture = Fixture::new();
        let go = fixture.fake_interpreter("fake-go");
        let script = fixture.work.join("job.go");
        write_file(&script, "package main\nfunc main() {}\n", 0o644);
        let mut invocation = fixture.file_invocation(&script);
        invocation.interpreters.go = Some(go.clone());

        let execution = run(invocation).await;
        assert_succeeded(&execution);
        let lines = stdout_lines(&execution);
        assert_eq!(lines[0], format!("argv0={}", go.display()));
        assert_eq!(
            arg_lines(&execution),
            vec!["arg=run".to_string(), format!("arg={}", script.display())]
        );
        // The server's own Go environment, when present, wins over the
        // defaults under the scripts root.
        let expected = |name: &str, default: String| match std::env::var_os(name) {
            Some(value) if !value.is_empty() => value.to_string_lossy().into_owned(),
            _ => default,
        };
        let cache = fixture.root.join(GO_CACHE_DIR_NAME);
        let tmp = fixture.root.join(GO_TMP_DIR_NAME);
        assert!(lines.contains(&format!(
            "GOCACHE={}",
            expected("GOCACHE", cache.display().to_string())
        )));
        assert!(lines.contains(&format!(
            "GOTMPDIR={}",
            expected("GOTMPDIR", tmp.display().to_string())
        )));
        assert!(lines.contains(&format!(
            "GOFLAGS={}",
            expected("GOFLAGS", "-mod=mod".to_string())
        )));
    }

    #[test]
    fn go_environment_defaults_under_the_root_and_keeps_existing_choices() {
        let fixture = Fixture::new();
        let invocation = fixture.invocation(ScriptSource::File {
            path: "/unused/job.go".to_string(),
        });

        let defaults = go_environment(&invocation, |_| None).expect("go env");
        let cache = fixture.root.join(GO_CACHE_DIR_NAME);
        let tmp = fixture.root.join(GO_TMP_DIR_NAME);
        assert_eq!(
            defaults,
            vec![
                ("GOCACHE".to_string(), cache.to_string_lossy().into_owned()),
                ("GOTMPDIR".to_string(), tmp.to_string_lossy().into_owned()),
                ("GOFLAGS".to_string(), "-mod=mod".to_string()),
            ]
        );
        assert!(cache.is_dir());
        assert!(tmp.is_dir());

        let from_process = go_environment(&invocation, |name| {
            matches!(name, "GOCACHE" | "GOFLAGS").then(|| OsString::from("/srv/synthetic"))
        })
        .expect("go env");
        assert_eq!(
            from_process,
            vec![("GOTMPDIR".to_string(), tmp.to_string_lossy().into_owned())]
        );

        let mut chosen = invocation.clone();
        chosen.env = vec![
            ("GOCACHE".to_string(), "/srv/cache".to_string()),
            ("GOTMPDIR".to_string(), "/srv/tmp".to_string()),
            ("GOFLAGS".to_string(), "-mod=vendor".to_string()),
        ];
        assert!(
            go_environment(&chosen, |_| None)
                .expect("go env")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn non_executable_cmd_file_dispatches_to_the_configured_batch_interpreter() {
        let fixture = Fixture::new();
        let batch = fixture.fake_interpreter("fake-cmd");
        let script = fixture.work.join("job.cmd");
        write_file(&script, "@echo synthetic\r\n", 0o644);
        let mut invocation = fixture.file_invocation(&script);
        invocation.interpreters.batch = Some(batch.clone());

        let execution = run(invocation).await;
        assert_succeeded(&execution);
        assert_eq!(
            stdout_lines(&execution)[0],
            format!("argv0={}", batch.display())
        );
        assert_eq!(
            arg_lines(&execution),
            vec![
                "arg=/D".to_string(),
                "arg=/C".to_string(),
                format!("arg={}", script.display()),
            ]
        );
    }

    #[tokio::test]
    async fn non_executable_file_with_shebang_runs_through_its_interpreter() {
        let fixture = Fixture::new();
        let interpreter = fixture.fake_interpreter("fake-shebang");
        let script = fixture.work.join("job-task");
        write_file(
            &script,
            &format!("#!{} --flag value\nbody\n", interpreter.display()),
            0o644,
        );

        let execution = run(fixture.file_invocation(&script)).await;
        assert_succeeded(&execution);
        assert_eq!(
            stdout_lines(&execution)[0],
            format!("argv0={}", interpreter.display())
        );
        assert_eq!(
            arg_lines(&execution),
            vec![
                "arg=--flag".to_string(),
                "arg=value".to_string(),
                format!("arg={}", script.display()),
            ]
        );
    }

    #[tokio::test]
    async fn executable_file_without_known_extension_is_executed_directly() {
        let fixture = Fixture::new();
        let script = fixture.work.join("job-direct");
        write_file(&script, "#!/bin/sh\necho direct-run\n", 0o755);
        let invocation = fixture.file_invocation(&script);

        let plan = plan_launch(&invocation).expect("plan");
        assert_eq!(plan.program, script);
        assert!(plan.args.is_empty());
        let execution = run(invocation).await;
        assert_succeeded(&execution);
        assert_eq!(execution.stdout_tail.as_deref(), Some("direct-run"));
    }

    #[tokio::test]
    async fn file_script_launch_failures_are_reported_not_raised() {
        let fixture = Fixture::new();

        let relative = run(fixture.invocation(ScriptSource::File {
            path: "relative/job.sh".to_string(),
        }))
        .await;
        assert_eq!(
            relative.outcome,
            ScriptOutcome::SpawnFailed {
                reason: "file script path must be absolute".to_string()
            }
        );

        let missing_path = fixture.work.join("missing-job");
        let missing = run(fixture.file_invocation(&missing_path)).await;
        match &missing.outcome {
            ScriptOutcome::SpawnFailed { reason } => assert!(
                reason.starts_with(&format!("{}: ", missing_path.display())),
                "spawn errors name the program: {reason}"
            ),
            other => panic!("expected a spawn failure, got {other:?}"),
        }
        assert_eq!(missing.stdout_tail, None);
        assert_eq!(missing.stderr_tail, None);
    }

    #[tokio::test]
    async fn unsafe_script_ids_fail_materialization_without_writing() {
        let fixture = Fixture::new();
        for script_id in ["../escape", ".hidden", "nested/id", ""] {
            let mut invocation = fixture.invocation(inline("print(1)\n", ScriptLanguage::Python));
            invocation.script_id = script_id.to_string();

            let execution = run(invocation).await;
            assert!(
                matches!(
                    execution.outcome,
                    ScriptOutcome::SpawnFailed { ref reason }
                        if reason.starts_with("failed to materialize inline script")
                ),
                "{script_id:?} must be rejected"
            );
        }
        assert!(!fixture.root.exists());
    }

    #[tokio::test]
    async fn timeout_kills_the_whole_process_group() {
        let fixture = Fixture::new();
        // The background child keeps the output pipes open and would outlive
        // the drain bound, so a prompt TimedOut shows the group was killed.
        let mut invocation = fixture.invocation(inline("sleep 300 & wait", ScriptLanguage::Shell));
        invocation.timeout = Duration::from_secs(1);

        let execution = run(invocation).await;
        assert_eq!(execution.outcome, ScriptOutcome::TimedOut);
        assert_eq!(execution.stdout_tail.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn a_background_process_holding_the_pipes_cannot_hang_the_run() {
        let fixture = Fixture::new();
        let invocation = fixture.invocation(inline(
            "sleep 300 &\necho \"$!\"\necho done",
            ScriptLanguage::Shell,
        ));

        let execution = run(invocation).await;
        let lines = stdout_lines(&execution);
        if let Some(pid) = lines.first().and_then(|line| line.parse::<i32>().ok()) {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        assert_succeeded(&execution);
        assert_eq!(lines.last().map(String::as_str), Some("done"));
    }

    #[tokio::test]
    async fn output_is_captured_only_when_requested() {
        let fixture = Fixture::new();
        let content = "echo synthetic-out; echo synthetic-err >&2; exit 3";

        let mut quiet = fixture.invocation(inline(content, ScriptLanguage::Shell));
        quiet.capture_output = false;
        let quiet = run(quiet).await;
        assert_eq!(
            quiet.outcome,
            ScriptOutcome::Exited {
                code: Some(3),
                success: false
            }
        );
        assert_eq!(quiet.stdout_tail, None);
        assert_eq!(quiet.stderr_tail, None);

        let captured = run(fixture.invocation(inline(content, ScriptLanguage::Shell))).await;
        assert_eq!(
            captured.outcome,
            ScriptOutcome::Exited {
                code: Some(3),
                success: false
            }
        );
        assert_eq!(captured.stdout_tail.as_deref(), Some("synthetic-out"));
        assert_eq!(captured.stderr_tail.as_deref(), Some("synthetic-err"));
    }

    #[tokio::test]
    async fn captured_output_keeps_only_the_last_bytes() {
        let fixture = Fixture::new();
        let content = "i=0; while [ $i -lt 2000 ]; do printf 'line-%05d-padding-padding\\n' $i; i=$((i+1)); done";

        let execution = run(fixture.invocation(inline(content, ScriptLanguage::Shell))).await;
        assert_succeeded(&execution);
        let tail = execution.stdout_tail.expect("stdout captured");
        assert!(tail.len() <= OUTPUT_TAIL_BYTES);
        assert!(tail.len() > OUTPUT_TAIL_BYTES - 64);
        assert!(tail.ends_with("line-01999-padding-padding"));
        assert!(!tail.contains("line-00000-"));
    }

    #[test]
    fn tails_keep_the_last_bytes() {
        let buf = vec![b'a'; OUTPUT_TAIL_BYTES + 10];
        assert_eq!(
            last_bytes_utf8(&buf, OUTPUT_TAIL_BYTES).len(),
            OUTPUT_TAIL_BYTES
        );
        assert_eq!(last_bytes_utf8(b"  padded \n", OUTPUT_TAIL_BYTES), "padded");
    }
}

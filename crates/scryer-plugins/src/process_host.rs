use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

use scryer_plugin_sdk::host::{PluginProcessExecRequest, PluginProcessExecResponse};

use crate::types::{
    PluginDescriptor, PluginError, PluginErrorCode, PluginResult, ProviderDescriptor,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const OUTPUT_READER_JOIN_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_STDIN_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 512 * 1024;
const MAX_ARGS: usize = 128;
const MAX_ENV_VARS: usize = 256;

#[derive(Clone)]
pub(crate) struct ProcessHost {
    state: Arc<Mutex<ProcessHostState>>,
}

impl ProcessHost {
    pub(crate) fn disabled() -> Self {
        Self {
            state: Arc::new(Mutex::new(ProcessHostState::new(Vec::new()))),
        }
    }

    pub(crate) fn from_descriptor(
        descriptor: &PluginDescriptor,
        config_json: Option<&str>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(ProcessHostState::new(resolve_allowed_commands(
                descriptor,
                config_json,
            )))),
        }
    }

    /// Spawn one allowlisted command on behalf of a host-call guest.
    ///
    /// Every caller lands on [`ProcessHostState::execute`], so the allowlist
    /// check, the environment sanitizing, the stdin cap and the timeout are
    /// enforced once. The outer `Err` is a poisoned host lock — a host fault,
    /// not an answer about the command.
    pub(crate) fn exec(
        &self,
        request: PluginProcessExecRequest,
    ) -> Result<PluginResult<PluginProcessExecResponse>, String> {
        let inner = ProcessExecRequest {
            command: request.command,
            args: request.args,
            env: request.env,
            working_directory: request.cwd,
            // Re-encoding rather than shortcutting past `decode_stdin` is
            // deliberate: the 64 KiB stdin cap lives there, so the host-call
            // door cannot enforce a different one by accident.
            stdin_base64: Some(STANDARD.encode(request.stdin)),
            timeout_ms: request.timeout_ms,
        };
        Ok(match self.execute(inner)? {
            Ok(response) => PluginResult::Ok(PluginProcessExecResponse {
                // `PluginProcessExecResponse` has no `timed_out` field and no
                // optional status, so a killed run — the timeout, or a signal —
                // reports -1 with its captured output intact. That preserves
                // the decision a guest makes (a non-zero status is a failed
                // run) rather than discarding stdout and stderr to signal the
                // kill some other way.
                exit_code: response.status_code.unwrap_or(-1),
                stdout: decode_output(&response.stdout_base64),
                stderr: decode_output(&response.stderr_base64),
            }),
            Err(error) => PluginResult::Err(process_plugin_error(&error)),
        })
    }

    fn execute(
        &self,
        request: ProcessExecRequest,
    ) -> Result<Result<ProcessExecResponse, ProcessError>, String> {
        let state = self
            .state
            .lock()
            .map_err(|error| format!("process state lock poisoned: {error}"))?;
        Ok(state.execute(request))
    }

    /// Number of commands this host is allowed to spawn. A `disabled()` host (what
    /// non-first-party/Unverified plugins receive) always reports `0`, so any
    /// `scryer_process_exec` call resolves to PermissionDenied.
    #[cfg(test)]
    pub(crate) fn allowed_command_count(&self) -> usize {
        self.state
            .lock()
            .expect("process host state lock")
            .allowed_commands
            .len()
    }
}

#[derive(Debug)]
struct ProcessHostState {
    allowed_commands: Vec<String>,
}

impl ProcessHostState {
    fn new(allowed_commands: Vec<String>) -> Self {
        Self { allowed_commands }
    }

    fn execute(&self, request: ProcessExecRequest) -> Result<ProcessExecResponse, ProcessError> {
        validate_request(&request)?;
        if !self.command_allowed(&request.command) {
            return Err(process_error(
                ProcessErrorCode::PermissionDenied,
                format!("process permission denied for {}", request.command),
            ));
        }

        let stdin = decode_stdin(request.stdin_base64.as_deref())?;
        let timeout = request
            .timeout_ms
            .filter(|value| *value > 0)
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_TIMEOUT)
            .min(MAX_TIMEOUT);

        let mut command = Command::new(&request.command);
        command
            .args(&request.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        apply_sanitized_env(&mut command, &request.env);
        if let Some(working_directory) = request
            .working_directory
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            command.current_dir(working_directory);
        }

        let mut child = command.spawn().map_err(|error| {
            process_error(
                ProcessErrorCode::SpawnFailed,
                format!("failed to start {}: {error}", request.command),
            )
        })?;

        if let Some(mut child_stdin) = child.stdin.take() {
            let input = stdin.clone();
            thread::spawn(move || {
                let _ = child_stdin.write_all(&input);
            });
        }

        let stdout_handle = child
            .stdout
            .take()
            .map(|stdout| thread::spawn(move || read_limited(stdout)));
        let stderr_handle = child
            .stderr
            .take()
            .map(|stderr| thread::spawn(move || read_limited(stderr)));

        let deadline = Instant::now() + timeout;
        let (status_code, timed_out) = loop {
            match child.try_wait() {
                Ok(Some(status)) => break (status.code(), false),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        break (None, true);
                    }
                    thread::sleep(POLL_INTERVAL);
                }
                Err(error) => {
                    return Err(process_error(
                        ProcessErrorCode::IoFailed,
                        format!("failed while waiting for {}: {error}", request.command),
                    ));
                }
            }
        };

        let reader_deadline = Instant::now() + OUTPUT_READER_JOIN_TIMEOUT;
        Ok(ProcessExecResponse {
            status_code,
            stdout_base64: join_reader(stdout_handle, reader_deadline)?,
            stderr_base64: join_reader(stderr_handle, reader_deadline)?,
            timed_out,
        })
    }

    fn command_allowed(&self, command: &str) -> bool {
        self.allowed_commands
            .iter()
            .any(|allowed| same_command_path(command, allowed))
    }
}

#[derive(Debug, Deserialize)]
struct ProcessExecRequest {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    working_directory: Option<String>,
    #[serde(default)]
    stdin_base64: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct ProcessExecResponse {
    status_code: Option<i32>,
    stdout_base64: String,
    stderr_base64: String,
    timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProcessErrorCode {
    PermissionDenied,
    SpawnFailed,
    IoFailed,
    ProtocolError,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProcessError {
    code: ProcessErrorCode,
    message: String,
}

fn resolve_allowed_commands(
    descriptor: &PluginDescriptor,
    config_json: Option<&str>,
) -> Vec<String> {
    let ProviderDescriptor::Notification(notification) = &descriptor.provider else {
        return Vec::new();
    };
    if !notification.capabilities.requires_host_process {
        return Vec::new();
    }

    let mut commands = Vec::new();
    if let Some(config) =
        config_json.and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
    {
        for key in ["path", "command", "executable", "script_path"] {
            if let Some(value) = config
                .get(key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                commands.push(value.to_string());
            }
        }
    }

    if notification.provider_type.eq_ignore_ascii_case("synology") {
        commands.push("/usr/syno/bin/synoindex".to_string());
    }

    commands.sort();
    commands.dedup();
    commands
}

/// Clean, minimal `PATH` handed to every spawned host process. The host-process
/// capability is reserved for first-party/verified plugins, but we still refuse
/// to resolve bare command names through a guest- or host-supplied `PATH`.
const SANITIZED_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Dynamic-linker control variables (`LD_PRELOAD`, `LD_LIBRARY_PATH`, `LD_AUDIT`,
/// the macOS `DYLD_*` family, ...) can turn an otherwise-benign, allowlisted
/// interpreter into an arbitrary-code loader. They must never reach a spawned
/// child, whether the guest supplied them or the host process inherited them.
fn is_dynamic_linker_env_key(key: &str) -> bool {
    let upper = key.trim().to_ascii_uppercase();
    upper.starts_with("LD_") || upper.starts_with("DYLD_")
}

/// Sanitize the child environment before spawning a host process.
///
/// Defense-in-depth for the host-process capability: even though it is now gated
/// to first-party/verified plugins, we do not forward a guest-controlled
/// dynamic-linker environment or an attacker-chosen `PATH` into the child. We
/// strip any inherited `LD_*`/`DYLD_*`, drop the same keys from the guest-provided
/// env, and force a clean minimal `PATH`.
///
/// NOTE: argv is intentionally left unconstrained here. A first-party
/// `custom_script` provider legitimately passes arbitrary arguments, so an argv
/// denylist would break the supported use case. A future hardening option is
/// per-provider argv templating so the argument vector is also constrained.
fn apply_sanitized_env(command: &mut Command, guest_env: &BTreeMap<String, String>) {
    // Strip dynamic-linker controls inherited from the host process so they can
    // never reach the child even if Scryer itself was launched with them set.
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(is_dynamic_linker_env_key) {
            command.env_remove(&key);
        }
    }

    for (key, value) in guest_env {
        // Never let the guest reintroduce a dynamic-linker override...
        if is_dynamic_linker_env_key(key) {
            continue;
        }
        // ...or pick the PATH used to resolve bare command names.
        if key.eq_ignore_ascii_case("PATH") {
            continue;
        }
        command.env(key, value);
    }

    command.env("PATH", SANITIZED_PATH);
}

fn validate_request(request: &ProcessExecRequest) -> Result<(), ProcessError> {
    if request.command.trim().is_empty() {
        return Err(process_error(
            ProcessErrorCode::ProtocolError,
            "process command must not be empty",
        ));
    }
    if request.args.len() > MAX_ARGS {
        return Err(process_error(
            ProcessErrorCode::ProtocolError,
            format!("process arg count exceeds {MAX_ARGS}"),
        ));
    }
    if request.env.len() > MAX_ENV_VARS {
        return Err(process_error(
            ProcessErrorCode::ProtocolError,
            format!("process environment count exceeds {MAX_ENV_VARS}"),
        ));
    }
    Ok(())
}

fn decode_stdin(stdin_base64: Option<&str>) -> Result<Vec<u8>, ProcessError> {
    let Some(stdin_base64) = stdin_base64 else {
        return Ok(Vec::new());
    };
    let bytes = STANDARD.decode(stdin_base64.as_bytes()).map_err(|error| {
        process_error(
            ProcessErrorCode::ProtocolError,
            format!("failed to decode process stdin: {error}"),
        )
    })?;
    if bytes.len() > MAX_STDIN_BYTES {
        return Err(process_error(
            ProcessErrorCode::ProtocolError,
            format!("process stdin exceeds {MAX_STDIN_BYTES} bytes"),
        ));
    }
    Ok(bytes)
}

fn same_command_path(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    if left == right {
        return true;
    }

    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => Path::new(left) == Path::new(right),
    }
}

fn read_limited(mut reader: impl Read) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(output);
        }
        let remaining = MAX_OUTPUT_BYTES.saturating_sub(output.len());
        if remaining > 0 {
            output.extend_from_slice(&buffer[..read.min(remaining)]);
        }
    }
}

fn join_reader(
    handle: Option<thread::JoinHandle<io::Result<Vec<u8>>>>,
    deadline: Instant,
) -> Result<String, ProcessError> {
    let Some(handle) = handle else {
        return Ok(String::new());
    };
    while !handle.is_finished() {
        if Instant::now() >= deadline {
            return Err(process_error(
                ProcessErrorCode::IoFailed,
                "process output reader did not finish before timeout",
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
    let bytes = handle
        .join()
        .map_err(|_| process_error(ProcessErrorCode::IoFailed, "process output reader panicked"))?
        .map_err(|error| {
            process_error(
                ProcessErrorCode::IoFailed,
                format!("failed to read process output: {error}"),
            )
        })?;
    Ok(STANDARD.encode(bytes))
}

/// Base64 the host produced itself, so a decode failure is a host bug rather
/// than something a guest can provoke; an empty payload is the safe reading.
fn decode_output(encoded: &str) -> Vec<u8> {
    STANDARD.decode(encoded.as_bytes()).unwrap_or_default()
}

/// Project a process failure onto the SDK's error shape.
///
/// `PluginError` has no room for the process layer's own code, so a JSON
/// document carrying it is placed in
/// `debug_message`: a guest that wants the exact `permission_denied` /
/// `spawn_failed` / `io_failed` / `protocol_error` discriminant parses it back
/// out, and one that only needs to branch reads `code`.
fn process_plugin_error(error: &ProcessError) -> PluginError {
    let code = match &error.code {
        // A denied command is a policy decision about this plugin, not a
        // missing capability: retrying cannot change it, and it is not the
        // "service is not configured" answer a disabled host gives.
        ProcessErrorCode::PermissionDenied | ProcessErrorCode::ProtocolError => {
            PluginErrorCode::Permanent
        }
        ProcessErrorCode::SpawnFailed | ProcessErrorCode::IoFailed => PluginErrorCode::Temporary,
        ProcessErrorCode::Unsupported => PluginErrorCode::Unsupported,
    };
    PluginError {
        code,
        public_message: error.message.clone(),
        debug_message: Some(serde_json::to_string(error).unwrap_or_else(|_| error.message.clone())),
        retry_after_seconds: Some(0),
        details: None,
    }
}

fn process_error(code: ProcessErrorCode, message: impl Into<String>) -> ProcessError {
    ProcessError {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{NotificationCapabilities, NotificationDescriptor};

    fn notification_descriptor(requires_host_process: bool) -> PluginDescriptor {
        PluginDescriptor {
            id: "custom-script".to_string(),
            name: "Custom Script".to_string(),
            version: "0.1.0".to_string(),
            sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            socket_permissions: vec![],
            provider: ProviderDescriptor::Notification(NotificationDescriptor {
                provider_type: "custom_script".to_string(),
                provider_aliases: vec![],
                config_fields: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                capabilities: NotificationCapabilities {
                    requires_host_process,
                    ..Default::default()
                },
            }),
        }
    }

    // (b) A first-party/verified plugin that declares the capability still
    // resolves a working, non-empty allowlist from its config.
    #[test]
    fn resolve_allowed_commands_uses_config_paths_when_capability_declared() {
        let descriptor = notification_descriptor(true);
        let commands = resolve_allowed_commands(&descriptor, Some(r#"{"path":"/usr/bin/env"}"#));
        assert_eq!(commands, vec!["/usr/bin/env".to_string()]);
    }

    #[test]
    fn resolve_allowed_commands_empty_without_capability() {
        let descriptor = notification_descriptor(false);
        let commands = resolve_allowed_commands(&descriptor, Some(r#"{"path":"/usr/bin/env"}"#));
        assert!(commands.is_empty());
    }

    // (c) The disabled host that non-first-party/Unverified plugins receive denies
    // every exec, regardless of the requested command.
    #[test]
    fn disabled_process_host_denies_exec() {
        let host = ProcessHost::disabled();
        assert_eq!(host.allowed_command_count(), 0);

        let result = host
            .exec(PluginProcessExecRequest {
                command: "/usr/bin/env".to_string(),
                args: Vec::new(),
                env: BTreeMap::new(),
                cwd: None,
                stdin: Vec::new(),
                timeout_ms: None,
            })
            .expect("a denial is an answer, not a host fault");

        let PluginResult::Err(error) = result else {
            panic!("a disabled process host must deny every exec");
        };
        assert_eq!(error.code, PluginErrorCode::Permanent);
        assert!(
            error
                .debug_message
                .as_deref()
                .is_some_and(|debug| debug.contains("permission_denied")),
            "{:?}",
            error.debug_message
        );
    }

    // (d) The child environment never carries a guest- or host-supplied
    // dynamic-linker override, and PATH is forced to the sanitized value.
    #[test]
    fn execute_strips_dynamic_linker_env_and_forces_clean_path() {
        let mut env = BTreeMap::new();
        env.insert("LD_PRELOAD".to_string(), "/tmp/evil.so".to_string());
        env.insert("LD_LIBRARY_PATH".to_string(), "/tmp/evil".to_string());
        env.insert(
            "DYLD_INSERT_LIBRARIES".to_string(),
            "/tmp/evil.dylib".to_string(),
        );
        env.insert("PATH".to_string(), "/tmp/attacker/bin".to_string());
        env.insert("SCRYER_TEST_MARKER".to_string(), "kept".to_string());

        let request = ProcessExecRequest {
            command: "/usr/bin/env".to_string(),
            args: vec![],
            env,
            working_directory: None,
            stdin_base64: None,
            timeout_ms: Some(5_000),
        };

        let state = ProcessHostState::new(vec!["/usr/bin/env".to_string()]);
        let response = state.execute(request).expect("env should execute");
        let stdout_bytes = STANDARD
            .decode(response.stdout_base64.as_bytes())
            .expect("stdout base64");
        let stdout = String::from_utf8_lossy(&stdout_bytes);

        for line in stdout.lines() {
            assert!(
                !line.starts_with("LD_") && !line.starts_with("DYLD_"),
                "dynamic-linker env leaked into child: {line}"
            );
        }
        assert!(
            stdout.contains("SCRYER_TEST_MARKER=kept"),
            "non-sensitive guest env should be preserved: {stdout}"
        );
        assert!(
            stdout.contains(&format!("PATH={SANITIZED_PATH}")),
            "PATH should be forced to the sanitized value: {stdout}"
        );
    }
}

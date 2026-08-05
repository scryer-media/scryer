use std::env;
use std::ffi::{CString, OsString};
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const CONFIG_DIR: &str = "/config";
const DEFAULT_DB_PATH: &str = "/config/scryer.db";
const PAYLOAD_NAME: &str = "scryer";
const LAUNCHER_UID_DEFAULT: u32 = 1000;
const LAUNCHER_GID_DEFAULT: u32 = 1000;

fn main() {
    let config = LaunchConfig::from_env(env::args_os().skip(1).collect());
    let ops = RealLauncherOps;
    if let Err(error) = run_with_ops(&ops, &config) {
        eprintln!("failed to launch scryer: {error}");
        std::process::exit(1);
    }
    unreachable!("launcher exec unexpectedly returned without replacing the process");
}

#[derive(Clone, Debug)]
struct LaunchConfig {
    args: Vec<OsString>,
    payload_root: PathBuf,
    db_path: PathBuf,
    requested_uid: Option<String>,
    requested_gid: Option<String>,
    umask: Option<String>,
}

impl LaunchConfig {
    fn from_env(args: Vec<OsString>) -> Self {
        let db_path = resolved_db_path(env::var("SCRYER_DB_PATH").ok());
        Self {
            args,
            payload_root: env::var_os("SCRYER_PAYLOAD_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/opt/scryer")),
            db_path,
            requested_uid: env::var("PUID").ok(),
            requested_gid: env::var("PGID").ok(),
            umask: env::var("UMASK").ok(),
        }
    }

    fn launch_args(&self) -> Vec<OsString> {
        let mut args = Vec::with_capacity(self.args.len() + 2);
        args.push(OsString::from("--data-dir"));
        args.push(OsString::from(CONFIG_DIR));
        args.extend(self.args.iter().cloned());
        args
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeLaunch {
    primary: PathBuf,
}

trait LauncherOps {
    fn effective_uid(&self) -> u32;
    fn effective_gid(&self) -> u32;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn path_is_dir(&self, path: &Path) -> bool;
    fn path_is_executable_file(&self, path: &Path) -> bool;
    fn chown_recursive(&self, path: &Path, uid: u32, gid: u32) -> io::Result<()>;
    fn set_umask(&self, mask: u32) -> io::Result<()>;
    fn drop_privileges(&self, uid: u32, gid: u32) -> io::Result<()>;
    fn exec(&self, program: &Path, args: &[OsString]) -> io::Result<()>;
    fn warn(&self, message: &str);

    fn is_root(&self) -> bool {
        self.effective_uid() == 0
    }
}

struct RealLauncherOps;

impl LauncherOps for RealLauncherOps {
    fn effective_uid(&self) -> u32 {
        // SAFETY: geteuid has no preconditions.
        unsafe { libc::geteuid() }
    }

    fn effective_gid(&self) -> u32 {
        // SAFETY: getegid has no preconditions.
        unsafe { libc::getegid() }
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn path_is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn path_is_executable_file(&self, path: &Path) -> bool {
        match fs::metadata(path) {
            Ok(metadata) => metadata.is_file() && (metadata.permissions().mode() & 0o111 != 0),
            Err(_) => false,
        }
    }

    fn chown_recursive(&self, path: &Path, uid: u32, gid: u32) -> io::Result<()> {
        chown_recursive(path, uid, gid)
    }

    fn set_umask(&self, mask: u32) -> io::Result<()> {
        // SAFETY: umask has no additional preconditions beyond a plain integer mode.
        unsafe {
            libc::umask(mask as libc::mode_t);
        }
        Ok(())
    }

    fn drop_privileges(&self, uid: u32, gid: u32) -> io::Result<()> {
        // SAFETY: setgroups/setgid/setuid are process-global but valid here before exec.
        unsafe {
            if libc::setgroups(0, std::ptr::null()) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::setgid(gid) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::setuid(uid) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::getuid() != uid
                || libc::geteuid() != uid
                || libc::getgid() != gid
                || libc::getegid() != gid
            {
                return Err(io::Error::other(format!(
                    "credential verification failed after privilege drop; uid={} euid={} gid={} egid={}",
                    libc::getuid(),
                    libc::geteuid(),
                    libc::getgid(),
                    libc::getegid()
                )));
            }
        }
        Ok(())
    }

    fn exec(&self, program: &Path, args: &[OsString]) -> io::Result<()> {
        let mut command = Command::new(program);
        command.args(args.iter());
        Err(command.exec())
    }

    fn warn(&self, message: &str) {
        eprintln!("warning: {message}");
    }
}

fn run_with_ops<O: LauncherOps>(ops: &O, config: &LaunchConfig) -> Result<(), String> {
    let runtime = resolve_runtime_launch(ops, config)?;

    apply_umask(ops, config);
    if ops.is_root() {
        let (uid, gid) = requested_identity(ops, config);
        repair_ownership(ops, config, uid, gid);
        ops.drop_privileges(uid, gid)
            .map_err(|error| format!("failed to drop privileges to {uid}:{gid}: {error}"))?;
    }

    let args = config.launch_args();
    ops.exec(&runtime.primary, &args)
        .map_err(|error| format!("failed to launch '{}': {error}", runtime.primary.display()))
}

fn resolve_runtime_launch<O: LauncherOps>(
    ops: &O,
    config: &LaunchConfig,
) -> Result<RuntimeLaunch, String> {
    let payload = config.payload_root.join(PAYLOAD_NAME);
    if ops.path_is_executable_file(&payload) {
        Ok(RuntimeLaunch { primary: payload })
    } else {
        Err(format!(
            "no executable scryer payload found at {}",
            payload.display()
        ))
    }
}

fn resolved_db_path(raw: Option<String>) -> PathBuf {
    let Some(raw) = raw else {
        return PathBuf::from(DEFAULT_DB_PATH);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return PathBuf::from(DEFAULT_DB_PATH);
    }

    let stripped = trimmed.strip_prefix("sqlite://").unwrap_or(trimmed);
    let without_query = stripped.split('?').next().unwrap_or(stripped).trim();
    if without_query.is_empty() {
        PathBuf::from(DEFAULT_DB_PATH)
    } else {
        let candidate = PathBuf::from(without_query);
        if candidate.is_absolute() {
            candidate
        } else {
            PathBuf::from(DEFAULT_DB_PATH)
        }
    }
}

fn apply_umask<O: LauncherOps>(ops: &O, config: &LaunchConfig) {
    let Some(raw) = config.umask.as_deref() else {
        return;
    };
    match parse_octal_mode(raw) {
        Ok(Some(mask)) => {
            if let Err(error) = ops.set_umask(mask) {
                ops.warn(&format!(
                    "failed to apply UMASK '{}': {error}; continuing with the process default",
                    raw
                ));
            }
        }
        Ok(None) => {}
        Err(error) => ops.warn(&format!(
            "invalid UMASK '{}': {error}; continuing with the process default",
            raw
        )),
    }
}

fn parse_octal_mode(raw: &str) -> Result<Option<u32>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let digits = trimmed.strip_prefix("0o").unwrap_or(trimmed);
    if digits.is_empty() || !digits.chars().all(|ch| matches!(ch, '0'..='7')) {
        return Err("expected an octal mode such as 022".into());
    }
    u32::from_str_radix(digits, 8)
        .map(Some)
        .map_err(|error| format!("failed to parse octal mode: {error}"))
}

fn requested_identity<O: LauncherOps>(ops: &O, config: &LaunchConfig) -> (u32, u32) {
    if !ops.is_root() {
        return (ops.effective_uid(), ops.effective_gid());
    }

    let uid = parse_requested_id(config.requested_uid.as_deref(), LAUNCHER_UID_DEFAULT)
        .unwrap_or_else(|error| {
            ops.warn(&format!(
                "invalid PUID '{}': {error}; falling back to {LAUNCHER_UID_DEFAULT}",
                config.requested_uid.as_deref().unwrap_or("")
            ));
            LAUNCHER_UID_DEFAULT
        });
    let gid = parse_requested_id(config.requested_gid.as_deref(), LAUNCHER_GID_DEFAULT)
        .unwrap_or_else(|error| {
            ops.warn(&format!(
                "invalid PGID '{}': {error}; falling back to {LAUNCHER_GID_DEFAULT}",
                config.requested_gid.as_deref().unwrap_or("")
            ));
            LAUNCHER_GID_DEFAULT
        });
    (uid, gid)
}

fn parse_requested_id(raw: Option<&str>, default: u32) -> Result<u32, String> {
    let Some(raw) = raw else {
        return Ok(default);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(default);
    }
    trimmed
        .parse::<u32>()
        .map_err(|error| format!("failed to parse numeric id: {error}"))
}

fn repair_ownership<O: LauncherOps>(ops: &O, config: &LaunchConfig, uid: u32, gid: u32) {
    if let Err(error) = ops.create_dir_all(Path::new(CONFIG_DIR)) {
        ops.warn(&format!(
            "failed to create {CONFIG_DIR} before ownership repair: {error}"
        ));
    }

    if let Err(error) = ops.chown_recursive(Path::new(CONFIG_DIR), uid, gid) {
        ops.warn(&format!(
            "failed to re-own {CONFIG_DIR} to {uid}:{gid}: {error}"
        ));
    }

    let db_dir = config
        .db_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(CONFIG_DIR));
    if db_dir != Path::new(CONFIG_DIR)
        && ops.path_is_dir(&db_dir)
        && let Err(error) = ops.chown_recursive(&db_dir, uid, gid)
    {
        ops.warn(&format!(
            "failed to re-own {} to {uid}:{gid}: {error}",
            db_dir.display()
        ));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChownNodeKind {
    Directory,
    Other,
}

trait ChownTraversal {
    fn chown(&self, path: &Path, uid: u32, gid: u32) -> io::Result<()>;
    fn node_kind(&self, path: &Path) -> io::Result<ChownNodeKind>;
    fn child_paths(&self, path: &Path) -> io::Result<Vec<io::Result<PathBuf>>>;
}

struct FsChownTraversal;

impl ChownTraversal for FsChownTraversal {
    fn chown(&self, path: &Path, uid: u32, gid: u32) -> io::Result<()> {
        chown_one(path, uid, gid)
    }

    fn node_kind(&self, path: &Path) -> io::Result<ChownNodeKind> {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_dir() {
            Ok(ChownNodeKind::Directory)
        } else {
            Ok(ChownNodeKind::Other)
        }
    }

    fn child_paths(&self, path: &Path) -> io::Result<Vec<io::Result<PathBuf>>> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(path)? {
            paths.push(entry.map(|entry| entry.path()));
        }
        Ok(paths)
    }
}

#[derive(Default)]
struct ChownRepairFailures {
    count: usize,
    first: Option<String>,
}

impl ChownRepairFailures {
    fn record(&mut self, path: &Path, error: io::Error) {
        self.count += 1;
        if self.first.is_none() {
            self.first = Some(format!("{}: {}", path.display(), error));
        }
    }

    fn into_result(self) -> io::Result<()> {
        if self.count == 0 {
            return Ok(());
        }

        Err(io::Error::other(format!(
            "{} ownership repair operation(s) failed; first failure: {}",
            self.count,
            self.first.unwrap_or_else(|| "unknown".to_string())
        )))
    }
}

fn chown_recursive(path: &Path, uid: u32, gid: u32) -> io::Result<()> {
    chown_recursive_with(&FsChownTraversal, path, uid, gid)
}

fn chown_recursive_with<T: ChownTraversal>(
    traversal: &T,
    path: &Path,
    uid: u32,
    gid: u32,
) -> io::Result<()> {
    let mut failures = ChownRepairFailures::default();
    chown_recursive_inner(traversal, path, uid, gid, &mut failures);
    failures.into_result()
}

fn chown_recursive_inner<T: ChownTraversal>(
    traversal: &T,
    path: &Path,
    uid: u32,
    gid: u32,
    failures: &mut ChownRepairFailures,
) {
    if let Err(error) = traversal.chown(path, uid, gid) {
        failures.record(path, error);
    }

    let kind = match traversal.node_kind(path) {
        Ok(kind) => kind,
        Err(error) => {
            failures.record(path, error);
            return;
        }
    };
    if kind != ChownNodeKind::Directory {
        return;
    }

    let children = match traversal.child_paths(path) {
        Ok(children) => children,
        Err(error) => {
            failures.record(path, error);
            return;
        }
    };

    for child in children {
        let child_path = match child {
            Ok(child_path) => child_path,
            Err(error) => {
                failures.record(path, error);
                continue;
            }
        };
        match traversal.node_kind(&child_path) {
            Ok(ChownNodeKind::Directory) => {
                chown_recursive_inner(traversal, &child_path, uid, gid, failures);
            }
            Ok(ChownNodeKind::Other) => {
                if let Err(error) = traversal.chown(&child_path, uid, gid) {
                    failures.record(&child_path, error);
                }
            }
            Err(error) => {
                failures.record(&child_path, error);
            }
        }
    }
}

fn chown_one(path: &Path, uid: u32, gid: u32) -> io::Result<()> {
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL byte"))?;
    // SAFETY: libc::lchown expects a valid NUL-terminated path pointer and plain numeric ids.
    let result = unsafe { libc::lchown(c_path.as_ptr(), uid, gid) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Clone)]
    struct MockEntry {
        is_dir: bool,
        executable: bool,
    }

    impl MockEntry {
        fn directory() -> Self {
            Self {
                is_dir: true,
                executable: false,
            }
        }

        fn executable_file() -> Self {
            Self {
                is_dir: false,
                executable: true,
            }
        }
    }

    #[derive(Clone, Copy)]
    enum MockExecResult {
        Success,
        Failure,
    }

    #[derive(Clone, Copy)]
    enum FakeChownEntry {
        Directory,
        Other,
        MetadataError,
    }

    #[derive(Default)]
    struct FakeChownTraversal {
        entries: HashMap<PathBuf, FakeChownEntry>,
        children: HashMap<PathBuf, Vec<Result<PathBuf, &'static str>>>,
        chown_failures: HashMap<PathBuf, &'static str>,
        chown_calls: RefCell<Vec<PathBuf>>,
    }

    impl FakeChownTraversal {
        fn insert_entry<P: AsRef<Path>>(&mut self, path: P, entry: FakeChownEntry) {
            self.entries.insert(path.as_ref().to_path_buf(), entry);
        }

        fn insert_children<P: AsRef<Path>>(
            &mut self,
            path: P,
            children: Vec<Result<PathBuf, &'static str>>,
        ) {
            self.children.insert(path.as_ref().to_path_buf(), children);
        }

        fn fail_chown<P: AsRef<Path>>(&mut self, path: P, error: &'static str) {
            self.chown_failures
                .insert(path.as_ref().to_path_buf(), error);
        }

        fn chown_calls(&self) -> Vec<PathBuf> {
            self.chown_calls.borrow().clone()
        }
    }

    impl ChownTraversal for FakeChownTraversal {
        fn chown(&self, path: &Path, _uid: u32, _gid: u32) -> io::Result<()> {
            self.chown_calls.borrow_mut().push(path.to_path_buf());
            if let Some(error) = self.chown_failures.get(path) {
                Err(io::Error::other(*error))
            } else {
                Ok(())
            }
        }

        fn node_kind(&self, path: &Path) -> io::Result<ChownNodeKind> {
            match self.entries.get(path) {
                Some(FakeChownEntry::Directory) => Ok(ChownNodeKind::Directory),
                Some(FakeChownEntry::Other) => Ok(ChownNodeKind::Other),
                Some(FakeChownEntry::MetadataError) => Err(io::Error::other("metadata failed")),
                None => Err(io::Error::new(io::ErrorKind::NotFound, "missing entry")),
            }
        }

        fn child_paths(&self, path: &Path) -> io::Result<Vec<io::Result<PathBuf>>> {
            Ok(self
                .children
                .get(path)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|child| child.map_err(io::Error::other))
                .collect())
        }
    }

    struct MockLauncherOps {
        uid: u32,
        gid: u32,
        entries: RefCell<HashMap<PathBuf, MockEntry>>,
        warnings: RefCell<Vec<String>>,
        chown_calls: RefCell<Vec<(PathBuf, u32, u32)>>,
        drop_calls: RefCell<Vec<(u32, u32)>>,
        exec_calls: RefCell<Vec<(PathBuf, Vec<OsString>)>>,
        exec_results: RefCell<Vec<MockExecResult>>,
        fail_create_dir: bool,
        fail_chown: bool,
        fail_umask: bool,
        fail_drop: bool,
    }

    impl Default for MockLauncherOps {
        fn default() -> Self {
            Self {
                uid: 0,
                gid: 0,
                entries: RefCell::new(HashMap::new()),
                warnings: RefCell::new(Vec::new()),
                chown_calls: RefCell::new(Vec::new()),
                drop_calls: RefCell::new(Vec::new()),
                exec_calls: RefCell::new(Vec::new()),
                exec_results: RefCell::new(Vec::new()),
                fail_create_dir: false,
                fail_chown: false,
                fail_umask: false,
                fail_drop: false,
            }
        }
    }

    impl MockLauncherOps {
        fn insert_entry<P: AsRef<Path>>(&self, path: P, entry: MockEntry) {
            self.entries
                .borrow_mut()
                .insert(path.as_ref().to_path_buf(), entry);
        }

        fn push_exec_results(&self, results: &[MockExecResult]) {
            self.exec_results
                .borrow_mut()
                .extend(results.iter().copied());
        }

        fn warnings(&self) -> Vec<String> {
            self.warnings.borrow().clone()
        }

        fn chown_calls(&self) -> Vec<(PathBuf, u32, u32)> {
            self.chown_calls.borrow().clone()
        }

        fn drop_calls(&self) -> Vec<(u32, u32)> {
            self.drop_calls.borrow().clone()
        }

        fn exec_paths(&self) -> Vec<PathBuf> {
            self.exec_calls
                .borrow()
                .iter()
                .map(|(path, _)| path.clone())
                .collect()
        }
    }

    impl LauncherOps for MockLauncherOps {
        fn effective_uid(&self) -> u32 {
            self.uid
        }

        fn effective_gid(&self) -> u32 {
            self.gid
        }

        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            if self.fail_create_dir {
                return Err(io::Error::other("create dir failed"));
            }
            self.insert_entry(path, MockEntry::directory());
            Ok(())
        }

        fn path_is_dir(&self, path: &Path) -> bool {
            self.entries
                .borrow()
                .get(path)
                .is_some_and(|entry| entry.is_dir)
        }

        fn path_is_executable_file(&self, path: &Path) -> bool {
            self.entries
                .borrow()
                .get(path)
                .is_some_and(|entry| !entry.is_dir && entry.executable)
        }

        fn chown_recursive(&self, path: &Path, uid: u32, gid: u32) -> io::Result<()> {
            self.chown_calls
                .borrow_mut()
                .push((path.to_path_buf(), uid, gid));
            if self.fail_chown {
                Err(io::Error::other("chown failed"))
            } else {
                Ok(())
            }
        }

        fn set_umask(&self, _mask: u32) -> io::Result<()> {
            if self.fail_umask {
                Err(io::Error::other("umask failed"))
            } else {
                Ok(())
            }
        }

        fn drop_privileges(&self, uid: u32, gid: u32) -> io::Result<()> {
            self.drop_calls.borrow_mut().push((uid, gid));
            if self.fail_drop {
                Err(io::Error::other("drop failed"))
            } else {
                Ok(())
            }
        }

        fn exec(&self, program: &Path, args: &[OsString]) -> io::Result<()> {
            self.exec_calls
                .borrow_mut()
                .push((program.to_path_buf(), args.to_vec()));
            let next_result = {
                let mut results = self.exec_results.borrow_mut();
                if results.is_empty() {
                    None
                } else {
                    Some(results.remove(0))
                }
            };
            match next_result.unwrap_or(MockExecResult::Success) {
                MockExecResult::Success => Ok(()),
                MockExecResult::Failure => Err(io::Error::other("exec failed")),
            }
        }

        fn warn(&self, message: &str) {
            self.warnings.borrow_mut().push(message.to_string());
        }
    }

    fn base_config() -> LaunchConfig {
        LaunchConfig {
            args: vec![OsString::from("--version")],
            payload_root: PathBuf::from("/payloads"),
            db_path: PathBuf::from(DEFAULT_DB_PATH),
            requested_uid: Some("1000".into()),
            requested_gid: Some("1000".into()),
            umask: None,
        }
    }

    #[test]
    fn runtime_launch_should_select_single_payload() {
        let ops = MockLauncherOps::default();
        ops.insert_entry("/payloads/scryer", MockEntry::executable_file());
        let config = base_config();
        let runtime = resolve_runtime_launch(&ops, &config).expect("runtime payload");
        assert_eq!(runtime.primary, PathBuf::from("/payloads/scryer"));
    }

    #[test]
    fn runtime_launch_should_report_missing_single_payload() {
        let ops = MockLauncherOps::default();
        let error = resolve_runtime_launch(&ops, &base_config()).expect_err("missing payload");
        assert_eq!(
            error,
            "no executable scryer payload found at /payloads/scryer"
        );
    }

    #[test]
    fn relative_db_paths_should_fallback_to_the_default_config_database() {
        assert_eq!(
            resolved_db_path(Some("./scryer.db".into())),
            PathBuf::from(DEFAULT_DB_PATH)
        );
        assert_eq!(
            resolved_db_path(Some("sqlite://relative/scryer.db?mode=rwc".into())),
            PathBuf::from(DEFAULT_DB_PATH)
        );
    }

    #[test]
    fn invalid_umask_should_still_attempt_to_launch() {
        let ops = MockLauncherOps::default();
        ops.insert_entry("/payloads/scryer", MockEntry::executable_file());
        let mut config = base_config();
        config.umask = Some("not-octal".into());
        run_with_ops(&ops, &config).expect("launch should still proceed");
        assert_eq!(ops.exec_paths(), vec![PathBuf::from("/payloads/scryer")]);
        assert!(
            ops.warnings()
                .iter()
                .any(|warning| warning.contains("invalid UMASK"))
        );
    }

    #[test]
    fn failed_ownership_repair_should_still_attempt_to_launch() {
        let ops = MockLauncherOps {
            fail_chown: true,
            ..Default::default()
        };
        ops.insert_entry("/payloads/scryer", MockEntry::executable_file());
        run_with_ops(&ops, &base_config()).expect("launch should still proceed");
        assert_eq!(ops.exec_paths(), vec![PathBuf::from("/payloads/scryer")]);
        assert!(
            ops.warnings()
                .iter()
                .any(|warning| warning.contains("failed to re-own"))
        );
    }

    #[test]
    fn recursive_chown_should_continue_after_child_failures() {
        let mut traversal = FakeChownTraversal::default();
        traversal.insert_entry("/config", FakeChownEntry::Directory);
        traversal.insert_entry("/config/gone", FakeChownEntry::MetadataError);
        traversal.insert_entry("/config/readonly", FakeChownEntry::Directory);
        traversal.insert_entry("/config/later-file", FakeChownEntry::Other);
        traversal.insert_entry("/config/later-dir", FakeChownEntry::Directory);
        traversal.insert_entry("/config/later-dir/nested", FakeChownEntry::Other);
        traversal.insert_children(
            "/config",
            vec![
                Ok(PathBuf::from("/config/gone")),
                Ok(PathBuf::from("/config/readonly")),
                Err("entry vanished"),
                Ok(PathBuf::from("/config/later-file")),
                Ok(PathBuf::from("/config/later-dir")),
            ],
        );
        traversal.insert_children("/config/readonly", vec![Err("read-only submount")]);
        traversal.insert_children(
            "/config/later-dir",
            vec![Ok(PathBuf::from("/config/later-dir/nested"))],
        );
        traversal.fail_chown("/config/later-file", "chown failed");

        let error = chown_recursive_with(&traversal, Path::new("/config"), 1000, 1000)
            .expect_err("partial recursive chown should report aggregate failure");

        let calls = traversal.chown_calls();
        assert!(calls.contains(&PathBuf::from("/config")));
        assert!(calls.contains(&PathBuf::from("/config/readonly")));
        assert!(calls.contains(&PathBuf::from("/config/later-file")));
        assert!(calls.contains(&PathBuf::from("/config/later-dir")));
        assert!(calls.contains(&PathBuf::from("/config/later-dir/nested")));

        let message = error.to_string();
        assert!(message.contains("4 ownership repair operation(s) failed"));
        assert!(message.contains("/config/gone"));
    }

    #[test]
    fn failed_privilege_drop_should_stop_before_launch() {
        let ops = MockLauncherOps {
            fail_drop: true,
            ..Default::default()
        };
        ops.insert_entry("/payloads/scryer", MockEntry::executable_file());
        let error = run_with_ops(&ops, &base_config()).expect_err("launch should fail closed");
        assert!(error.contains("failed to drop privileges"));
        assert!(ops.exec_paths().is_empty());
    }

    #[test]
    fn non_root_launcher_path_should_skip_ownership_repair_and_privilege_drop() {
        let ops = MockLauncherOps {
            uid: 1001,
            gid: 1002,
            ..Default::default()
        };
        ops.insert_entry("/payloads/scryer", MockEntry::executable_file());
        run_with_ops(&ops, &base_config()).expect("launch should proceed");
        assert!(ops.chown_calls().is_empty());
        assert!(ops.drop_calls().is_empty());
        assert_eq!(ops.exec_paths(), vec![PathBuf::from("/payloads/scryer")]);
    }

    #[test]
    fn failed_exec_should_report_single_payload_error() {
        let ops = MockLauncherOps::default();
        ops.insert_entry("/payloads/scryer", MockEntry::executable_file());
        ops.push_exec_results(&[MockExecResult::Failure]);
        let error = run_with_ops(&ops, &base_config()).expect_err("exec should fail");
        assert!(error.contains("failed to launch '/payloads/scryer'"));
        assert_eq!(ops.exec_paths(), vec![PathBuf::from("/payloads/scryer")]);
    }

    #[test]
    fn launcher_should_prefix_data_dir_before_user_args() {
        let ops = MockLauncherOps::default();
        ops.insert_entry("/payloads/scryer", MockEntry::executable_file());
        let mut config = base_config();
        config.args = vec![OsString::from("--version")];
        run_with_ops(&ops, &config).expect("launch should proceed");
        let args = &ops.exec_calls.borrow()[0].1;
        assert_eq!(
            args,
            &vec![
                OsString::from("--data-dir"),
                OsString::from("/config"),
                OsString::from("--version"),
            ]
        );
    }
}

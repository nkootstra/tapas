use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::json::Value;
mod base64;
mod trust;

use trust::{ExecutableSnapshot, sha256, sha256_bytes, trusted_plugin_path, valid_sha256};

macro_rules! json {
    ({$($key:literal : $value:expr),* $(,)?}) => {
        Value::Object(vec![$(($key.as_bytes().to_vec(), Value::from($value))),*])
    };
    ([$($value:expr),* $(,)?]) => { Value::Array(vec![$(Value::from($value)),*]) };
    ($value:expr) => { Value::from($value) };
}

pub enum Management<'a> {
    Check {
        path: &'a OsStr,
    },
    Test {
        id: &'a OsStr,
    },
    Trust {
        id: &'a OsStr,
        path: &'a OsStr,
        pinned: bool,
        replace: bool,
        expected_sha256: Option<&'a OsStr>,
    },
    BindUser {
        id: &'a OsStr,
        prefix: &'a [OsString],
    },
    BindProject {
        id: &'a OsStr,
        prefix: &'a [OsString],
    },
    ApproveProject {
        expected_sha256: Option<&'a OsStr>,
    },
    Pin {
        id: &'a OsStr,
        sha256: Option<&'a OsStr>,
    },
    Unpin {
        id: &'a OsStr,
    },
    Untrust {
        id: &'a OsStr,
    },
    RevokeProject,
    List {
        json: bool,
    },
    Resolve {
        argv: &'a [OsString],
        json: bool,
    },
}

pub struct Transformed {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub evidence: crate::filters::EvidenceClass,
}

pub struct ResolvedRoute {
    id: String,
    path: PathBuf,
    digest: String,
    pinned: bool,
}

impl ResolvedRoute {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
}

pub enum Dispatch {
    Transformed(Transformed),
    Original,
}

const HELLO_DEADLINE: Duration = Duration::from_secs(2);
const TOTAL_DEADLINE: Duration = Duration::from_secs(10);
const DIAGNOSTIC_LIMIT: usize = 64 * 1024;
const HELLO_LIMIT: usize = 4 * 1024;
const RESPONSE_LIMIT: usize = crate::process::MAX_OUTPUT_BYTES.div_ceil(3) * 4 + 8 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn manage(action: Management<'_>, stdout: &mut dyn Write) -> io::Result<i32> {
    let _state_lock = if matches!(
        &action,
        Management::Trust { .. }
            | Management::BindUser { .. }
            | Management::BindProject { .. }
            | Management::ApproveProject { .. }
            | Management::Pin { .. }
            | Management::Unpin { .. }
            | Management::Untrust { .. }
            | Management::RevokeProject
    ) {
        Some(StateLock::acquire()?)
    } else {
        None
    };
    match action {
        Management::Check { path } => {
            let path = Path::new(path);
            if !path.is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "plugin path must be absolute",
                ));
            }
            check_conformance(&fs::canonicalize(path)?, "check", stdout)?;
        }
        Management::Test { id } => {
            let id = valid_id(id)?;
            let state = read_state("plugins.json")?;
            let path = state["plugins"][id]["path"]
                .as_str()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "plugin is not trusted"))?;
            check_conformance(Path::new(path), id, stdout)?;
        }
        Management::Trust {
            id,
            path,
            pinned,
            replace,
            expected_sha256,
        } => {
            let id = valid_id(id)?;
            let path = Path::new(path);
            if !path.is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "plugin path must be absolute",
                ));
            }
            let path = trusted_plugin_path(path)?;
            let mut state = read_state("plugins.json")?;
            if state["plugins"].get(id.as_bytes()).is_some() && !replace {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "plugin is already trusted; use --replace",
                ));
            }
            let actual = sha256(&path)?;
            let expected = expected_sha256.map(valid_sha256).transpose()?;
            if expected
                .as_deref()
                .is_some_and(|expected| expected != actual)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "plugin SHA-256 does not match",
                ));
            }
            let path = path.to_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "plugin path must be UTF-8")
            })?;
            state["plugins"][id] = json!({
                "path": path,
                "sha256": actual,
                "pinned": pinned || expected.is_some(),
            });
            write_state("plugins.json", &state)?;
            writeln!(stdout, "trusted local plugin {id}")?;
        }
        Management::BindUser { id, prefix } => {
            let id = valid_id(id)?;
            if prefix.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "binding prefix is empty",
                ));
            }
            let plugins = read_state("plugins.json")?;
            if plugins["plugins"].get(id.as_bytes()).is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "plugin is not trusted",
                ));
            }
            let mut state = read_state("config.json")?;
            let bindings = state["bindings"]
                .as_array_mut()
                .expect("state bindings array");
            let command = normalized_prefix(prefix)?;
            if bindings
                .iter()
                .any(|binding| binding["command"] == json!(command.clone()))
            {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "an equal user binding prefix already exists",
                ));
            }
            bindings.push(json!({
                "plugin": id,
                "command": command,
            }));
            write_state("config.json", &state)?;
            writeln!(stdout, "bound {id}")?;
        }
        Management::BindProject { id, prefix } => {
            let id = valid_id(id)?;
            if prefix.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "binding prefix is empty",
                ));
            }
            let plugins = read_state("plugins.json")?;
            if plugins["plugins"].get(id.as_bytes()).is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "plugin is not trusted",
                ));
            }
            let root = repository_root()?;
            let path = root.join(".tapas.json");
            let mut config = match fs::read(&path) {
                Ok(bytes) => parse_json(&bytes)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    json!({"version":1,"filters":Value::Array(Vec::new())})
                }
                Err(error) => return Err(error),
            };
            if !config.as_object().is_some_and(|object| object.len() == 2)
                || config["version"] != 1
                || !config["filters"].is_array()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid project plugin config",
                ));
            }
            let command = normalized_prefix(prefix)?;
            let filters = config["filters"].as_array_mut().unwrap();
            if filters
                .iter()
                .any(|filter| filter["command"] == json!(command.clone()))
            {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "an equal project binding prefix already exists",
                ));
            }
            filters.push(json!({"command":command,"plugin":id}));
            write_file_atomic(&path, &crate::json::serialize(&config), 0o644)?;
            writeln!(stdout, "bound {id} for {}", root.display())?;
        }
        Management::ApproveProject { expected_sha256 } => {
            let root = repository_root()?;
            let config_path = root.join(".tapas.json");
            let digest = sha256(&config_path)?;
            let expected = expected_sha256.map(valid_sha256).transpose()?;
            if expected
                .as_deref()
                .is_some_and(|expected| expected != digest)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "project config SHA-256 does not match current bytes",
                ));
            }
            let mut state = read_state("projects.json")?;
            state["projects"][path_key(&root)?] = json!({"sha256": digest});
            write_state("projects.json", &state)?;
            writeln!(stdout, "approved project {}", root.display())?;
        }
        Management::Pin {
            id,
            sha256: expected,
        } => {
            let id = valid_id(id)?;
            let mut state = read_state("plugins.json")?;
            let plugin = state["plugins"]
                .get_mut(id.as_bytes())
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "plugin is not trusted"))?;
            let path = plugin["path"]
                .as_str()
                .ok_or_else(|| invalid_protocol("trusted plugin has no path"))?;
            let actual = sha256(Path::new(path))?;
            let expected = expected.map(valid_sha256).transpose()?;
            if expected
                .as_deref()
                .is_some_and(|expected| expected != actual)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "plugin SHA-256 does not match current bytes",
                ));
            }
            let digest = expected.unwrap_or(actual);
            plugin["sha256"] = Value::from(digest);
            plugin["pinned"] = Value::Bool(true);
            write_state("plugins.json", &state)?;
            writeln!(stdout, "pinned {id}")?;
        }
        Management::Unpin { id } => {
            let id = valid_id(id)?;
            let mut state = read_state("plugins.json")?;
            let plugin = state["plugins"]
                .get_mut(id.as_bytes())
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "plugin is not trusted"))?;
            plugin["pinned"] = Value::Bool(false);
            write_state("plugins.json", &state)?;
            writeln!(stdout, "unpinned {id}")?;
        }
        Management::Untrust { id } => {
            let id = valid_id(id)?;
            let mut state = read_state("plugins.json")?;
            if state["plugins"].remove(id).is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "plugin is not trusted",
                ));
            }
            write_state("plugins.json", &state)?;
            writeln!(stdout, "untrusted {id}")?;
        }
        Management::RevokeProject => {
            let root = repository_root()?;
            let mut state = read_state("projects.json")?;
            state["projects"].remove(path_key(&root)?);
            write_state("projects.json", &state)?;
            writeln!(stdout, "revoked project {}", root.display())?;
        }
        Management::List { json: as_json } => {
            let state = read_state("plugins.json")?;
            if as_json {
                stdout.write_all(&crate::json::serialize(&state))?;
                writeln!(stdout)?;
            } else if let Some(plugins) = state["plugins"].as_object() {
                for (id, _) in plugins {
                    writeln!(stdout, "{}", String::from_utf8_lossy(id))?;
                }
            }
        }
        Management::Resolve {
            argv,
            json: as_json,
        } => {
            let report = resolution_report(argv)?;
            if as_json {
                stdout.write_all(&crate::json::serialize(&report))?;
                writeln!(stdout)?;
            } else {
                writeln!(
                    stdout,
                    "{}",
                    report["disposition"].as_str().unwrap_or("no-match")
                )?;
            }
        }
    }
    Ok(0)
}

fn resolution_report(argv: &[OsString]) -> io::Result<Value> {
    if crate::process::invocation::is_supported(argv) {
        return Ok(json!({
            "version": 1,
            "protocol_version": 1,
            "disposition": "core-route",
            "plugin": Value::Null,
            "scope": "core",
            "source": "static",
            "matched_prefix": Value::Null,
        }));
    }
    let project = project_config(&std::env::current_dir()?)?;
    let project_bindings = match &project {
        ProjectConfig::Approved(bindings) => bindings.as_slice(),
        ProjectConfig::Unapproved(bindings) => {
            if let Some(binding) = best_binding(bindings.iter(), argv) {
                return Ok(json!({
                    "version": 1,
                    "protocol_version": 1,
                    "disposition": "repo-unapproved",
                    "plugin": binding["plugin"].as_str(),
                    "scope": "project",
                    "source": ".tapas.json",
                    "repo_approval": false,
                    "matched_prefix": &binding["command"],
                }));
            }
            &[]
        }
        ProjectConfig::Invalid => {
            return Ok(
                json!({"version":1,"protocol_version":1,"disposition":"invalid-config","plugin":Value::Null,"scope":"project","source":".tapas.json","repo_approval":false,"matched_prefix":Value::Null}),
            );
        }
        ProjectConfig::Ambiguous => {
            return Ok(
                json!({"version":1,"protocol_version":1,"disposition":"ambiguous","plugin":Value::Null,"scope":"project","source":".tapas.json","repo_approval":true,"matched_prefix":Value::Null}),
            );
        }
        ProjectConfig::Absent => &[],
    };
    let config = read_state("config.json")?;
    let selected = best_binding(project_bindings.iter(), argv)
        .map(|binding| (binding, "project", ".tapas.json"))
        .or_else(|| {
            best_binding(config["bindings"].as_array()?.iter(), argv)
                .map(|binding| (binding, "user", "~/.tapas/config.json"))
        });
    let Some((binding, scope, source)) = selected else {
        return Ok(
            json!({"version":1,"protocol_version":1,"disposition":"no-match","plugin":Value::Null,"scope":Value::Null,"source":Value::Null,"matched_prefix":Value::Null}),
        );
    };
    let Some(id) = binding["plugin"].as_str() else {
        return Ok(
            json!({"version":1,"protocol_version":1,"disposition":"invalid-config","plugin":Value::Null,"scope":scope,"source":source,"matched_prefix":binding["command"].clone()}),
        );
    };
    let plugins = read_state("plugins.json")?;
    let plugin = &plugins["plugins"][id];
    let Some(path) = plugin["path"].as_str() else {
        return Ok(
            json!({"version":1,"protocol_version":1,"disposition":"untrusted","plugin":id,"scope":scope,"source":source,"repo_approval":scope == "project","matched_prefix":binding["command"].clone()}),
        );
    };
    let current = sha256(Path::new(path)).ok();
    let trusted = plugin["sha256"].as_str();
    let pinned = plugin["pinned"] == true;
    let disposition = if pinned && current.as_deref() != trusted {
        "integrity-mismatch"
    } else {
        "active"
    };
    let trust = json!({"mode": if pinned {"pinned"} else {"path"}, "path": path, "pinned": pinned});
    let digest = json!({"trusted": trusted, "current": current.clone(), "matches": current.as_deref() == trusted});
    Ok(json!({
        "version": 1,
        "protocol_version": 1,
        "disposition": disposition,
        "plugin": id,
        "scope": scope,
        "source": source,
        "trust": trust,
        "digest": digest,
        "repo_approval": scope == "project",
        "matched_prefix": binding["command"].clone(),
    }))
}

pub fn resolve_route(argv: &[OsString]) -> io::Result<Option<ResolvedRoute>> {
    if std::env::var_os("TAPAS_PLUGIN_ACTIVE").is_some() {
        return Ok(None);
    }
    if crate::process::invocation::is_supported(argv) {
        return Ok(None);
    }
    let project = project_config(&std::env::current_dir()?)?;
    let project = match project {
        ProjectConfig::Approved(bindings) => bindings,
        ProjectConfig::Invalid | ProjectConfig::Ambiguous => {
            return Err(invalid_protocol("invalid project plugin routing"));
        }
        ProjectConfig::Absent | ProjectConfig::Unapproved(_) => Vec::new(),
    };
    let config = read_state("config.json")?;
    let binding = best_binding(project.iter(), argv)
        .or_else(|| best_binding(config["bindings"].as_array()?.iter(), argv));
    let Some(binding) = binding else {
        return Ok(None);
    };
    let Some(id) = binding["plugin"].as_str() else {
        return Ok(None);
    };
    let plugins = read_state("plugins.json")?;
    let Some(path) = plugins["plugins"][id]["path"].as_str() else {
        return Ok(None);
    };
    let path = trusted_plugin_path(Path::new(path))?;
    let digest = sha256(&path)?;
    let pinned = plugins["plugins"][id]["pinned"] == true;
    Ok(Some(ResolvedRoute {
        id: id.to_owned(),
        path,
        digest,
        pinned,
    }))
}

pub(crate) fn hook_should_wrap(argv: &[OsString], cwd: &Path) -> bool {
    let Ok(cwd) = fs::canonicalize(cwd) else {
        return false;
    };
    match project_config(&cwd) {
        Ok(ProjectConfig::Approved(bindings)) => {
            if let Some(binding) = best_binding(bindings.iter(), argv) {
                return binding_is_active(binding, false);
            }
        }
        Ok(ProjectConfig::Absent) => {}
        Ok(ProjectConfig::Unapproved(_)) => {}
        Ok(ProjectConfig::Invalid | ProjectConfig::Ambiguous) | Err(_) => return false,
    }
    let Ok(config) = read_state("config.json") else {
        return false;
    };
    best_binding(config["bindings"].as_array().into_iter().flatten(), argv)
        .is_some_and(|binding| binding_is_active(binding, true))
}

fn binding_is_active(binding: &Value, require_pinned: bool) -> bool {
    let Some(id) = binding["plugin"].as_str() else {
        return false;
    };
    let Ok(plugins) = read_state("plugins.json") else {
        return false;
    };
    let plugin = &plugins["plugins"][id];
    let pinned = plugin["pinned"] == true;
    if require_pinned && !pinned {
        return false;
    }
    let Some(path) = plugin["path"].as_str() else {
        return false;
    };
    let Ok(current) = sha256(Path::new(path)) else {
        return false;
    };
    !pinned || plugin["sha256"].as_str() == Some(current.as_str())
}

pub fn dispatch(
    route: &ResolvedRoute,
    argv: &[OsString],
    exit_code: i32,
    outcome: crate::process::capture::CommandOutcome,
    stdout: &[u8],
    stderr: &[u8],
) -> Dispatch {
    match dispatch_inner(route, argv, exit_code, outcome, stdout, stderr) {
        Ok(transformed) => Dispatch::Transformed(transformed),
        Err(_) => Dispatch::Original,
    }
}

fn dispatch_inner(
    route: &ResolvedRoute,
    argv: &[OsString],
    exit_code: i32,
    outcome: crate::process::capture::CommandOutcome,
    stdout: &[u8],
    stderr: &[u8],
) -> io::Result<Transformed> {
    let plugins = read_state("plugins.json")?;
    let live = &plugins["plugins"][&route.id];
    if live["path"].as_str() != route.path.to_str() || sha256(&route.path)? != route.digest {
        return Err(invalid_protocol("plugin trust or integrity changed"));
    }
    if route.pinned && live["sha256"].as_str() != Some(route.digest.as_str()) {
        return Err(invalid_protocol("pinned plugin integrity changed"));
    }
    let snapshot = ExecutableSnapshot::create(&route.path, &route.digest, &state_dir()?)?;
    let transformed = match run_plugin(
        snapshot.path(),
        &route.id,
        argv,
        exit_code,
        outcome,
        stdout,
        stderr,
    )? {
        PluginResult::Transform(transformed) => transformed,
        PluginResult::Decline => return Err(invalid_protocol("plugin declined selected route")),
    };
    if live["sha256"].as_str() != Some(route.digest.as_str()) {
        let _lock = StateLock::acquire()?;
        let mut current = read_state("plugins.json")?;
        let plugin = &current["plugins"][route.id.as_str()];
        if plugin["path"].as_str() == route.path.to_str() && plugin["pinned"] == false {
            current["plugins"][route.id.as_str()]["sha256"] = Value::from(route.digest.clone());
            write_state("plugins.json", &current)?;
        }
    }
    Ok(transformed)
}

enum ProjectConfig {
    Absent,
    Unapproved(Vec<Value>),
    Approved(Vec<Value>),
    Invalid,
    Ambiguous,
}

fn project_config(cwd: &Path) -> io::Result<ProjectConfig> {
    let Ok(root) = repository_root_from(cwd) else {
        return Ok(ProjectConfig::Absent);
    };
    let path = root.join(".tapas.json");
    let bytes = match read_limited(&path, 1024 * 1024) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ProjectConfig::Absent);
        }
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return Ok(ProjectConfig::Invalid);
        }
        Err(error) => return Err(error),
    };
    let Ok(value) = parse_json(&bytes) else {
        return Ok(ProjectConfig::Invalid);
    };
    let Some(object) = value.as_object() else {
        return Ok(ProjectConfig::Invalid);
    };
    if object.len() != 2 || value["version"] != 1 || !value["filters"].is_array() {
        return Ok(ProjectConfig::Invalid);
    }
    let filters = value["filters"].as_array().unwrap();
    if filters.iter().any(|filter| {
        !filter.as_object().is_some_and(|object| object.len() == 2)
            || !filter["command"]
                .as_array()
                .is_some_and(|command| command.iter().all(Value::is_string))
            || !filter["plugin"].is_string()
    }) {
        return Ok(ProjectConfig::Invalid);
    }
    let approvals = read_state("projects.json")?;
    let expected = approvals["projects"][path_key(&root)?]["sha256"].as_str();
    let actual = sha256_bytes(&bytes);
    if expected != Some(&actual) {
        return Ok(ProjectConfig::Unapproved(filters.clone()));
    }
    for (index, left) in filters.iter().enumerate() {
        if filters[index + 1..]
            .iter()
            .any(|right| left["command"] == right["command"] && left["plugin"] != right["plugin"])
        {
            return Ok(ProjectConfig::Ambiguous);
        }
    }
    Ok(ProjectConfig::Approved(filters.clone()))
}

fn repository_root() -> io::Result<PathBuf> {
    repository_root_from(&std::env::current_dir()?)
}

fn repository_root_from(cwd: &Path) -> io::Result<PathBuf> {
    let mut current = fs::canonicalize(cwd)?;
    loop {
        if current.join(".git").exists() {
            return Ok(current);
        }
        if !current.pop() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "not inside a repository",
            ));
        }
    }
}

enum PluginResult {
    Transform(Transformed),
    Decline,
}

fn run_plugin(
    path: &Path,
    id: &str,
    argv: &[OsString],
    exit_code: i32,
    outcome: crate::process::capture::CommandOutcome,
    original_stdout: &[u8],
    original_stderr: &[u8],
) -> io::Result<PluginResult> {
    let checked_path = trusted_plugin_path(path)?;
    if checked_path != path {
        return Err(invalid_protocol("plugin path changed before execution"));
    }
    let cwd = std::env::current_dir()?;
    let started = Instant::now();
    let mut command = Command::new(path);
    command.env_clear().current_dir("/");
    for name in [
        "PATH", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR", "TEMP", "TMP",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command
        .env("TAPAS_PLUGIN_ACTIVE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let pid = child.id() as libc::pid_t;
    let child_stdout = child.stdout.take().expect("plugin stdout pipe");
    let child_stderr = child.stderr.take().expect("plugin stderr pipe");
    let (hello_tx, hello_rx) = mpsc::sync_channel(1);
    let stdout_thread = std::thread::spawn(move || -> io::Result<Vec<u8>> {
        let mut reader = BufReader::new(child_stdout);
        let hello_result = read_record_bounded(&mut reader, HELLO_LIMIT);
        let _ = hello_tx.send(hello_result);
        read_to_end_bounded(&mut reader, RESPONSE_LIMIT)
    });
    let stderr_thread = std::thread::spawn(move || -> io::Result<bool> {
        let mut reader = child_stderr;
        let mut buffer = [0_u8; 8192];
        let mut total = 0_usize;
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                return Ok(total > DIAGNOSTIC_LIMIT);
            }
            total = total.saturating_add(read);
        }
    });
    let hello = match hello_rx.recv_timeout(hello_deadline()) {
        Ok(Ok(hello)) => hello,
        Ok(Err(error)) => {
            kill_group(pid);
            let _ = child.wait();
            return Err(error);
        }
        Err(_) => {
            kill_group(pid);
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "plugin hello timed out",
            ));
        }
    };
    let hello: Value = match parse_json(hello.as_bytes()) {
        Ok(hello) => hello,
        Err(error) => {
            kill_group(pid);
            let _ = child.wait();
            return Err(invalid_protocol(error));
        }
    };
    if hello["protocol"] != "tapas-filter"
        || !hello["versions"]
            .as_array()
            .is_some_and(|v| v.iter().any(|v| *v == 1))
    {
        kill_group(pid);
        let _ = child.wait();
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported plugin hello",
        ));
    }
    let status = match outcome {
        crate::process::capture::CommandOutcome::Exited(code) => {
            json!({"kind":"exited","code":code})
        }
        crate::process::capture::CommandOutcome::Signaled(signal) => {
            json!({"kind":"signaled","signal":signal})
        }
    };
    let request = json!({
        "protocol": "tapas-filter",
        "version": 1,
        "plugin": id,
        "argv_b64": argv.iter().map(|arg| base64::encode(arg.as_bytes())).collect::<Vec<_>>(),
        "cwd_b64": base64::encode(cwd.as_os_str().as_bytes()),
        "status": status,
        "stdout_b64": base64::encode(original_stdout),
        "stderr_b64": base64::encode(original_stderr),
    });
    let mut child_stdin = child.stdin.take().expect("plugin stdin pipe");
    let writer = std::thread::spawn(move || -> io::Result<()> {
        child_stdin.write_all(&crate::json::serialize(&request))?;
        child_stdin.write_all(b"\n")
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                kill_group(pid);
                let _ = child.wait();
                return Err(error);
            }
        }
        if started.elapsed() >= plugin_deadline() {
            kill_group(pid);
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "plugin timed out"));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    writer
        .join()
        .map_err(|_| io::Error::other("plugin writer panicked"))??;
    let response = stdout_thread
        .join()
        .map_err(|_| io::Error::other("plugin reader panicked"))??;
    let diagnostics_overflowed = stderr_thread
        .join()
        .map_err(|_| io::Error::other("plugin diagnostics reader panicked"))??;
    if !status.success() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "plugin failed"));
    }
    if diagnostics_overflowed {
        return Err(invalid_protocol("plugin diagnostics overflow"));
    }
    let Some(record) = response.strip_suffix(b"\n") else {
        return Err(invalid_protocol("truncated plugin response record"));
    };
    if record.is_empty() || record.contains(&b'\n') || record.contains(&b'\r') {
        return Err(invalid_protocol(
            "plugin stdout must contain exactly one response record",
        ));
    }
    let response: Value = parse_json(record)?;
    if response["version"] == 1 && response["result"] == "decline" {
        if response.as_object().is_some_and(|object| object.len() == 2) {
            return Ok(PluginResult::Decline);
        }
        return Err(invalid_protocol("decline response has unexpected fields"));
    }
    if response["version"] != 1 || response["result"] != "transform" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid plugin response",
        ));
    }
    if !response.as_object().is_some_and(|object| object.len() == 5) {
        return Err(invalid_protocol("transform response has unexpected fields"));
    }
    if response["evidence"] != "fact-complete" && response["evidence"] != "potentially-lossy" {
        return Err(invalid_protocol("invalid plugin evidence"));
    }
    if exit_code != 0 && response["evidence"] != "fact-complete" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "lossy failure transform",
        ));
    }
    let stdout = decode(&response["stdout_b64"])?;
    let stderr = decode(&response["stderr_b64"])?;
    if stdout.len().saturating_add(stderr.len()) > crate::process::MAX_OUTPUT_BYTES {
        return Err(invalid_protocol("plugin output exceeds capture limit"));
    }
    if stdout.len() + stderr.len() >= original_stdout.len() + original_stderr.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "plugin output is not smaller",
        ));
    }
    let evidence = if response["evidence"] == "potentially-lossy" {
        crate::filters::EvidenceClass::PotentiallyLossy
    } else {
        crate::filters::EvidenceClass::FactComplete
    };
    Ok(PluginResult::Transform(Transformed {
        stdout,
        stderr,
        evidence,
    }))
}

fn hello_deadline() -> Duration {
    plugin_deadline().min(HELLO_DEADLINE)
}

fn plugin_deadline() -> Duration {
    std::env::var("TAPAS_PLUGIN_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map_or(TOTAL_DEADLINE, Duration::from_millis)
}

fn read_record_bounded(reader: &mut impl BufRead, limit: usize) -> io::Result<String> {
    let mut bytes = Vec::new();
    reader
        .take((limit + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    if bytes.len() > limit || !bytes.ends_with(b"\n") {
        return Err(invalid_protocol(
            "plugin hello exceeds limit or is truncated",
        ));
    }
    String::from_utf8(bytes).map_err(invalid_protocol)
}

fn read_to_end_bounded(reader: &mut impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(invalid_protocol("plugin response exceeds limit"));
    }
    Ok(bytes)
}

fn check_conformance(path: &Path, id: &str, stdout: &mut dyn Write) -> io::Result<()> {
    #[derive(Clone, Copy)]
    enum Expected {
        Transform,
        Decline,
    }
    type ConformanceCase<'a> = (&'a [&'a str], i32, &'a [u8], &'a [u8], Expected);
    let binary = b"PASS widget_spec\nraw widget_spec:\0\xff\n";
    let cases: &[ConformanceCase<'_>] = &[
        (
            &["acme", "test"],
            0,
            b"PASS widget_spec\nPASS other\nFAIL widget_spec owner\n",
            b"WARN deprecated flag\nWARN deprecated flag\n",
            Expected::Transform,
        ),
        (
            &["acme", "test"],
            1,
            binary,
            b"WARN deprecated \xff\nWARN deprecated \xff\n",
            Expected::Transform,
        ),
        (
            &["acme", "build"],
            1,
            b"COMPILE widget_spec\nCOMPILE other\nERROR widget_spec\n",
            b"WARN deprecated flag\nWARN deprecated flag\n",
            Expected::Transform,
        ),
        (
            &["unsupported", "shape"],
            0,
            b"PASS widget_spec\nPASS other\nFAIL widget_spec owner\n",
            b"",
            Expected::Decline,
        ),
    ];
    let path = trusted_plugin_path(path)?;
    let digest = sha256(&path)?;
    let directory = state_dir()?;
    fs::create_dir_all(&directory)?;
    let snapshot = ExecutableSnapshot::create(&path, &digest, &directory)?;
    for (argv, status, out, err, transform) in cases {
        let argv = argv.iter().map(OsString::from).collect::<Vec<_>>();
        match (
            run_plugin(
                snapshot.path(),
                id,
                &argv,
                *status,
                crate::process::capture::CommandOutcome::Exited(*status),
                out,
                err,
            )?,
            transform,
        ) {
            (PluginResult::Transform(result), Expected::Transform) => {
                if !result
                    .stdout
                    .windows("widget_spec".len())
                    .any(|bytes| bytes == b"widget_spec")
                {
                    return Err(invalid_protocol("transform dropped required evidence"));
                }
                if (out.contains(&0) && !result.stdout.contains(&0))
                    || (out.contains(&0xff) && !result.stdout.contains(&0xff))
                    || (err.contains(&0xff) && !result.stderr.contains(&0xff))
                {
                    return Err(invalid_protocol(
                        "transform did not preserve arbitrary bytes",
                    ));
                }
            }
            (PluginResult::Decline, Expected::Decline) => {}
            _ => return Err(invalid_protocol("unexpected transform/decline result")),
        }
    }
    writeln!(stdout, "{} conforms to tapas-filter v1", path.display())?;
    writeln!(
        stdout,
        "Protocol conformance does not establish trust, safety, or semantic quality."
    )
}

fn kill_group(pid: libc::pid_t) {
    // SAFETY: the plugin was spawned as the leader of its own process group.
    let _ = unsafe { libc::kill(-pid, libc::SIGKILL) };
}

fn decode(value: &Value) -> io::Result<Vec<u8>> {
    let encoded = value
        .as_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing base64 stream"))?;
    let max_encoded = crate::process::MAX_OUTPUT_BYTES.div_ceil(3) * 4 + 4;
    if encoded.len() > max_encoded {
        return Err(invalid_protocol("base64 stream exceeds capture limit"));
    }
    base64::decode(encoded.as_bytes()).map_err(invalid_protocol)
}

fn invalid_protocol(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

fn best_binding<'a>(
    bindings: impl IntoIterator<Item = &'a Value>,
    argv: &[OsString],
) -> Option<&'a Value> {
    bindings
        .into_iter()
        .filter(|binding| matches_prefix(binding, argv))
        .max_by_key(|binding| binding["command"].as_array().map_or(0, Vec::len))
}

fn matches_prefix(binding: &Value, argv: &[OsString]) -> bool {
    let Some(prefix) = binding["command"].as_array() else {
        return false;
    };
    prefix.len() <= argv.len()
        && prefix
            .iter()
            .zip(argv)
            .enumerate()
            .all(|(index, (expected, actual))| {
                expected.as_str().is_some_and(|expected| {
                    let actual = if index == 0 {
                        crate::catalog::command_basename(actual).unwrap_or(actual)
                    } else {
                        actual.as_os_str()
                    };
                    actual.to_str().is_some_and(|actual| actual == expected)
                })
            })
}

fn normalized_prefix(prefix: &[OsString]) -> io::Result<Vec<String>> {
    prefix
        .iter()
        .enumerate()
        .map(|(index, argument)| {
            let argument = if index == 0 {
                crate::catalog::command_basename(argument).unwrap_or(argument)
            } else {
                argument.as_os_str()
            };
            argument.to_str().map(str::to_owned).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "binding arguments must be UTF-8",
                )
            })
        })
        .collect()
}

fn valid_id(id: &OsStr) -> io::Result<&str> {
    let id = id
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "plugin id must be UTF-8"))?;
    let valid = !id.is_empty()
        && id.len() <= 64
        && id.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
        });
    if valid {
        Ok(id)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid plugin id",
        ))
    }
}

fn state_dir() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    Ok(PathBuf::from(home).join(".tapas"))
}

struct StateLock(fs::File);

impl StateLock {
    fn acquire() -> io::Result<Self> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

        let directory = state_dir()?;
        if !directory.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&directory)?;
        }
        let metadata = fs::symlink_metadata(&directory)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "plugin state directory must be private and owned by the current user",
            ));
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join("state.lock"))?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(file))
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn path_key(path: &Path) -> io::Result<&str> {
    path.to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "repository path must be UTF-8"))
}

fn read_state(name: &str) -> io::Result<Value> {
    let directory = state_dir()?;
    if fs::symlink_metadata(&directory).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "plugin state directory must not be a symlink",
        ));
    }
    if let Ok(metadata) = fs::metadata(&directory)
        && (!metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o022 != 0)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "plugin state directory must be private and owned by the current user",
        ));
    }
    let path = directory.join(name);
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "plugin state must not be a symlink",
        ));
    }
    if let Ok(metadata) = fs::metadata(&path)
        && (!metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o022 != 0)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "plugin state must be private and owned by the current user",
        ));
    }
    match read_limited(&path, 1024 * 1024) {
        Ok(bytes) => {
            let value = parse_json(&bytes)?;
            validate_state(name, &value)?;
            Ok(value)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => match name {
            "plugins.json" => Ok(json!({"version": 1, "plugins": Value::Object(Vec::new())})),
            "config.json" => Ok(json!({"version": 1, "bindings": Value::Array(Vec::new())})),
            "projects.json" => Ok(json!({"version": 1, "projects": Value::Object(Vec::new())})),
            _ => Err(invalid_protocol("unknown plugin state file")),
        },
        Err(error) => Err(error),
    }
}

fn validate_state(name: &str, value: &Value) -> io::Result<()> {
    let valid = value["version"] == 1
        && match name {
            "plugins.json" => value["plugins"].as_object().is_some_and(|plugins| {
                plugins.iter().all(|(_, plugin)| {
                    plugin.as_object().is_some()
                        && plugin["path"].is_string()
                        && plugin["sha256"].is_string()
                        && matches!(plugin["pinned"], Value::Bool(_))
                })
            }),
            "config.json" => value["bindings"].as_array().is_some_and(|bindings| {
                bindings.iter().all(|binding| {
                    binding.as_object().is_some()
                        && binding["plugin"].is_string()
                        && binding["command"].as_array().is_some_and(|command| {
                            !command.is_empty() && command.iter().all(Value::is_string)
                        })
                })
            }),
            "projects.json" => value["projects"].as_object().is_some_and(|projects| {
                projects.iter().all(|(_, project)| {
                    project.as_object().is_some() && project["sha256"].is_string()
                })
            }),
            _ => false,
        };
    if valid {
        Ok(())
    } else {
        Err(invalid_protocol("invalid plugin state schema"))
    }
}

fn read_limited(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "plugin state exceeds 1 MiB",
        ));
    }
    Ok(bytes)
}

fn parse_json(bytes: &[u8]) -> io::Result<Value> {
    crate::json::parse(bytes).map_err(invalid_protocol)
}

fn write_state(name: &str, state: &Value) -> io::Result<()> {
    let directory = state_dir()?;
    fs::create_dir_all(&directory)?;
    write_file_atomic(&directory.join(name), &crate::json::serialize(state), 0o600)
}

fn write_file_atomic(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    if bytes.len() > 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "plugin state exceeds 1 MiB",
        ));
    }
    let directory = path.parent().expect("state file parent");
    if fs::symlink_metadata(directory).is_ok_and(|metadata| metadata.file_type().is_symlink())
        || fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "plugin state paths must not be symlinks",
        ));
    }
    if !directory.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
    }
    if path.file_name() != Some(OsStr::new(".tapas.json")) {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    let (pending, mut file) = loop {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let pending = directory.join(format!(
            ".{}.{}.{}.pending",
            path.file_name().unwrap().to_string_lossy(),
            std::process::id(),
            sequence,
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&pending)
        {
            Ok(file) => break (pending, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let publish = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&pending, path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        fs::File::open(directory)?.sync_all()
    })();
    if publish.is_err() {
        let _ = fs::remove_file(&pending);
    }
    publish
}

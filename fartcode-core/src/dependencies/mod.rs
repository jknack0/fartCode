//! Host dependency detection + install/update/uninstall (ticket E3-02).
//!
//! Ports `main/core/dependencies/` for Phase 0: PATH detection with a kv
//! cache (TTL + re-detect on app start), per-manager command builders, and
//! an install runner that streams output to a sink. The visible-PTY install
//! (reference `install-runner.ts`) arrives with E2-06 — the runner's sink is
//! the seam (E2-06 feeds the PTY bytes into it). Network update checks
//! (npm registry / GitHub releases) are a documented stub returning "unknown".

pub mod host_dependency;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::db::Db;
use crate::Error;
use host_dependency::{
    host_dependencies, host_dependency, DependencyStatus, InstallPlan, InstallType,
    DETECTION_TTL_SECS, HOST_DEP_KV_PREFIX,
};

pub use host_dependency::{
    DependencyStatus as HostDependencyStatus, HostDependency, InstallType as InstallTypeDto,
};

/// Where an install/update/uninstall command runs. Phase 0 streams the child
/// output into `sink`; E2-06 swaps in a PTY-backed runner (reference
/// `createLocalInstallCommandRunner`).
pub trait InstallRunner: Send + Sync {
    /// Runs `argv` and streams combined output into `sink`.
    /// Returns Ok(true) on exit-0.
    fn run(&self, argv: &[String], sink: &mut dyn FnMut(&str)) -> Result<bool, Error>;
}

/// std::process-backed runner (Phase 0; no PTY yet).
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessInstallRunner;

impl InstallRunner for ProcessInstallRunner {
    fn run(&self, argv: &[String], sink: &mut dyn FnMut(&str)) -> Result<bool, Error> {
        if argv.is_empty() {
            return Err(Error::Internal("empty install command".into()));
        }
        let mut child = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("CI", "1")
            // Never let an installer consume the app's stdin (`curl | bash`).
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| Error::Internal(format!("failed to spawn install command: {e}")))?;
        // Drain stdout + stderr CONCURRENTLY on threads (a child flooding one
        // pipe >64KB while the other stays open would deadlock a sequential
        // drain). The sink is not Send, so the threads buffer and we flush
        // after both join — E2-06's PTY runner will stream live.
        let mut stdout = child.stdout.take().expect("stdout piped");
        let mut stderr = child.stderr.take().expect("stderr piped");
        let stdout_reader = std::thread::spawn(move || drain_pipe(&mut stdout));
        let stderr_reader = std::thread::spawn(move || drain_pipe(&mut stderr));
        let out = stdout_reader.join().unwrap_or_default();
        let err = stderr_reader.join().unwrap_or_default();
        if !out.is_empty() {
            sink(&out);
        }
        if !err.is_empty() {
            sink(&err);
        }
        let status = child
            .wait()
            .map_err(|e| Error::Internal(format!("install command wait failed: {e}")))?;
        Ok(status.success())
    }
}

/// Per-manager command builders (reference `dependency-managers/`).
pub trait DependencyManager: Send + Sync {
    fn install(&self, plan: &InstallPlan) -> Option<Vec<String>>;
    fn update(&self, plan: &InstallPlan) -> Option<Vec<String>>;
    fn uninstall(&self, plan: &InstallPlan) -> Option<Vec<String>>;
}

pub struct NpmManager;
impl DependencyManager for NpmManager {
    fn install(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.install_args
            .first()
            .map(|pkg| vec!["npm".into(), "install".into(), "-g".into(), pkg.clone()])
    }
    fn update(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.install_args
            .first()
            .map(|pkg| vec!["npm".into(), "update".into(), "-g".into(), pkg.clone()])
    }
    fn uninstall(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.install_args
            .first()
            .map(|pkg| vec!["npm".into(), "uninstall".into(), "-g".into(), pkg.clone()])
    }
}

pub struct PipManager;
impl DependencyManager for PipManager {
    fn install(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.install_args
            .first()
            .map(|pkg| vec!["pip".into(), "install".into(), pkg.clone()])
    }
    fn update(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.install_args
            .first()
            .map(|pkg| vec!["pip".into(), "install".into(), "-U".into(), pkg.clone()])
    }
    fn uninstall(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.install_args
            .first()
            .map(|pkg| vec!["pip".into(), "uninstall".into(), "-y".into(), pkg.clone()])
    }
}

pub struct BrewManager;
impl DependencyManager for BrewManager {
    fn install(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.install_args.first().map(|pkg| {
            vec![
                "brew".into(),
                "install".into(),
                "--cask".into(),
                pkg.clone(),
            ]
        })
    }
    fn update(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.install_args.first().map(|pkg| {
            vec![
                "brew".into(),
                "upgrade".into(),
                "--cask".into(),
                pkg.clone(),
            ]
        })
    }
    fn uninstall(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.install_args.first().map(|pkg| {
            vec![
                "brew".into(),
                "uninstall".into(),
                "--cask".into(),
                pkg.clone(),
            ]
        })
    }
}

/// curl-style installs run the reference's shell one-liner via `sh -c`.
///
/// SECURITY NOTE: these are compile-time-embedded strings from
/// `install_metadata.json` (copied verbatim from the reference); no
/// user input ever reaches the shell. The residual risk is inherent supply
/// chain: `curl -fsSL <vendor> | bash` executes unvetted remote code with
/// the user's privileges (no checksum/TLS pinning) — the reference does the
/// same. Surfaced in the install UI (E1-08) before running.
pub struct CurlManager;
impl DependencyManager for CurlManager {
    fn install(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.shell_command
            .clone()
            .map(|cmd| vec!["sh".into(), "-c".into(), cmd])
    }
    fn update(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        // Update = re-run the installer (idempotent install scripts).
        self.install(plan)
    }
    fn uninstall(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.uninstall_command
            .clone()
            .map(|cmd| vec!["sh".into(), "-c".into(), cmd])
    }
}

/// Cargo / Go / GitHub-release managers (command builders; no providers use
/// them in the extraction, but the ticket lists them).
pub struct CargoManager;
impl DependencyManager for CargoManager {
    fn install(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.install_args
            .first()
            .map(|pkg| vec!["cargo".into(), "install".into(), pkg.clone()])
    }
    fn update(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.install_args.first().map(|pkg| {
            vec![
                "cargo".into(),
                "install".into(),
                "--force".into(),
                pkg.clone(),
            ]
        })
    }
    fn uninstall(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.install_args
            .first()
            .map(|pkg| vec!["cargo".into(), "uninstall".into(), pkg.clone()])
    }
}

pub struct GoManager;
impl DependencyManager for GoManager {
    fn install(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.install_args
            .first()
            .map(|pkg| vec!["go".into(), "install".into(), pkg.clone()])
    }
    fn update(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        self.install(plan)
    }
    fn uninstall(&self, _plan: &InstallPlan) -> Option<Vec<String>> {
        None // `go` has no uninstall; the user removes GOBIN binaries.
    }
}

pub struct GhReleaseManager;
impl DependencyManager for GhReleaseManager {
    fn install(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.shell_command
            .clone()
            .map(|cmd| vec!["sh".into(), "-c".into(), cmd])
    }
    fn update(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        self.install(plan)
    }
    fn uninstall(&self, plan: &InstallPlan) -> Option<Vec<String>> {
        plan.uninstall_command
            .clone()
            .map(|cmd| vec!["sh".into(), "-c".into(), cmd])
    }
}

/// Manager display name for the confirm sheet ("npm", "brew", "curl", …).
fn manager_name(install_type: InstallType) -> &'static str {
    match install_type {
        InstallType::Npm => "npm",
        InstallType::Pip => "pip",
        InstallType::Cargo => "cargo",
        InstallType::Brew => "brew",
        InstallType::Go => "go",
        InstallType::Curl => "curl",
        InstallType::GhRelease => "gh-release",
        InstallType::Other => "shell",
    }
}

/// The exact command the runner executes, as the user should see it before
/// consenting. Shell-backed plans (`sh -c '<one-liner>'`) show the embedded
/// one-liner — that IS the remote code the user is agreeing to run; argv
/// plans show the joined argv.
fn display_command(install_type: InstallType, argv: &[String]) -> String {
    if matches!(
        install_type,
        InstallType::Curl | InstallType::GhRelease | InstallType::Other
    ) {
        argv.get(2).cloned().unwrap_or_else(|| argv.join(" "))
    } else {
        argv.join(" ")
    }
}

fn manager_for(install_type: InstallType) -> Box<dyn DependencyManager> {
    match install_type {
        InstallType::Npm => Box::new(NpmManager),
        InstallType::Pip => Box::new(PipManager),
        InstallType::Cargo => Box::new(CargoManager),
        InstallType::Brew => Box::new(BrewManager),
        InstallType::Go => Box::new(GoManager),
        InstallType::Curl => Box::new(CurlManager),
        InstallType::GhRelease => Box::new(GhReleaseManager),
        InstallType::Other => Box::new(CurlManager),
    }
}

/// E3-02 service: detection + cache + install/update/uninstall.
pub struct HostDependencyStore {
    db: Arc<dyn Db>,
    runner: Arc<dyn InstallRunner>,
}

impl HostDependencyStore {
    pub fn new(db: Arc<dyn Db>, runner: Arc<dyn InstallRunner>) -> Self {
        Self { db, runner }
    }

    /// Re-detect every dependency (app start / explicit refresh). Only
    /// installed binaries spawn a version probe, so this is cheap.
    ///
    /// The app bootstrap (Tauri shell, E5) calls this once at startup so a
    /// PATH change between runs is picked up within the first launch (the
    /// kv cache alone would serve a stale entry for up to `DETECTION_TTL_SECS`).
    pub fn refresh(&self) -> Result<Vec<DependencyStatus>, Error> {
        let mut out = Vec::new();
        for dep in host_dependencies() {
            out.push(self.detect(&dep.id)?);
        }
        Ok(out)
    }

    /// `detect(id)`: PATH scan + version probe, cached under
    /// `host-dep:<id>` (TTL'd).
    pub fn detect(&self, provider_id: &str) -> Result<DependencyStatus, Error> {
        let dep =
            host_dependency(provider_id).ok_or_else(|| Error::AgentNotFound(provider_id.into()))?;
        let status = detect_one(&dep);
        let status = self.cache(&status)?;
        Ok(status)
    }

    /// Cached status; re-detects when the entry is stale (TTL) or absent.
    pub fn get_status(&self, provider_id: &str) -> Result<DependencyStatus, Error> {
        if let Some(cached) = self.read_cache(provider_id)? {
            if !cache_stale(&cached) {
                return Ok(cached);
            }
        }
        self.detect(provider_id)
    }

    /// (manager, exact command) for the confirm sheet before an install.
    pub fn install_command(&self, provider_id: &str) -> Option<(String, String)> {
        let dep = host_dependency(provider_id)?;
        let plan = dep.install_plan.as_ref()?;
        let argv = manager_for(plan.install_type).install(plan)?;
        Some((
            manager_name(plan.install_type).into(),
            display_command(plan.install_type, &argv),
        ))
    }

    /// (manager, exact command) for the confirm sheet before an update.
    pub fn update_command(&self, provider_id: &str) -> Option<(String, String)> {
        let dep = host_dependency(provider_id)?;
        let plan = dep.install_plan.as_ref()?;
        let argv = manager_for(plan.install_type).update(plan)?;
        Some((
            manager_name(plan.install_type).into(),
            display_command(plan.install_type, &argv),
        ))
    }

    /// Reference install instructions for a not-installed dependency.
    pub fn install_instructions(&self, provider_id: &str) -> Option<String> {
        let dep = host_dependency(provider_id)?;
        let plan = dep.install_plan?;
        match plan.install_type {
            InstallType::Npm => plan
                .install_args
                .first()
                .map(|pkg| format!("npm install -g {pkg}")),
            InstallType::Curl => plan.shell_command.clone(),
            InstallType::Brew => plan
                .install_args
                .first()
                .map(|pkg| format!("brew install --cask {pkg}")),
            InstallType::Pip => plan
                .install_args
                .first()
                .map(|pkg| format!("pip install {pkg}")),
            InstallType::Cargo => plan
                .install_args
                .first()
                .map(|pkg| format!("cargo install {pkg}")),
            InstallType::Go => plan
                .install_args
                .first()
                .map(|pkg| format!("go install {pkg}")),
            InstallType::GhRelease | InstallType::Other => plan.shell_command.clone(),
        }
    }

    /// Install the agent CLI; streams output to `sink` and re-detects on
    /// success (cache updated). Network-stubbed updates return `None` from
    /// `latest_version` until the npm/GitHub clients land.
    pub fn install(
        &self,
        provider_id: &str,
        sink: &mut dyn FnMut(&str),
    ) -> Result<DependencyStatus, Error> {
        let dep =
            host_dependency(provider_id).ok_or_else(|| Error::AgentNotFound(provider_id.into()))?;
        let plan = dep
            .install_plan
            .as_ref()
            .ok_or_else(|| Error::Internal(format!("no installer for {provider_id}")))?;
        let argv = manager_for(plan.install_type)
            .install(plan)
            .ok_or_else(|| Error::Internal(format!("no install command for {provider_id}")))?;
        let ok = self.runner.run(&argv, sink)?;
        if !ok {
            return Err(Error::Internal(format!(
                "install failed for {provider_id} (exit != 0)"
            )));
        }
        self.detect(provider_id)
    }

    pub fn update(
        &self,
        provider_id: &str,
        sink: &mut dyn FnMut(&str),
    ) -> Result<DependencyStatus, Error> {
        let dep =
            host_dependency(provider_id).ok_or_else(|| Error::AgentNotFound(provider_id.into()))?;
        let plan = dep
            .install_plan
            .as_ref()
            .ok_or_else(|| Error::Internal(format!("no installer for {provider_id}")))?;
        let argv = manager_for(plan.install_type)
            .update(plan)
            .ok_or_else(|| Error::Internal(format!("no update command for {provider_id}")))?;
        let ok = self.runner.run(&argv, sink)?;
        if !ok {
            return Err(Error::Internal(format!(
                "update failed for {provider_id} (exit != 0)"
            )));
        }
        self.detect(provider_id)
    }

    pub fn uninstall(
        &self,
        provider_id: &str,
        sink: &mut dyn FnMut(&str),
    ) -> Result<DependencyStatus, Error> {
        let dep =
            host_dependency(provider_id).ok_or_else(|| Error::AgentNotFound(provider_id.into()))?;
        let plan = dep
            .install_plan
            .as_ref()
            .ok_or_else(|| Error::Internal(format!("no installer for {provider_id}")))?;
        let argv = manager_for(plan.install_type)
            .uninstall(plan)
            .ok_or_else(|| Error::Internal(format!("no uninstall command for {provider_id}")))?;
        let ok = self.runner.run(&argv, sink)?;
        if !ok {
            return Err(Error::Internal(format!(
                "uninstall failed for {provider_id} (exit != 0)"
            )));
        }
        let status =
            DependencyStatus::not_installed(provider_id, self.install_instructions(provider_id));
        self.cache(&status)?;
        Ok(status)
    }

    /// `latest_version(provider_id)`: npm registry / GitHub releases lookup.
    /// Phase 0 stub — network clients land with E3-05; returns None (unknown).
    pub fn latest_version(&self, _provider_id: &str) -> Option<String> {
        None
    }

    // -- internals -----------------------------------------------------------

    fn cache(&self, status: &DependencyStatus) -> Result<DependencyStatus, Error> {
        let mut s = status.clone();
        let now = now_unix();
        s.checked_at = Some(now.to_string());
        let value = serde_json::to_string(&s)?;
        self.db
            .kv_set(&format!("{HOST_DEP_KV_PREFIX}{}", s.provider_id), &value)?;
        Ok(s)
    }

    fn read_cache(&self, provider_id: &str) -> Result<Option<DependencyStatus>, Error> {
        let key = format!("{HOST_DEP_KV_PREFIX}{provider_id}");
        let Some(raw) = self.db.kv_get(&key)? else {
            return Ok(None);
        };
        Ok(serde_json::from_str(&raw).ok())
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn cache_stale(status: &DependencyStatus) -> bool {
    let Some(checked) = status
        .checked_at
        .as_deref()
        .and_then(|c| c.parse::<i64>().ok())
    else {
        return true;
    };
    now_unix() - checked > DETECTION_TTL_SECS
}

/// PATH scan + version probe. Uses the caller's PATH (tests inject a fake
/// one via `PATH` env on the probe command).
fn detect_one(dep: &HostDependency) -> DependencyStatus {
    let mut status = DependencyStatus {
        provider_id: dep.id.clone(),
        installed: false,
        version: None,
        path: None,
        checked_at: None,
        install_instructions: None,
    };
    let Some(path_dir) = find_in_path(&dep.detect_paths) else {
        return status;
    };
    status.installed = true;
    status.path = Some(path_dir.0.display().to_string());
    status.version = probe_version(&dep.version_command);
    status
}

/// Drains a child pipe to EOF into a String (used by the concurrent drain
/// threads in `ProcessInstallRunner`).
fn drain_pipe<R: std::io::Read>(pipe: &mut R) -> String {
    let mut buf = [0u8; 4096];
    let mut out = String::new();
    loop {
        match pipe.read(buf.as_mut_slice()) {
            Ok(0) => break,
            Ok(n) => out.push_str(&String::from_utf8_lossy(&buf[..n])),
            Err(_) => break,
        }
    }
    out
}

/// Finds the first `detect_paths` entry present on PATH (executable).
/// Names are the OUTER loop so the first listed binary wins across all
/// directories (reference `detect_paths` ordering).
fn find_in_path(detect_paths: &[String]) -> Option<(PathBuf, String)> {
    let path = std::env::var_os("PATH")?;
    for name in detect_paths {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if is_executable(&candidate) {
                return Some((candidate, name.clone()));
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.is_file()
        && p.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    // Windows: executables carry extensions (.exe/.cmd/.bat/...); check the
    // bare file OR each PATHEXT-extension candidate.
    if p.is_file() {
        return true;
    }
    let pathext = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .to_ascii_lowercase();
    for ext in pathext.split(';').filter(|e| !e.is_empty()) {
        let mut candidate = p.as_os_str().to_os_string();
        candidate.push(ext);
        if Path::new(&candidate).is_file() {
            return true;
        }
    }
    false
}

/// Runs `<binary> --version`, trimmed (first line). Never fails the detection
/// — a version probe failure just yields `version: None`.
fn probe_version(argv: &[String]) -> Option<String> {
    if argv.is_empty() {
        return None;
    }
    let out = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().find(|l| !l.trim().is_empty())?;
    Some(first.trim().to_string())
}

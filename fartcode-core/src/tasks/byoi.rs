//! BYOI ("bring your own infrastructure") remote tasks (E12-07).
//!
//! A project can hand workspace creation to two scripts. *Provision* prints a
//! JSON descriptor of a machine on stdout; *terminate* destroys it. Both run
//! wherever the project lives — this laptop for a local project, the SSH host
//! for a remote one — so the flow is written against one trait
//! ([`ScriptRunner`]) with an implementation per machine, the same split
//! [`crate::projects::remote::RemoteHost`] uses.
//!
//! Three rules the rest of this module leans on:
//!
//! - **The descriptor is a contract, not a suggestion.** Empty output, output
//!   that is not JSON, and JSON without a host are three different errors,
//!   each quotable back to the user who wrote the script.
//! - **A password in the descriptor never leaves this module** — not through
//!   `Debug`, not through an error message.
//! - **Nothing is shell-interpolated.** Env values go through
//!   [`crate::shell_escape::single_quote`]; keys are validated, not quoted.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use rusqlite::OptionalExtension;

use crate::db::Db;
use crate::projects::remote::RemoteOutput;
use crate::settings::registry::WorkspaceProvider;
use crate::shell_escape::single_quote;
use crate::Error;

/// Reference parity: a provisioner may boot a VM, so it gets ten minutes.
/// Terminate gets the same — it may wait on the same infrastructure.
pub const SCRIPT_TIMEOUT: Duration = Duration::from_secs(600);

/// The `workspaceProvider.type` that means "these scripts own the workspace".
pub const SCRIPT_PROVIDER: &str = "script";

/// Env var carrying the provisioned machine's id into the terminate script.
pub const REMOTE_WORKSPACE_ID: &str = "REMOTE_WORKSPACE_ID";

/// What a provision script must print on stdout.
///
/// Deserialization is deliberately lenient about UNKNOWN fields (a script may
/// print extra bookkeeping) and strict about the two that matter.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionOutput {
    /// Opaque machine id, handed back to terminate as `REMOTE_WORKSPACE_ID`.
    pub id: String,
    /// Hostname, IP, or `user@host`.
    pub host: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    /// Where the task's work lives on that machine. `None` means "the
    /// project path".
    pub worktree_path: Option<String>,
    /// Password auth for the provisioned box (AC2: never logged).
    pub password: Option<String>,
    pub forward_agent: Option<bool>,
}

/// Hand-written so a stray `{:?}` — a tracing field, an error wrapper, a test
/// failure dump — cannot print the provisioned machine's password.
impl std::fmt::Debug for ProvisionOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProvisionOutput")
            .field("id", &self.id)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("worktree_path", &self.worktree_path)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("forward_agent", &self.forward_agent)
            .finish()
    }
}

impl ProvisionOutput {
    /// The machine id, unless the script left it blank.
    pub fn machine_id(&self) -> Option<&str> {
        let id = self.id.trim();
        (!id.is_empty()).then_some(id)
    }
}

/// Parses a provision script's stdout (AC1).
///
/// The error text quotes the script's own output, capped — a provisioner that
/// prints a stack trace should show the first lines of it, not paste a
/// megabyte into a toast.
pub fn parse_provision_output(stdout: &str) -> Result<ProvisionOutput, Error> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidTaskInput(
            "provision script printed nothing — expected a JSON descriptor on stdout".into(),
        ));
    }
    let mut output: ProvisionOutput = serde_json::from_str(trimmed).map_err(|e| {
        Error::InvalidTaskInput(format!(
            "provision script output is not the expected JSON ({e}): {}",
            snippet(trimmed)
        ))
    })?;
    output.host = output.host.trim().to_string();
    if output.host.is_empty() {
        return Err(Error::InvalidTaskInput(
            "provision script output needs a non-empty \"host\"".into(),
        ));
    }
    Ok(output)
}

fn snippet(text: &str) -> String {
    const CAP: usize = 200;
    if text.chars().count() <= CAP {
        return text.to_string();
    }
    let head: String = text.chars().take(CAP).collect();
    format!("{head}…")
}

/// Where a provision/terminate script runs.
///
/// One method, because that is the whole surface: the flow never needs a
/// filesystem, only "run this shell line and tell me how it went". Local and
/// SSH implementations differ in the machine, not the contract.
#[async_trait::async_trait]
pub trait ScriptRunner: Send + Sync {
    /// Runs `command` through `/bin/sh -c` in the project directory, with
    /// `env` exported for the duration.
    ///
    /// Implementations must fail with [`Error::Internal`] naming the timeout
    /// when `timeout` elapses, and must not leave the child running.
    async fn run_script(
        &self,
        command: &str,
        env: &[(String, String)],
        timeout: Duration,
    ) -> Result<RemoteOutput, Error>;
}

/// Prefixes `command` with `KEY='value'` assignments (AC5).
///
/// Values are single-quoted; keys are VALIDATED rather than quoted, because a
/// shell assignment cannot quote its left-hand side — a key that is not a
/// plain identifier is a bug in our own call site, not user data.
pub fn script_command_line(command: &str, env: &[(String, String)]) -> Result<String, Error> {
    let mut line = String::new();
    for (key, value) in env {
        if key.is_empty()
            || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || key.starts_with(|c: char| c.is_ascii_digit())
        {
            return Err(Error::Internal(format!(
                "invalid environment variable name for script: {key}"
            )));
        }
        line.push_str(key);
        line.push('=');
        line.push_str(&single_quote(value));
        line.push(' ');
    }
    line.push_str(command);
    Ok(line)
}

/// Runs the project's provision script and parses its descriptor (AC1/AC3).
pub async fn provision(
    runner: &dyn ScriptRunner,
    provider: &WorkspaceProvider,
) -> Result<ProvisionOutput, Error> {
    if provider.r#type != SCRIPT_PROVIDER {
        return Err(Error::InvalidTaskInput(format!(
            "workspace provider is '{}', not '{SCRIPT_PROVIDER}' — nothing to provision",
            provider.r#type
        )));
    }
    let command = provider
        .provision_command
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| {
            Error::InvalidTaskInput(
                "workspace provider 'script' has no provision command configured".into(),
            )
        })?;

    let output = runner.run_script(command, &[], SCRIPT_TIMEOUT).await?;
    if !output.ok() {
        return Err(Error::Internal(format!(
            "provision script failed ({}): {}",
            output.exit_code,
            snippet(output.stderr.trim())
        )));
    }
    parse_provision_output(&output.stdout)
}

/// Runs the project's terminate script (AC4).
///
/// Never fails. Teardown runs while a task is being deleted, and a machine
/// that is already gone, a script that exits nonzero, or a host that has
/// stopped answering must not strand the task in a half-deleted state. Each
/// of those warns; the caller continues.
pub async fn terminate(
    runner: &dyn ScriptRunner,
    provider: &WorkspaceProvider,
    remote_workspace_id: Option<&str>,
) {
    let Some(command) = provider
        .terminate_command
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    else {
        return;
    };
    let env: Vec<(String, String)> = remote_workspace_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| vec![(REMOTE_WORKSPACE_ID.to_string(), id.to_string())])
        .unwrap_or_default();

    match runner.run_script(command, &env, SCRIPT_TIMEOUT).await {
        Ok(output) if output.ok() => {}
        Ok(output) => tracing::warn!(
            exit_code = output.exit_code,
            stderr = %snippet(output.stderr.trim()),
            "terminate script failed — continuing teardown"
        ),
        Err(error) => tracing::warn!(
            error = %error,
            "terminate script could not run — continuing teardown"
        ),
    }
}

/// `/bin/sh`-backed runner for scripts that belong on THIS machine (a local
/// project). Remote projects use `fartcode_ssh`'s implementation instead.
#[derive(Debug, Default, Clone)]
pub struct LocalScriptRunner {
    /// Working directory: the project root.
    root: std::path::PathBuf,
}

impl LocalScriptRunner {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[async_trait::async_trait]
impl ScriptRunner for LocalScriptRunner {
    async fn run_script(
        &self,
        command: &str,
        env: &[(String, String)],
        timeout: Duration,
    ) -> Result<RemoteOutput, Error> {
        let child = tokio::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .current_dir(&self.root)
            .envs(env.iter().map(|(k, v)| (k.clone(), v.clone())))
            // A provisioner must never consume the app's stdin (same rule as
            // the dependency installer).
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::Internal(format!("failed to run script: {e}")))?;

        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(result) => {
                let output =
                    result.map_err(|e| Error::Internal(format!("script wait failed: {e}")))?;
                Ok(RemoteOutput {
                    exit_code: output.status.code().unwrap_or(-1),
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                })
            }
            // `kill_on_drop` reaps the child when the future is dropped here,
            // so a timed-out provisioner does not outlive the app.
            Err(_) => Err(Error::Internal(format!(
                "script timed out after {}s",
                timeout.as_secs()
            ))),
        }
    }
}

// ── Task-side state (E12-10) ──────────────────────────────────────

/// A task's BYOI workspace row, and what provisioning has recorded on it.
///
/// `remote_workspace_id.is_some()` is the "already provisioned" test: the
/// provision command is idempotent, and re-running a script that boots a VM
/// would leak the first one.
#[derive(Debug, Clone, PartialEq)]
pub struct ByoiWorkspace {
    pub workspace_id: String,
    pub remote_workspace_id: Option<String>,
    pub ssh_connection_id: Option<String>,
    pub path: Option<String>,
}

/// `(workspace id, config JSON, ssh connection id, path)` as stored.
type ByoiRow = (String, Option<String>, Option<String>, Option<String>);

/// The task's BYOI workspace, or `None` when the task's workspace is an
/// ordinary worktree / project-root row.
pub fn byoi_workspace_for_task(db: &dyn Db, task_id: &str) -> Result<Option<ByoiWorkspace>, Error> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    let row: Option<ByoiRow> = conn
        .query_row(
            "SELECT w.id, w.config, w.ssh_connection_id, w.path
               FROM tasks t JOIN workspaces w ON w.id = t.workspace_id
              WHERE t.id = ?1 AND w.kind = 'byoi'",
            [task_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    Ok(row.map(
        |(workspace_id, config, ssh_connection_id, path)| ByoiWorkspace {
            workspace_id,
            remote_workspace_id: config
                .as_deref()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok())
                .and_then(|v| {
                    v.get("workspace")
                        .and_then(|w| w.get("remoteWorkspaceId"))
                        .and_then(|id| id.as_str())
                        .map(String::from)
                })
                .filter(|id| !id.trim().is_empty()),
            ssh_connection_id,
            path,
        },
    ))
}

/// Records the machine a provision script just described, on the workspace
/// row itself.
///
/// The row — not an in-memory registry — is what survives a restart, and it
/// is what teardown reads to know which machine to destroy. `location` and
/// `ssh_connection_id` follow the same convention as remote projects
/// (E12-04), so terminals and agents route over SSH without a second rule.
pub fn record_provisioned_machine(
    db: &dyn Db,
    workspace_id: &str,
    machine_id: &str,
    ssh_connection_id: &str,
    path: Option<&str>,
) -> Result<(), Error> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    let config: Option<String> = conn
        .query_row(
            "SELECT config FROM workspaces WHERE id = ?1",
            [workspace_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| Error::TaskNotFound(format!("workspace {workspace_id}")))?;

    let mut value: serde_json::Value = config
        .as_deref()
        .and_then(|c| serde_json::from_str(c).ok())
        .unwrap_or_else(|| serde_json::json!({ "version": "2" }));
    if !value["workspace"].is_object() {
        value["workspace"] = serde_json::json!({ "kind": "byoi" });
    }
    value["workspace"]["remoteWorkspaceId"] = serde_json::json!(machine_id);

    conn.execute(
        "UPDATE workspaces
            SET config = ?1,
                ssh_connection_id = ?2,
                path = COALESCE(?3, path),
                type = 'byoi',
                location = 'remote',
                updated_at = datetime('now')
          WHERE id = ?4",
        rusqlite::params![value.to_string(), ssh_connection_id, path, workspace_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn provider(provision: Option<&str>, terminate: Option<&str>) -> WorkspaceProvider {
        WorkspaceProvider {
            r#type: SCRIPT_PROVIDER.into(),
            provision_command: provision.map(String::from),
            terminate_command: terminate.map(String::from),
        }
    }

    /// One recorded `run_script` call: command, env, budget.
    type ScriptCall = (String, Vec<(String, String)>, Duration);

    /// Records what it was asked to run and replays a canned result.
    struct FakeRunner {
        result: Result<RemoteOutput, Error>,
        calls: Mutex<Vec<ScriptCall>>,
    }

    impl FakeRunner {
        fn ok(stdout: &str) -> Self {
            Self::with(RemoteOutput {
                exit_code: 0,
                stdout: stdout.into(),
                stderr: String::new(),
            })
        }

        fn with(output: RemoteOutput) -> Self {
            Self {
                result: Ok(output),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn failing() -> Self {
            Self {
                result: Err(Error::Internal("host unreachable".into())),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl ScriptRunner for FakeRunner {
        async fn run_script(
            &self,
            command: &str,
            env: &[(String, String)],
            timeout: Duration,
        ) -> Result<RemoteOutput, Error> {
            self.calls
                .lock()
                .unwrap()
                .push((command.to_string(), env.to_vec(), timeout));
            match &self.result {
                Ok(output) => Ok(output.clone()),
                Err(e) => Err(Error::Internal(e.to_string())),
            }
        }
    }

    #[test]
    fn parses_the_full_descriptor() {
        let output = parse_provision_output(
            r#"{"id":"vm-1","host":" build@10.0.0.4 ","port":2222,"username":"ci",
                "worktreePath":"/srv/work","password":"hunter2","forwardAgent":true,
                "extra":"ignored"}"#,
        )
        .unwrap();
        assert_eq!(output.id, "vm-1");
        assert_eq!(output.host, "build@10.0.0.4");
        assert_eq!(output.port, Some(2222));
        assert_eq!(output.worktree_path.as_deref(), Some("/srv/work"));
        assert_eq!(output.forward_agent, Some(true));
    }

    #[test]
    fn minimal_descriptor_is_enough() {
        let output = parse_provision_output("{\"id\":\"\",\"host\":\"box\"}\n").unwrap();
        assert_eq!(output.host, "box");
        assert_eq!(output.port, None);
        // A blank id is not a machine id — terminate must not export it.
        assert_eq!(output.machine_id(), None);
    }

    /// AC1: the three failure modes stay distinguishable.
    #[test]
    fn contract_failures_are_distinct() {
        let empty = parse_provision_output("   \n").unwrap_err().to_string();
        assert!(empty.contains("printed nothing"), "{empty}");

        let garbage = parse_provision_output("provisioning...")
            .unwrap_err()
            .to_string();
        assert!(garbage.contains("not the expected JSON"), "{garbage}");
        assert!(garbage.contains("provisioning..."), "{garbage}");

        let hostless = parse_provision_output("{\"id\":\"vm-1\",\"host\":\"  \"}")
            .unwrap_err()
            .to_string();
        assert!(hostless.contains("non-empty"), "{hostless}");
    }

    /// AC2: the password is in the struct and nowhere else.
    #[test]
    fn debug_redacts_the_password() {
        let output =
            parse_provision_output("{\"id\":\"1\",\"host\":\"h\",\"password\":\"hunter2\"}")
                .unwrap();
        let debug = format!("{output:?}");
        assert!(!debug.contains("hunter2"), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
        assert_eq!(output.password.as_deref(), Some("hunter2"));
    }

    #[test]
    fn long_garbage_output_is_capped() {
        let error = parse_provision_output(&"x".repeat(5_000))
            .unwrap_err()
            .to_string();
        assert!(error.contains('…'));
        assert!(error.len() < 400, "error was {} chars", error.len());
    }

    #[tokio::test]
    async fn provision_runs_the_command_with_the_ten_minute_budget() {
        let runner = FakeRunner::ok("{\"id\":\"vm-1\",\"host\":\"box\"}");
        let output = provision(&runner, &provider(Some(" ./provision.sh "), None))
            .await
            .unwrap();
        assert_eq!(output.id, "vm-1");
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls[0].0, "./provision.sh");
        assert!(calls[0].1.is_empty());
        assert_eq!(calls[0].2, Duration::from_secs(600));
    }

    #[tokio::test]
    async fn provision_refuses_a_non_script_provider() {
        let runner = FakeRunner::ok("{}");
        let mut wp = provider(Some("./p.sh"), None);
        wp.r#type = "local".into();
        assert!(provision(&runner, &wp).await.is_err());
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn provision_surfaces_a_failing_script() {
        let runner = FakeRunner::with(RemoteOutput {
            exit_code: 3,
            stdout: String::new(),
            stderr: "no capacity".into(),
        });
        let error = provision(&runner, &provider(Some("./p.sh"), None))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("(3)"), "{error}");
        assert!(error.contains("no capacity"), "{error}");
    }

    /// AC4: the id is exported (quoted at the shell layer), and a blank id or
    /// missing command is a silent no-op.
    #[tokio::test]
    async fn terminate_exports_the_machine_id() {
        let runner = FakeRunner::ok("");
        terminate(
            &runner,
            &provider(None, Some("./terminate.sh")),
            Some("vm-1"),
        )
        .await;
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls[0].0, "./terminate.sh");
        assert_eq!(
            calls[0].1,
            vec![(REMOTE_WORKSPACE_ID.to_string(), "vm-1".to_string())]
        );
    }

    #[tokio::test]
    async fn terminate_without_a_command_does_nothing() {
        let runner = FakeRunner::ok("");
        terminate(
            &runner,
            &provider(Some("./p.sh"), Some("   ")),
            Some("vm-1"),
        )
        .await;
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn terminate_without_an_id_runs_bare() {
        let runner = FakeRunner::ok("");
        terminate(&runner, &provider(None, Some("./t.sh")), Some("  ")).await;
        assert!(runner.calls.lock().unwrap()[0].1.is_empty());
    }

    /// AC4: teardown continues through a failing script AND an unreachable
    /// host — neither returns, because neither can.
    #[tokio::test]
    async fn terminate_swallows_failures() {
        let failed = FakeRunner::with(RemoteOutput {
            exit_code: 1,
            stdout: String::new(),
            stderr: "already gone".into(),
        });
        terminate(&failed, &provider(None, Some("./t.sh")), Some("vm-1")).await;

        let unreachable = FakeRunner::failing();
        terminate(&unreachable, &provider(None, Some("./t.sh")), Some("vm-1")).await;
        // Both ran; neither panicked or returned an error to the caller.
        assert_eq!(failed.calls.lock().unwrap().len(), 1);
        assert_eq!(unreachable.calls.lock().unwrap().len(), 1);
    }

    /// AC5: values are quoted, keys are validated.
    #[test]
    fn env_values_are_quoted_not_interpolated() {
        let line = script_command_line(
            "./t.sh",
            &[(REMOTE_WORKSPACE_ID.into(), "vm'; rm -rf /; echo '".into())],
        )
        .unwrap();
        assert_eq!(
            line,
            r#"REMOTE_WORKSPACE_ID='vm'\''; rm -rf /; echo '\''' ./t.sh"#
        );
    }

    #[test]
    fn invalid_env_keys_are_refused() {
        for key in ["", "2FOO", "FOO BAR", "FOO=BAR", "FOO;"] {
            assert!(
                script_command_line("./t.sh", &[(key.into(), "v".into())]).is_err(),
                "key {key:?} should be refused"
            );
        }
        assert!(script_command_line("./t.sh", &[("FOO_1".into(), "v".into())]).is_ok());
    }

    // ── Local runner ────────────────────────────────────────

    #[tokio::test]
    async fn local_runner_collects_output_and_env() {
        let runner = LocalScriptRunner::new(std::env::temp_dir());
        let output = runner
            .run_script(
                "printf '%s' \"$REMOTE_WORKSPACE_ID\"; echo oops >&2",
                &[(REMOTE_WORKSPACE_ID.into(), "vm-9".into())],
                Duration::from_secs(10),
            )
            .await
            .unwrap();
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout, "vm-9");
        assert_eq!(output.stderr.trim(), "oops");
    }

    #[tokio::test]
    async fn local_runner_reports_exit_codes() {
        let runner = LocalScriptRunner::new(std::env::temp_dir());
        let output = runner
            .run_script("exit 7", &[], Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(output.exit_code, 7);
    }

    /// AC3: a hanging script is an error naming the timeout, and the child
    /// does not survive it (`kill_on_drop`).
    #[tokio::test]
    async fn local_runner_times_out() {
        let runner = LocalScriptRunner::new(std::env::temp_dir());
        let error = runner
            .run_script("sleep 30", &[], Duration::from_millis(150))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("timed out"), "{error}");
    }

    // ── Task-side state ─────────────────────────────────────────

    fn db_with_task(kind: &str, config: Option<&str>) -> std::sync::Arc<dyn crate::db::Db> {
        let db = crate::db::SqliteDb::init_in_memory().unwrap();
        {
            let conn = db.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO workspaces (id, kind, config) VALUES ('w1', ?1, ?2)",
                rusqlite::params![kind, config],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'p', '/tmp/p1')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, name, status, workspace_id)
                 VALUES ('t1', 'p1', 't', 'todo', 'w1')",
                [],
            )
            .unwrap();
        }
        db
    }

    #[test]
    fn byoi_workspace_is_none_for_a_worktree_task() {
        let db = db_with_task("worktree", None);
        assert_eq!(byoi_workspace_for_task(db.as_ref(), "t1").unwrap(), None);
    }

    #[test]
    fn unprovisioned_byoi_workspace_has_no_machine() {
        let db = db_with_task(
            "byoi",
            Some(r#"{"version":"2","workspace":{"kind":"byoi"}}"#),
        );
        let ws = byoi_workspace_for_task(db.as_ref(), "t1").unwrap().unwrap();
        assert_eq!(ws.workspace_id, "w1");
        assert_eq!(ws.remote_workspace_id, None);
        assert_eq!(ws.ssh_connection_id, None);
    }

    /// The recorded machine survives as ROW state — what teardown reads after
    /// a restart, when no registry remembers anything.
    #[test]
    fn recording_a_machine_makes_the_workspace_remote() {
        let db = db_with_task(
            "byoi",
            Some(r#"{"version":"2","workspace":{"kind":"byoi"}}"#),
        );
        record_provisioned_machine(db.as_ref(), "w1", "vm-1", "task:t1", Some("/srv/work"))
            .unwrap();

        let ws = byoi_workspace_for_task(db.as_ref(), "t1").unwrap().unwrap();
        assert_eq!(ws.remote_workspace_id.as_deref(), Some("vm-1"));
        assert_eq!(ws.ssh_connection_id.as_deref(), Some("task:t1"));
        assert_eq!(ws.path.as_deref(), Some("/srv/work"));

        let (kind, location): (String, String) = {
            let conn = db.conn().lock().unwrap();
            conn.query_row(
                "SELECT kind, location FROM workspaces WHERE id = 'w1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        // The row keeps its kind (the intent) and gains the transport.
        assert_eq!(kind, "byoi");
        assert_eq!(location, "remote");
    }

    /// A row whose config was never written still records cleanly — legacy
    /// byoi rows predate the versioned config.
    #[test]
    fn recording_repairs_a_configless_row() {
        let db = db_with_task("byoi", None);
        record_provisioned_machine(db.as_ref(), "w1", "vm-2", "task:t1", None).unwrap();
        let ws = byoi_workspace_for_task(db.as_ref(), "t1").unwrap().unwrap();
        assert_eq!(ws.remote_workspace_id.as_deref(), Some("vm-2"));
        assert_eq!(ws.path, None);
    }

    #[test]
    fn recording_against_a_missing_workspace_fails() {
        let db = db_with_task("byoi", None);
        assert!(record_provisioned_machine(db.as_ref(), "nope", "vm", "c", None).is_err());
    }
}

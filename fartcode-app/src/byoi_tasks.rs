//! BYOI task wiring (E12-10): the seam between a task and the machine its
//! project's scripts provision.
//!
//! The contract and both runners are E12-07 (`fartcode_core::tasks::byoi`,
//! `fartcode_ssh::byoi`); this module decides WHEN they run, WHERE (the
//! project's own machine), and what the app records afterwards.
//!
//! **Feature gate.** Remote tasks ship dark: [`ENABLED`] is the `remote-tasks`
//! cargo feature, off by default. The gate is enforced at provision — the one
//! place that spends money on someone else's infrastructure — rather than at
//! settings-write, so a project configured on an enabled build stays readable
//! (and terminable) on a disabled one.

use std::sync::Arc;

use fartcode_core::projects::model::WorkspaceProviderKind;
use fartcode_core::projects::ProjectStore;
use fartcode_core::settings::registry::WorkspaceProvider;
use fartcode_core::tasks::byoi::{
    self, byoi_workspace_for_task, record_provisioned_machine, LocalScriptRunner, ScriptRunner,
    SCRIPT_PROVIDER,
};
use fartcode_core::tasks::TaskStore;
use fartcode_core::Error;
use fartcode_ssh::byoi::SshScriptRunner;
use fartcode_ssh::host::SshRemoteHost;

use crate::app::App;

/// Whether this build can provision BYOI machines (`remote-tasks` feature).
pub const ENABLED: bool = cfg!(feature = "remote-tasks");

/// Connection id for a task's provisioned machine. Namespaced so it can never
/// collide with a saved profile's UUID.
fn connection_id_for(task_id: &str) -> String {
    format!("task:{task_id}")
}

/// Provisions the task's BYOI machine, if it has one (E12-10).
///
/// Three no-ops, in order: the task's workspace is not BYOI, the machine is
/// already provisioned (re-provision is idempotent — running a script that
/// boots a VM twice leaks the first one), or the project has no script
/// provider configured.
pub fn provision(app: &App, task_id: &str) -> Result<(), Error> {
    let Some(workspace) = byoi_workspace_for_task(app.db.as_ref(), task_id)? else {
        return Ok(());
    };
    if workspace.ssh_connection_id.is_some() {
        return Ok(());
    }
    if !ENABLED {
        return Err(Error::InvalidTaskInput(
            "remote tasks are not enabled in this build (feature `remote-tasks`)".into(),
        ));
    }
    let (provider, runner) = script_context(app, task_id)?;

    let output = tauri::async_runtime::block_on(byoi::provision(runner.as_ref(), &provider))?;
    let connection_id = connection_id_for(task_id);
    let params = fartcode_ssh::byoi::connect_params(&output, ssh_auth_sock().as_deref())?;

    // Register BEFORE recording: a row pointing at a connection the pool has
    // never heard of would route terminals into a `SshConnectionNotFound`.
    app.remote_pty.register_transient(&connection_id, params);
    record_provisioned_machine(
        app.db.as_ref(),
        &workspace.workspace_id,
        output.machine_id().unwrap_or_default(),
        &connection_id,
        output.worktree_path.as_deref(),
    )?;
    tracing::info!(
        task_id,
        connection_id,
        machine = output.machine_id().unwrap_or("<unnamed>"),
        "provisioned BYOI workspace"
    );
    Ok(())
}

/// Tears the task's BYOI machine down, if it has one (E12-10).
///
/// Runs during task deletion, so — like [`byoi::terminate`] itself — it never
/// returns an error: a machine that cannot be destroyed must not leave a task
/// undeletable. The gate is deliberately NOT checked: a build without the
/// feature must still be able to clean up what an enabled build created.
pub fn terminate(app: &App, task_id: &str) {
    let workspace = match byoi_workspace_for_task(app.db.as_ref(), task_id) {
        Ok(Some(workspace)) => workspace,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(task_id, error = %error, "could not read BYOI workspace for teardown");
            return;
        }
    };
    let Some(connection_id) = workspace.ssh_connection_id.clone() else {
        return; // never provisioned
    };

    let warning = match script_context(app, task_id) {
        Ok((provider, runner)) => tauri::async_runtime::block_on(byoi::terminate(
            runner.as_ref(),
            &provider,
            workspace.remote_workspace_id.as_deref(),
        )),
        Err(error) => {
            tracing::warn!(task_id, error = %error, "no terminate context — machine may leak");
            Some(format!(
                "no terminate context: {error} — the provisioned machine may leak"
            ))
        }
    };
    // ADR-0044's deferred call: the warning reaches the USER, because the
    // machine that may have leaked is billed to them.
    if let Some(message) = warning {
        use fartcode_core::events::{EventBus, InternalEvent};
        app.event_bus.send(InternalEvent::ByoiTerminateWarning {
            task_id: task_id.to_string(),
            message,
        });
    }
    // The session goes regardless: whatever happened to the machine, this
    // process has no further business holding a connection to it.
    app.remote_pty.forget_transient(&connection_id);
}

/// The project's script provider plus a runner pointed at the machine the
/// project lives on: this laptop for a local project, the project's SSH host
/// for a remote one.
fn script_context(
    app: &App,
    task_id: &str,
) -> Result<(WorkspaceProvider, Box<dyn ScriptRunner>), Error> {
    let task = app
        .tasks
        .get(task_id)?
        .ok_or_else(|| Error::TaskNotFound(task_id.into()))?;
    let project = app
        .projects
        .get(&task.project_id)?
        .ok_or_else(|| Error::ProjectNotFound(task.project_id.clone()))?;
    let settings = app
        .settings
        .get_project_settings(&project.id, &project.path)?;
    let provider = settings
        .workspace_provider
        .filter(|wp| wp.r#type == SCRIPT_PROVIDER)
        .ok_or_else(|| {
            Error::InvalidTaskInput(format!(
                "project {} has no '{SCRIPT_PROVIDER}' workspace provider configured",
                project.id
            ))
        })?;

    let runner: Box<dyn ScriptRunner> = match project.workspace_provider {
        WorkspaceProviderKind::Local => Box::new(LocalScriptRunner::new(project.path.clone())),
        WorkspaceProviderKind::Ssh => {
            let connection_id = project.ssh_connection_id.clone().ok_or_else(|| {
                Error::Internal(format!("ssh project {} has no connection", project.id))
            })?;
            let client = tauri::async_runtime::block_on(app.remote_pty.client_for(&connection_id))?;
            let host = tauri::async_runtime::block_on(SshRemoteHost::new(client))?;
            Box::new(SshScriptRunner::new(
                Arc::new(host),
                Some(project.path.to_string_lossy().into_owned()),
            ))
        }
    };
    Ok((provider, runner))
}

/// The agent socket a provisioned machine may need for `forwardAgent`.
fn ssh_auth_sock() -> Option<String> {
    std::env::var("SSH_AUTH_SOCK")
        .ok()
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_task(kind: &str) -> Arc<App> {
        let app = App::init(Some(":memory:")).unwrap();
        let handle = app.db.conn();
        let conn = handle.lock().unwrap();
        conn.execute(
            "INSERT INTO projects (id, name, path) VALUES ('p1', 'p', '/tmp/p1')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO workspaces (id, kind, config) VALUES ('w1', ?1, '{\"version\":\"2\",\"workspace\":{\"kind\":\"byoi\"}}')",
            [kind],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, project_id, name, status, workspace_id)
             VALUES ('t1', 'p1', 't', 'todo', 'w1')",
            [],
        )
        .unwrap();
        drop(conn);
        app
    }

    fn recorded_connection(app: &App) -> Option<String> {
        let handle = app.db.conn();
        let conn = handle.lock().unwrap();
        conn.query_row(
            "SELECT ssh_connection_id FROM workspaces WHERE id = 'w1'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn connection_ids_are_namespaced() {
        assert_eq!(connection_id_for("t1"), "task:t1");
    }

    /// The gate ships off. Read through a binding so this is a real
    /// assertion about the default build, not a folded constant: turning the
    /// feature on by accident in CI fails here.
    #[test]
    #[cfg_attr(feature = "remote-tasks", ignore)]
    fn remote_tasks_are_off_by_default() {
        let enabled: bool = ENABLED;
        assert!(!enabled, "`remote-tasks` must stay opt-in per build");
    }

    /// An ordinary worktree task never reaches the gate, the settings, or a
    /// script — BYOI is a property of the workspace row, not a flag.
    #[test]
    fn non_byoi_tasks_are_untouched() {
        let app = app_with_task("worktree");
        assert!(provision(&app, "t1").is_ok());
        terminate(&app, "t1");
        assert_eq!(recorded_connection(&app), None);
    }

    /// On a build without the feature, a BYOI task refuses to provision and
    /// records nothing — no half-provisioned row to clean up later.
    #[test]
    #[cfg_attr(feature = "remote-tasks", ignore)]
    fn gate_off_refuses_byoi_provision() {
        let app = app_with_task("byoi");
        let error = provision(&app, "t1").unwrap_err().to_string();
        assert!(error.contains("remote-tasks"), "{error}");
        assert_eq!(recorded_connection(&app), None);
    }

    /// Teardown does NOT consult the gate — a disabled build must still be
    /// able to clean up what an enabled one created — but an unprovisioned
    /// workspace has nothing to terminate.
    #[test]
    fn terminate_is_a_no_op_before_provisioning() {
        let app = app_with_task("byoi");
        terminate(&app, "t1");
        assert_eq!(recorded_connection(&app), None);
    }
}

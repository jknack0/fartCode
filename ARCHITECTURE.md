# Architecture — ade

**Audience:** AI coding agents implementing Phase 0 tickets.
**Companion to:** `PRD.md` (product spec), `tickets-phase0.md` (work breakdown).

Every decision here is binding. Tickets assume this document exists; if a ticket
contradicts this file, this file wins (update the ticket).

---

## 1. Crate dependency graph

```
                    ┌──────────────────┐
                    │    ade-app     │  Tauri shell, command modules, events
                    └────────┬─────────┘
                             │
        ┌────────────────────┼────────────────────┐
        │                    │                    │
        ▼                    ▼                    ▼
┌───────────────┐   ┌───────────────┐   ┌───────────────┐
│  ade-core  │   │ade-terminal│   │  ade-git    │
│  (all domain) │   │  (PTY, tmux)  │   │ (worktrees,    │
│               │   │               │   │  git ops, PR)  │
└───────┬───────┘   └───────────────┘   └───────────────┘
        │
        │ depends on (Phase 0 subset shown):
        ▼
┌───────────────┐   ┌───────────────┐
│ade-providers│   │ ade-telemetry│
│ (35 agents,    │   │ (stub in Ph0)   │
│  capabilities) │   │                 │
└───────────────┘   └─────────────────┘
```

**Phase 0 crates that exist but are mostly stubs:** `ade-acp`, `ade-ssh`,
`ade-scheduler`, `ade-integrations`, `ade-server`, `ade-runtime`.
They compile, export placeholder types/traits, and are filled in later phases.

**Rule:** a crate may only depend on crates to its left or below in this graph.
No circular dependencies. `ade-core` is the leaf — it depends on nothing
except third-party crates.

---

## 2. Module layout: `ade-core`

`ade-core` is the largest crate. It follows the reference's domain-per-directory
pattern (`src/main/core/<domain>/`). Every domain gets its own module:

```
ade-core/src/
├── lib.rs                  // re-exports, prelude
├── error.rs                // the one Error enum (§3)
├── db/
│   ├── mod.rs
│   ├── connection.rs       // rusqlite singleton, PRAGMAs, path resolution
│   ├── migrations.rs       // numbered SQL runner, sha256 journal
│   ├── versioned_json.rs   // versioned JSON column helper
│   └── schema.rs           // CREATE TABLE statements (mirrors drizzle/0000_*.sql)
├── settings/
│   ├── mod.rs
│   ├── registry.rs         // typed setting keys + defaults
│   ├── service.rs          // get/set with layered precedence
│   └── kv.rs               // raw kv table access
├── projects/
│   ├── mod.rs
│   ├── model.rs            // Project struct, CRUD
│   ├── provider.rs         // openProject/closeProject lifecycle
│   └── settings.rs         // project_settings table access
├── tasks/
│   ├── mod.rs
│   ├── model.rs            // Task struct, status enum, CRUD
│   ├── naming.rs           // human-id style name+branch generation
│   └── lifecycle.rs        // status transitions, telemetry hooks
├── conversations/
│   ├── mod.rs
│   ├── model.rs            // Conversation struct, CRUD
│   └── supervisor.rs       // LocalExecutionContext, session ids
├── workspaces/
│   ├── mod.rs
│   └── model.rs            // Workspace struct
├── pty/
│   ├── mod.rs
│   └── env_allowlist.rs    // THE canonical env allowlist (§9)
├── view_state/
│   └── mod.rs              // view-state KV persistence
├── search/
│   └── mod.rs              // FTS index writes + queries
└── resource_monitor/
    └── mod.rs              // CPU/memory sampling (sysinfo)
```

**Naming rules:**
- `model.rs` in every domain exports the domain's struct + CRUD functions: `create`, `get`, `list`, `update`, `delete`.
- `mod.rs` re-exports the public API of the domain.
- Struct names are singular: `Project`, `Task`, `Conversation`, `Workspace`, `Terminal`.
- Row types (for DB serialization) are suffixed `Row`: `TaskRow`, `ProjectRow`. These are private to the domain module.
- DB column names use `snake_case` matching the SQL schema. Struct fields use `snake_case` matching the DB columns exactly — no `#[serde(rename)]` gymnastics.

---

## 3. Error type — one to rule them all

`ade-core/src/error.rs` defines the single error enum used by every crate:

```rust
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    // -- Database --
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("migration failed: {0}")]
    Migration(String),

    #[error("versioned JSON parse failed for column {column}: {reason}")]
    VersionedJson { column: String, reason: String },

    // -- Settings --
    #[error("invalid setting key: {0}")]
    InvalidSettingKey(String),

    #[error("invalid setting value for {key}: {reason}")]
    InvalidSettingValue { key: String, reason: String },

    // -- Projects --
    #[error("project not found: {0}")]
    ProjectNotFound(String),

    #[error("project path already registered: {0}")]
    DuplicateProjectPath(PathBuf),

    #[error("project path does not exist: {0}")]
    ProjectPathNotFound(PathBuf),

    // -- Tasks --
    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("invalid task status transition: {from} -> {to}")]
    InvalidStatusTransition { from: String, to: String },

    // -- Worktrees/Git --
    #[error("git error: {0}")]
    Git(String),

    #[error("worktree path already exists: {0}")]
    WorktreeExists(PathBuf),

    #[error("cannot remove project root workspace")]
    CannotRemoveProjectRoot,

    // -- PTY --
    #[error("PTY error: {0}")]
    Pty(String),

    #[error("agent executable not found: {0}")]
    AgentNotFound(String),

    #[error("agent exited with non-zero status: {exit_code}")]
    AgentExited { exit_code: i32 },

    // -- Conversations --
    #[error("conversation not found: {0}")]
    ConversationNotFound(String),

    #[error("empty session id")]
    EmptySessionId,

    // -- I/O --
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    // -- Catch-all --
    #[error("{0}")]
    Internal(String),
}

// Tauri commands return Result<T, String>, so we need this conversion:
impl From<Error> for String {
    fn from(e: Error) -> String {
        e.to_string()
    }
}
```

**Rules for every crate:**
- Every public fallible function returns `Result<T, ade_core::Error>`.
- If a domain needs a new variant, add it to the central `Error` enum — do not create per-domain error types.
- `Internal(String)` is the escape hatch for one-off messages during prototyping. Refactor into a named variant before merging.
- `#[from]` derives on `rusqlite::Error` and `std::io::Error` mean you can use `?` directly in functions that touch the DB or filesystem.

---

## 4. Async boundary

### The rule

```
Tauri commands: sync fn
    │
    └─ if they need async work, call:
       tokio::runtime::Handle::current().block_on(async { ... })
```

The Tauri command handler runs on the main thread. `#[tauri::command]` functions
must be synchronous. Internal async work (DB queries that might block, git ops,
PTY I/O) goes through `block_on`.

### Background tasks

Long-lived tasks spawn on the tokio runtime:

```rust
// In app startup (ade-app/src/main.rs or ade-core init):
let handle = tokio::runtime::Handle::current();

// PTY reader: pipes output to Tauri events
handle.spawn(async move {
    while let Some(data) = pty_reader.read().await {
        app_handle.emit("pty:output", data)?;
    }
});

// File watcher: notify → internal events
handle.spawn(async move {
    while let Some(event) = watcher.recv().await {
        event_bus.send(InternalEvent::FileChanged { ... });
    }
});
```

### Which functions are async?

| Operation | Sync/Async | Why |
|---|---|---|
| DB reads/writes | Sync (rusqlite is sync) | WAL mode, short-lived locks |
| Git CLI calls | Sync with `block_on` | External process, few seconds |
| PTY read loop | Async (`tokio::spawn`) | Long-lived, streaming output |
| File watching | Async (`tokio::spawn`) | Long-lived, event stream |
| HTTP (GitHub API, later) | Async (`reqwest`) | Network I/O — block_on in Ph0 |
| Shell commands | Sync with `block_on` | External process |

### Tauri command template

Every Tauri command follows this pattern:

```rust
// In ade-app/src/commands/projects.rs
use ade_core::projects;
use ade_core::Error;

#[tauri::command]
fn add_project(path: String) -> Result<ProjectDto, String> {
    // 1. Call the domain function (may block internally)
    // 2. Convert Error → String via the blanket From impl
    // 3. Return a DTO (never a DB row type)
    let project = projects::create_local(&path).map_err(String::from)?;
    Ok(ProjectDto::from(project))
}
```

**DTOs vs Row types:** Tauri commands return DTOs (Data Transfer Objects) — flat structs
with `Serialize`. Row types are internal, may contain Connection references or raw JSON
that shouldn't leak to the frontend. Every domain module exports a `dto` submodule or
`impl From<Model> for ModelDto`.

---

## 5. Event bus contract

### Architecture

```
Domain services ──→ InternalEvent ──→ EventBus ──→ Tauri emit ──→ Frontend listeners
                     (enum)            (channel)    (typed event name)
```

### Internal event enum (in `ade-core/src/events.rs`)

```rust
/// Events emitted by domain services, consumed by Tauri command layer.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum InternalEvent {
    // Lifecycle
    AppStarted,
    AppClosed { was_crash: bool },

    // Projects
    ProjectAdded { id: String, name: String, path: String },
    ProjectDeleted { id: String },

    // Tasks
    TaskCreated { id: String, project_id: String, name: String },
    TaskProvisioned { id: String, workspace_id: String },
    TaskStatusChanged { id: String, old_status: String, new_status: String },
    TaskArchived { id: String },
    TaskDeleted { id: String },

    // Conversations
    ConversationCreated { id: String, task_id: String, provider: String, title: String },
    ConversationRenamed { id: String, title: String },
    ConversationDeleted { id: String },

    // Agent lifecycle
    AgentRunStarted { conversation_id: String, provider: String },
    AgentRunFinished { conversation_id: String, provider: String, exit_code: i32 },
    AgentSessionExited { conversation_id: String },

    // PTY
    PtyOutput { pty_id: String, data: Vec<u8> },
    PtyClosed { pty_id: String },

    // Terminals
    TerminalCreated { id: String, task_id: String },
    TerminalDeleted { id: String },

    // Git
    GitChanged { project_id: String, workspace_id: String },

    // Settings
    SettingChanged { key: String },

    // UI state
    SidebarToggled { visible: bool },

    // Errors
    Error { context: String, message: String },
}
```

### Tauri event names

Internal events are emitted to the frontend as Tauri events with the same name
(lowercased, `:` replaced with `:`):

| InternalEvent variant | Tauri event name |
|---|---|
| `TaskCreated` | `task:created` |
| `PtyOutput` | `pty:output` |
| `AgentRunFinished` | `agent:run-finished` |

### Event emission

```rust
// In ade-app, wrap the core event bus:
pub struct AppEventBus {
    app_handle: tauri::AppHandle,
}

impl AppEventBus {
    pub fn emit(&self, event: InternalEvent) {
        let name = event_name(&event); // derive from variant name
        let _ = self.app_handle.emit(&name, &event);
    }
}

fn event_name(event: &InternalEvent) -> String {
    // Snake-case the variant name, replace _ with :
    // TaskCreated → "task:created"
    // PtyOutput → "pty:output"
    todo!()
}
```

### Frontend listener pattern

```typescript
// In app-frontend/src/lib/events.ts
import { listen } from '@tauri-apps/api/event';

export function onTaskCreated(cb: (payload: TaskCreatedPayload) => void) {
  return listen<TaskCreatedPayload>('task:created', (event) => cb(event.payload));
}
```

---

## 6. Key traits

These traits define the boundaries between crates. Implementations live in
the owning crate; consumers depend on the trait.

### 6.1 Db (in `ade-core::db`)

```rust
/// Single connection to the SQLite database.
/// Created once at startup, shared via Arc.
pub trait Db: Send + Sync {
    /// Returns a reference to the underlying rusqlite connection.
    /// Callers use this for queries; the connection mutex is internal.
    fn conn(&self) -> &std::sync::Mutex<rusqlite::Connection>;

    /// Direct KV access (app_settings + kv tables are the same shape).
    fn kv_get(&self, key: &str) -> Result<Option<String>, Error>;
    fn kv_set(&self, key: &str, value: &str) -> Result<(), Error>;
    fn kv_delete(&self, key: &str) -> Result<(), Error>;

    /// Path to the database file.
    fn path(&self) -> &std::path::Path;
}

/// Concrete implementation (not a trait object at runtime — just a struct).
pub struct SqliteDb {
    conn: std::sync::Mutex<rusqlite::Connection>,
    path: std::path::PathBuf,
}

impl SqliteDb {
    /// Initializes the database: opens/creates, sets PRAGMAs, runs migrations.
    /// Called exactly once at app startup.
    pub fn init(db_path: Option<&str>) -> Result<Arc<Self>, Error> {
        let path = resolve_db_path(db_path);
        let conn = open_connection(&path)?;
        Self::run_migrations(&conn)?;
        Ok(Arc::new(Self {
            conn: std::sync::Mutex::new(conn),
            path,
        }))
    }
}
```

### 6.2 SettingsStore (in `ade-core::settings`)

> **Implemented (E1-02/E1-03).** The trait below is object-safe — ARCHITECTURE §7
> stores it as `Arc<dyn SettingsStore>`, so `get`/`set` take/return JSON rather
> than the generic `SettingKey<T>` sketch. Typed access (`settings::get(&PROJECT)`,
> `settings::set(&TERMINAL, group)`) lives on the concrete `DbSettingsStore` via
> `SettingKey<T>` wrappers. The project-settings surface was added so
> `ade-core::projects` can seed/read settings through the trait object.

```rust
/// Object-safe settings store. JSON surface (typed wrappers on DbSettingsStore).
pub trait SettingsStore: Send + Sync {
    /// Effective value for an app-setting key (deep-merged with defaults).
    fn get_json(&self, key: &str) -> Result<serde_json::Value, Error>;

    /// Validates and stores `value`, computing the delta vs defaults and
    /// deleting the row when the delta is empty.
    fn set_json(&self, key: &str, value: serde_json::Value) -> Result<(), Error>;

    /// Clear all local overrides. `None` clears `app_settings`; `Some(project_id)`
    /// clears the project's `project_settings` row (`.ade.json` untouched).
    fn reset(&self, project_id: Option<&str>) -> Result<(), Error>;

    /// Move local shareable values into the repo's `.ade.json` and clear the DB.
    fn share_with_team(&self, project_id: &str) -> Result<(), Error>;

    // -- project settings (used by `projects`) --
    fn seed_project_settings(&self, project_id: &str, repo_dir: &Path) -> Result<(), Error>;
    fn get_project_settings(&self, project_id: &str, repo_dir: &Path)
        -> Result<ProjectSettings, Error>;
    fn update_project_settings(&self, project_id: &str, repo_dir: &Path,
        settings: &ProjectSettings) -> Result<(), Error>;
    fn migrate_legacy_project_settings(&self, project_id: &str, repo_dir: &Path)
        -> Result<(), Error>;
}
```

### 6.3 PtyManager (in `ade-terminal`)

```rust
/// Spawns and controls PTY processes.
pub trait PtyManager: Send + Sync {
    /// Spawn a new PTY with the given config.
    /// Returns a handle for I/O and lifecycle control.
    fn spawn(&self, config: PtyConfig) -> Result<PtyHandle, Error>;

    /// Resize an existing PTY.
    fn resize(&self, id: &PtyId, cols: u16, rows: u16) -> Result<(), Error>;

    /// Kill a PTY process and clean up.
    fn kill(&self, id: &PtyId) -> Result<(), Error>;

    /// List active PTY sessions.
    fn list(&self) -> Vec<PtyId>;
}

pub struct PtyConfig {
    /// Unique ID for this PTY session.
    pub id: PtyId,
    /// Working directory (typically the task worktree).
    pub cwd: std::path::PathBuf,
    /// Command to run (e.g., the agent CLI).
    pub command: Vec<String>,
    /// Environment variables to inject.
    pub env: HashMap<String, String>,
    /// Initial terminal size.
    pub cols: u16,
    pub rows: u16,
    /// Whether this is a tmux-backed session.
    pub use_tmux: bool,
}

pub struct PtyId(pub String);

pub struct PtyHandle {
    pub id: PtyId,
    /// Channel receiving PTY output as bytes.
    pub output: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    /// Channel to send input to the PTY.
    pub input: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    /// Wait for the process to exit.
    pub exit: tokio::task::JoinHandle<i32>,
}
```

### 6.4 GitOps (in `ade-git`)

> **Implemented (E1-03).** The **trait lives in `ade-core::git`** — not here — so
> that `ade-core` domain modules (`projects`, later `workspaces`) can use it
> without violating the crate-graph rule that `ade-core` is the leaf (depends on
> nothing internal). `ade-git` provides the implementation (`CliGit`) and
> re-exports the trait. Phase 0 uses the **`git` CLI** (via `Command` arg arrays,
> no shell/quoting) — git2 worktree lifecycle bindings land with E2-02. The trait
> adds E1-03 ops beyond the sketch below (`init`, `clone`, `show_toplevel`,
> `git_dir`, `remotes`, `current_branch`, `remote_head`, `verify_ref`, `branches`);
> `remote_head` is local-only (symbolic-ref) — the reference's `git remote show`
> fallback is a network call and was dropped to avoid hangs.

```rust
/// Low-level git operations. Phase 0 implementation: ade_git::CliGit (git CLI).
pub trait GitOps: Send + Sync {
    /// Run `git worktree list --porcelain` and parse the output.
    fn worktree_list(&self, repo_path: &Path) -> Result<Vec<WorktreeEntry>, Error>;

    /// `git worktree add <path> <branch>`.
    fn worktree_add(&self, repo_path: &Path, target_path: &Path, branch: &str) -> Result<(), Error>;

    /// `git worktree prune`.
    fn worktree_prune(&self, repo_path: &Path) -> Result<(), Error>;

    /// Remove a worktree (rm -rf + git worktree prune).
    fn worktree_remove(&self, repo_path: &Path, worktree_path: &Path) -> Result<(), Error>;

    /// `git branch <name> <start_point>`.
    fn branch_create(&self, repo_path: &Path, name: &str, start_point: &str) -> Result<(), Error>;

    /// `git rev-parse --abbrev-ref HEAD` or resolve default branch.
    fn default_branch(&self, repo_path: &Path) -> Result<String, Error>;

    /// `git fetch <remote>`.
    fn fetch(&self, repo_path: &Path, remote: &str) -> Result<(), Error>;

    /// Check if a path is inside a git worktree.
    fn is_worktree(&self, path: &Path) -> Result<bool, Error>;
}

/// Parsed from `git worktree list --porcelain`.
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub head: String,
    pub branch: Option<String>, // "refs/heads/xxx"
    pub bare: bool,
    pub locked: bool,
}
```

### 6.5 AgentRegistry (in `ade-providers`)

```rust
/// Registry of all known agent CLIs.
pub trait AgentRegistry: Send + Sync {
    fn get(&self, provider: ProviderId) -> Option<&ProviderDef>;
    fn list(&self) -> &[ProviderDef];
    fn filter_by_capability(&self, cap: Capability) -> Vec<&ProviderDef>;
    fn resolve_executable(&self, provider: ProviderId) -> Result<PathBuf, Error>;
}

/// One registered agent provider.
pub struct ProviderDef {
    pub id: ProviderId,
    pub name: String,
    pub capabilities: CapabilityFlags,
    pub behavior: ProviderBehavior,
    pub models: Vec<String>, // empty = "Default model" only
}

/// What the provider supports.
#[derive(Default)]
pub struct CapabilityFlags {
    pub acp: bool,
    pub auth: bool,
    pub auto_approve: bool,
    pub effort: bool,
    pub hooks: bool,
    pub host_dependency: bool,
    pub mcp: bool,
    pub models: bool,
    pub plugins: bool,
    pub prompt: bool,
    pub sessions: bool,
    pub trust: bool,
}

/// How to launch this provider.
pub struct ProviderBehavior {
    pub prompt: PromptStrategy,
    /// CLI command + base args. {cli}, {prompt}, {session_id} etc. are template vars.
    pub command_template: String,
    pub resume_flag: Option<String>,       // e.g. "--resume"
    pub auto_approve_flag: Option<String>, // e.g. "--dangerously-skip-permissions"
}

pub enum PromptStrategy {
    /// -p or --prompt flag
    Argv { flag: String },
    /// Deliver prompt on stdin after spawn
    Stdin,
    /// Type the prompt into the TUI after startup
    KeystrokeInjection {
        /// Text that indicates the agent is ready (e.g., "How can I help?")
        startup_indicator: String,
        /// Delay between characters in ms
        delay_ms: u64,
    },
}
```

### 6.6 EventBus (in `ade-core::events`)

```rust
/// Internal event bus. Domain services push events; the Tauri layer subscribes
/// and forwards them to the frontend.
pub trait EventBus: Send + Sync {
    fn send(&self, event: InternalEvent);
}

/// Concrete implementation using tokio::sync::broadcast.
pub struct BroadcastEventBus {
    tx: tokio::sync::broadcast::Sender<InternalEvent>,
}

impl BroadcastEventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<InternalEvent> {
        self.tx.subscribe()
    }
}

impl EventBus for BroadcastEventBus {
    fn send(&self, event: InternalEvent) {
        let _ = self.tx.send(event); // ignore if no receivers
    }
}
```

---

## 7. Application bootstrap (the `App` struct)

Every domain service is created in `ade-app` at startup and shared via `Arc`.
This is the single place where concrete implementations are wired together:

```rust
// ade-app/src/app.rs
use std::sync::Arc;

pub struct App {
    pub db: Arc<ade_core::db::SqliteDb>,
    pub settings: Arc<dyn ade_core::settings::SettingsStore>,
    pub projects: Arc<dyn ade_core::projects::ProjectStore>,
    pub tasks: Arc<dyn ade_core::tasks::TaskStore>,
    pub conversations: Arc<dyn ade_core::conversations::ConversationStore>,
    pub agent_registry: Arc<dyn ade_providers::AgentRegistry>,
    pub pty_manager: Arc<dyn ade_terminal::PtyManager>,
    pub git: Arc<dyn ade_git::GitOps>,
    pub event_bus: Arc<dyn ade_core::events::EventBus>,
}

impl App {
    pub fn init(db_path: Option<&str>) -> Result<Arc<Self>, Error> {
        let db = ade_core::db::SqliteDb::init(db_path)?;
        let event_bus = Arc::new(ade_core::events::BroadcastEventBus::new(256));
        let settings = Arc::new(ade_core::settings::DbSettingsStore::new(db.clone()));
        let agent_registry = Arc::new(ade_providers::StaticRegistry::default());
        let git = Arc::new(ade_git::CliGit::new());
        let pty_manager = Arc::new(ade_terminal::PortablePtyManager::new());

        let projects = Arc::new(ade_core::projects::DbProjectStore::new(
            db.clone(), settings.clone(), git.clone(), event_bus.clone(),
        ));
        let tasks = Arc::new(ade_core::tasks::DbTaskStore::new(
            db.clone(), event_bus.clone(),
        ));
        let conversations = Arc::new(ade_core::conversations::DbConversationStore::new(
            db.clone(), event_bus.clone(),
        ));

        Ok(Arc::new(Self {
            db, settings, projects, tasks, conversations,
            agent_registry, pty_manager, git, event_bus,
        }))
    }
}
```

Tauri's `setup` hook initializes the app and manages it as Tauri state:

```rust
// ade-app/src/main.rs
fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let app_state = App::init(std::env::var("ADE_DB_FILE").ok().as_deref())?;

            // Forward internal events to the frontend
            let app_handle = app.handle().clone();
            let mut rx = app_state.event_bus.subscribe();
            tokio::spawn(async move {
                while let Ok(event) = rx.recv().await {
                    let _ = app_handle.emit(&event_name(&event), &event);
                }
            });

            // Register app state for Tauri commands
            app.manage(app_state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_project,
            commands::create_task,
            // ... etc
        ])
        .run(tauri::generate_context!())
        .expect("failed to start app");
}
```

**Tauri command access pattern:**

```rust
#[tauri::command]
fn add_project(state: tauri::State<'_, Arc<App>>, path: String) -> Result<ProjectDto, String> {
    state.projects.create_local(&path).map(ProjectDto::from).map_err(String::from)
}
```

---

## 8. Code sketches — critical algorithms

### 8.1 Migration runner

```rust
// ade-core/src/db/migrations.rs

/// Embedded migration: (number, label, SQL text).
/// The label is used for the migration journal, SQL is the raw DDL.
struct Migration {
    number: u32,
    label: &'static str,
    sql: &'static str,
}

// Migrations are embedded at compile time via include_str! or a build script.
const MIGRATIONS: &[Migration] = &[
    Migration { number: 0, label: "initial_schema", sql: include_str!("../../migrations/0000_initial.sql") },
    // ... more migrations added as the schema evolves
];

/// Called once at startup. Idempotent.
pub fn run_migrations(conn: &rusqlite::Connection) -> Result<(), Error> {
    // 1. Create the migrations table if missing
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS migrations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            label TEXT NOT NULL,
            hash TEXT NOT NULL,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP
        );"
    )?;

    // 2. Find the highest already-applied migration number
    let max_applied: Option<u32> = conn
        .query_row("SELECT MAX(id) FROM migrations", [], |row| row.get(0))
        .unwrap_or(None);

    // 3. Apply each pending migration, in order
    for mig in MIGRATIONS {
        if mig.number <= max_applied.unwrap_or(0) {
            continue; // already applied
        }

        // 4. Hash the SQL for the journal
        let hash = sha256_hash(mig.sql);

        // 5. Split on --> statement-breakpoint (Drizzle convention)
        for stmt in mig.sql.split("--> statement-breakpoint") {
            let stmt = stmt.trim();
            if stmt.is_empty() { continue; }
            conn.execute(stmt, [])?;
        }

        // 6. Record in journal
        conn.execute(
            "INSERT INTO migrations (id, label, hash) VALUES (?1, ?2, ?3)",
            rusqlite::params![mig.number, mig.label, hash],
        )?;
    }

    Ok(())
}
```

### 8.2 PTY agent launch flow

```rust
// ade-terminal/src/manager.rs

/// The complete agent launch sequence.
/// Called by the conversation supervisor (E2-06).
pub fn launch_agent(
    pty: &dyn PtyManager,
    registry: &dyn AgentRegistry,
    config: AgentLaunchConfig,
) -> Result<PtyHandle, Error> {
    // 1. Look up the provider
    let provider = registry.get(config.provider_id)
        .ok_or(Error::AgentNotFound(config.provider_id.0.clone()))?;

    // 2. Resolve the executable
    let executable = registry.resolve_executable(config.provider_id)?;

    // 3. Build the command from the provider's template
    let cmd = build_command(provider, &config, &executable)?;

    // 4. Build the env allowlist
    let env = ade_core::pty::env_allowlist::build_agent_env(
        &config.provider_id,
        &config.task_env,
        None, // hook_env — Phase 0: None
    );

    // 5. Spill large prompt to temp file if needed
    let (final_cmd, _temp_file) = spill_large_prompt(cmd, config.initial_prompt)?;

    // 6. Spawn the PTY
    let handle = pty.spawn(PtyConfig {
        id: PtyId(format!("conv:{}", config.conversation_id)),
        cwd: config.worktree_path,
        command: final_cmd,
        env,
        cols: 80,
        rows: 24,
        use_tmux: config.use_tmux,
    })?;

    // 7. If keystroke injection: wait for startup indicator, then type
    if matches!(provider.behavior.prompt, PromptStrategy::KeystrokeInjection { .. }) {
        inject_prompt(&handle, config.initial_prompt, provider)?;
    }

    Ok(handle)
}

fn build_command(
    provider: &ProviderDef,
    config: &AgentLaunchConfig,
    executable: &Path,
) -> Result<Vec<String>, Error> {
    let mut cmd = vec![executable.to_string_lossy().to_string()];
    let t = &provider.behavior;

    // Template substitution:
    // {prompt} → config.initial_prompt (or @/tmp/... if spilled)
    // {session_id} → config.session_id
    // {model} → config.model

    let command_str = t.command_template
        .replace("{prompt}", &shell_quote(&config.initial_prompt))
        .replace("{session_id}", &config.session_id)
        .replace("{model}", &config.model);

    for arg in command_str.split_whitespace() {
        cmd.push(arg.to_string());
    }

    if config.is_resuming {
        if let Some(ref flag) = t.resume_flag {
            cmd.push(flag.clone());
        }
    }

    if config.auto_approve {
        if let Some(ref flag) = t.auto_approve_flag {
            cmd.push(flag.clone());
        }
    }

    Ok(cmd)
}
```

### 8.3 Worktree create flow

```rust
// ade-core/src/projects/worktrees.rs

/// Create a new worktree from a source branch.
pub fn checkout_branch_worktree(
    git: &dyn GitOps,
    project: &Project,
    branch_name: &str,
    source_ref: &str,     // e.g., "refs/heads/main" or "origin/main"
    preserve_patterns: &[String],
) -> Result<WorktreeEntry, Error> {
    // 1. Fetch the source remote if needed
    if let Some(remote) = extract_remote(source_ref) {
        git.fetch(&project.path, remote)?;
    }

    // 2. Create the branch from the source ref
    git.branch_create(&project.path, branch_name, source_ref)?;

    // 3. Determine worktree path
    let pool_path = project.worktree_directory();
    let worktree_path = pool_path.join(branch_name);

    // 4. Add the worktree
    git.worktree_add(&project.path, &worktree_path, branch_name)?;

    // 5. Copy preserved files (.env, etc.)
    copy_preserved_files(&project.path, &worktree_path, preserve_patterns)?;

    // 6. Push the branch (non-fatal)
    let _ = git.push(&project.path, branch_name);

    // 7. Return the worktree entry
    let entries = git.worktree_list(&project.path)?;
    entries.into_iter()
        .find(|e| e.path == worktree_path)
        .ok_or(Error::Git("worktree not found after creation".into()))
}
```

### 8.4 Settings precedence resolution

```rust
// ade-core/src/settings/service.rs

/// Effective value = local override > .ade.json > default.
pub fn get_effective<T: SettingValue>(
    db: &dyn Db,
    project_id: &str,
    key: SettingKey<T>,
) -> Result<T, Error> {
    // 1. Try local override (base_project_settings_json in project_settings)
    if let Some(value) = get_local_override(db, project_id, &key)? {
        return Ok(value);
    }

    // 2. Try .ade.json (shareable_project_settings_json in project_settings)
    if let Some(value) = get_shareable_value(db, project_id, &key)? {
        return Ok(value);
    }

    // 3. Fall back to the registered default
    Ok(key.default_value().clone())
}

/// Set a local override. If the new value equals the default, delete the override.
pub fn set_local<T: SettingValue>(
    db: &dyn Db,
    project_id: &str,
    key: SettingKey<T>,
    value: T,
) -> Result<(), Error> {
    if value == *key.default_value() {
        // Remove the override — fall back to .ade.json or default
        delete_local_override(db, project_id, &key)?;
    } else {
        // Store the override
        upsert_local_override(db, project_id, &key, &value)?;
    }

    // Emit event
    event_bus.send(InternalEvent::SettingChanged { key: key.name().into() });
    Ok(())
}
```

### 8.5 Versioned JSON helper

```rust
// ade-core/src/db/versioned_json.rs

/// A column value that carries a version number and upgrade chain.
/// Stored as JSON: {"version": N, "data": ...}
///
/// On read: detect version → run upgrade chain → parse as T.
/// On write: always serialize as latest version.
/// Corrupt/unparseable values return None (never panic).
pub fn read_versioned<T: serde::de::DeserializeOwned>(
    raw: Option<&str>,
    current_version: u32,
    upgrade_chain: &[fn(serde_json::Value) -> serde_json::Value],
) -> Option<T> {
    let raw = raw?;
    let wrapper: VersionedWrapper = serde_json::from_str(raw).ok()?;

    let mut data = wrapper.data;

    // Run upgrades sequentially
    for version in wrapper.version..current_version {
        if let Some(upgrade_fn) = upgrade_chain.get(version as usize) {
            data = upgrade_fn(data);
        }
    }

    serde_json::from_value(data).ok()
}

pub fn write_versioned<T: serde::Serialize>(
    value: &T,
    current_version: u32,
) -> String {
    let wrapper = VersionedWrapper {
        version: current_version,
        data: serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
    };
    serde_json::to_string(&wrapper).unwrap_or_default()
}

#[derive(serde::Serialize, serde::Deserialize)]
struct VersionedWrapper {
    version: u32,
    data: serde_json::Value,
}
```

---

## 9. PTY env allowlist

One file. One function. Security-reviewed. Any addition = PR + review.

```rust
// ade-core/src/pty/env_allowlist.rs

/// Returns the final environment map for an agent process.
/// Starts with base env, merges only allowlisted vars from the host,
/// overlays task env vars, overlays hook env (when present).
pub fn build_agent_env(
    provider_id: &ProviderId,
    task_env: &HashMap<String, String>,
    hook_env: Option<&HashMap<String, String>>,
) -> HashMap<String, String> {
    let mut env = HashMap::new();

    // -- Base env (always set) --
    env.insert("TERM".into(), "xterm-256color".into());
    env.insert("COLORTERM".into(), "truecolor".into());
    env.insert("TERM_PROGRAM".into(), "ade".into());

    // -- Inherit allowlisted vars from the host process --
    let allowlist: &[&str] = &[
        "HOME", "USER", "PATH", "TMPDIR",
        "EDITOR", "VISUAL", "GIT_EDITOR", "HOSTNAME", "LANG", "TZ",
        // Provider keys
        "ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL",
        "CLAUDE_CONFIG_DIR",
        "OPENAI_API_KEY", "OPENAI_ORG_ID",
        "OPENROUTER_API_KEY",
        "GEMINI_API_KEY", "GOOGLE_API_KEY",
        "GITHUB_TOKEN", "GH_TOKEN", "GITLAB_TOKEN",
        "AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_REGION",
        "AZURE_OPENAI_API_KEY", "AZURE_OPENAI_ENDPOINT",
        "MISTRAL_API_KEY", "XAI_API_KEY", "DEEPSEEK_API_KEY",
        "GROQ_API_KEY", "TOGETHER_API_KEY",
        "CURSOR_API_KEY", "CODEX_HOME",
        "COPILOT_CLI_TOKEN",
        "GOOSE_API_KEY",
        "QWEN_API_KEY",
        "PI_API_KEY",
        // Proxy
        "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY",
        "http_proxy", "https_proxy", "no_proxy",
        // SSH
        "SSH_AUTH_SOCK",
    ];

    for key in allowlist {
        if let Ok(val) = std::env::var(key) {
            env.insert(key.to_string(), val);
        }
    }

    // Inject SSH_AUTH_SOCK if missing but available
    if !env.contains_key("SSH_AUTH_SOCK") {
        if let Some(sock) = detect_ssh_auth_sock() {
            env.insert("SSH_AUTH_SOCK".into(), sock);
        }
    }

    // Overlay task env (ADE_TASK_ID, etc.)
    for (k, v) in task_env {
        env.insert(k.clone(), v.clone());
    }

    // Overlay hook env (when E3-05 is running)
    if let Some(hook) = hook_env {
        for (k, v) in hook {
            env.insert(k.clone(), v.clone());
        }
    }

    env
}
```

---

## 10. Patterns to follow (checklist)

Every PR must satisfy these. If a ticket asks for something that violates a pattern,
the pattern wins — update the ticket.

1. **`Result<T, ade_core::Error>` everywhere.** No `unwrap()`, no `expect()`, no panics across crate boundaries. Use `?` pervasively. The only `unwrap()` allowed is in `main()` and tests.

2. **Versioned JSON for all JSON columns.** No raw `JSON.parse`/`JSON.stringify` at call sites. Use the `read_versioned`/`write_versioned` helpers. Corrupt data returns `None`, never panics.

3. **Arc<dyn Trait> for all services.** Every domain service is trait-based and shared via `Arc`. No global state, no lazy_static, no OnceCell for services.

4. **Tauri commands are thin wrappers.** They call a domain function, map the error, convert to DTO, return. No business logic in command handlers.

5. **DTOs at the boundary.** DB row types never cross the Tauri IPC boundary. Every domain exports `ModelDto` with `Serialize`.

6. **Shell quoting via the shared module.** No ad-hoc `format!("'{}'", path)` or manual escaping. Call `ade_core::shell_escape::quote(input)`.

7. **Path validation via realpath containment.** Before deleting or operating in a directory, verify it's inside the expected root. Never delete project root.

8. **Telemetry stubs.** Every domain emits `InternalEvent` variants. In Phase 0, the telemetry crate is a stub that logs events to `tracing::debug!`. The event pipeline is wired; actual ingestion comes in Phase 2 (E15).

9. **Tests use temp directories.** Git integration tests create temp repos in `std::env::temp_dir()`. DB tests use `rusqlite::Connection::open_in_memory()` or temp files. Never touch `~/Library/Application Support` in tests.

10. **Migrations are append-only.** Never edit an already-applied migration SQL. Add a new numbered migration instead. The migration journal hashes prevent tampering.

---

## 11. Database schema (Phase 0 subset)

The initial migration (`migrations/0000_initial.sql`) creates these tables.
Columns match the reference's `0000_cuddly_scarecrow.sql` + later additions
pulled forward for Phase 0 convenience. See the reference `drizzle/*.sql` files
for the full evolution — our initial migration is a consolidated version.

```sql
-- Settings
CREATE TABLE app_settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL,
    updated_at INTEGER DEFAULT (unixepoch()) NOT NULL
);

CREATE TABLE kv (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL,
    updated_at INTEGER DEFAULT (unixepoch()) NOT NULL
);

-- Projects
CREATE TABLE projects (
    id                 TEXT PRIMARY KEY NOT NULL,
    name               TEXT NOT NULL,
    path               TEXT NOT NULL,
    workspace_provider TEXT DEFAULT 'local' NOT NULL,
    base_ref           TEXT,
    ssh_connection_id  TEXT,
    repository_workspace_id TEXT,
    created_at         TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at         TEXT DEFAULT (datetime('now')) NOT NULL
);
CREATE UNIQUE INDEX idx_projects_path ON projects(path);

CREATE TABLE project_remotes (
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    remote_name TEXT NOT NULL,
    remote_url  TEXT NOT NULL,
    PRIMARY KEY (project_id, remote_name)
);

CREATE TABLE project_settings (
    project_id                   TEXT PRIMARY KEY NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    base_project_settings_json   TEXT NOT NULL DEFAULT '{}',
    shareable_project_settings_json TEXT NOT NULL DEFAULT '{}',
    legacy_config_migrated_at    TEXT,
    created_at                   TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at                   TEXT DEFAULT (datetime('now')) NOT NULL
);

-- Tasks
CREATE TABLE tasks (
    id                  TEXT PRIMARY KEY NOT NULL,
    project_id          TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    status              TEXT NOT NULL,
    linked_issue        TEXT,
    archived_at         TEXT,
    created_at          TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at          TEXT DEFAULT (datetime('now')) NOT NULL,
    last_interacted_at  TEXT,
    status_changed_at   TEXT DEFAULT (datetime('now')) NOT NULL,
    is_pinned           INTEGER DEFAULT 0 NOT NULL,
    created_by          TEXT DEFAULT 'user' NOT NULL,  -- 'user' | 'agent:<conversation_id>'
    workspace_id        TEXT,
    workspace_intent    TEXT,
    type                TEXT DEFAULT 'task' NOT NULL,
    automation_run_id   TEXT
);
CREATE INDEX idx_tasks_project_id ON tasks(project_id);

-- Workspaces
CREATE TABLE workspaces (
    id                TEXT PRIMARY KEY NOT NULL,
    key               TEXT,
    type              TEXT,  -- local | project-ssh | byoi
    kind              TEXT,  -- worktree | project-root | byoi
    location          TEXT,  -- local | remote
    ssh_connection_id TEXT,
    data              TEXT,  -- versioned JSON
    path              TEXT,
    config            TEXT,  -- versioned JSON
    branch_name       TEXT,
    lines_added       INTEGER,
    lines_deleted     INTEGER,
    created_at        TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at        TEXT DEFAULT (datetime('now')) NOT NULL
);
CREATE UNIQUE INDEX idx_workspaces_key ON workspaces(key) WHERE key IS NOT NULL;

-- Conversations
CREATE TABLE conversations (
    id                       TEXT PRIMARY KEY NOT NULL,
    project_id               TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    task_id                  TEXT REFERENCES tasks(id) ON DELETE CASCADE,  -- NULL for project-scoped
    scope                    TEXT DEFAULT 'task' NOT NULL,  -- 'task' | 'project'
    title                    TEXT NOT NULL,
    provider                 TEXT,
    config                   TEXT,  -- versioned JSON
    created_at               TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at               TEXT DEFAULT (datetime('now')) NOT NULL,
    last_interacted_at       TEXT,
    is_initial_conversation  INTEGER,
    session_id               TEXT,
    agent_status             TEXT,
    agent_status_seen        INTEGER DEFAULT 1,
    type                     TEXT
);
CREATE INDEX idx_conversations_task_id ON conversations(task_id);

-- Terminals
CREATE TABLE terminals (
    id         TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    task_id    TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    ssh        INTEGER DEFAULT 0 NOT NULL,
    name       TEXT NOT NULL,
    shell_id   TEXT NOT NULL DEFAULT 'system',
    created_at TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at TEXT DEFAULT (datetime('now')) NOT NULL
);
CREATE INDEX idx_terminals_task_id ON terminals(task_id);

-- Messages (legacy — read-only in Phase 0, useful for migration path)
CREATE TABLE messages (
    id              TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    content         TEXT NOT NULL,
    sender          TEXT NOT NULL,
    timestamp       TEXT DEFAULT (datetime('now')) NOT NULL,
    metadata        TEXT
);
CREATE INDEX idx_messages_conversation_id ON messages(conversation_id);

-- Editor buffers
CREATE TABLE editor_buffers (
    id           TEXT PRIMARY KEY NOT NULL,  -- {project_id}:{workspace_id}:{file_path}
    project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    file_path    TEXT NOT NULL,
    content      TEXT NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE INDEX idx_editor_buffers_workspace_file ON editor_buffers(workspace_id, file_path);

-- Automations (Phase 2, table exists for FK references)
CREATE TABLE automations (
    id                  TEXT PRIMARY KEY NOT NULL,
    name                TEXT NOT NULL,
    project_id          TEXT REFERENCES projects(id) ON DELETE SET NULL,
    trigger_config      TEXT,  -- versioned JSON
    conversation_config TEXT,  -- versioned JSON
    task_config         TEXT,  -- versioned JSON
    enabled             INTEGER DEFAULT 1 NOT NULL,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    deleted_at          INTEGER
);

CREATE TABLE automation_runs (
    id                            TEXT PRIMARY KEY NOT NULL,
    automation_id                 TEXT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
    scheduled_at                  INTEGER,
    deadline_at                   INTEGER,
    started_at                    INTEGER,
    task_created_at               INTEGER,
    launched_at                   INTEGER,
    finished_at                   INTEGER,
    status                        TEXT NOT NULL,
    error                         TEXT,
    trigger_kind                  TEXT NOT NULL,
    trigger_config_snapshot       TEXT NOT NULL DEFAULT '{}',
    conversation_config_snapshot  TEXT NOT NULL DEFAULT '{}',
    task_config_snapshot          TEXT,
    generated_task_name           TEXT
);

-- SSH connections (Phase 3, table exists for FK references)
CREATE TABLE ssh_connections (
    id               TEXT PRIMARY KEY NOT NULL,
    name             TEXT NOT NULL,
    host             TEXT NOT NULL,
    port             INTEGER DEFAULT 22 NOT NULL,
    username         TEXT NOT NULL,
    auth_type        TEXT DEFAULT 'agent' NOT NULL,
    private_key_path TEXT,
    use_agent        INTEGER DEFAULT 0 NOT NULL,
    metadata         TEXT,  -- versioned JSON
    created_at       TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at       TEXT DEFAULT (datetime('now')) NOT NULL
);

-- Provider accounts (Phase 2, table exists for FK references)
CREATE TABLE provider_accounts (
    id             TEXT PRIMARY KEY NOT NULL,
    provider_id    TEXT NOT NULL,
    account_id     TEXT NOT NULL,
    credential_ref TEXT NOT NULL,
    is_default     INTEGER DEFAULT 0 NOT NULL,
    meta           TEXT,  -- versioned JSON
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);

-- FTS tables (created outside migrations, version-gated via kv)
CREATE VIRTUAL TABLE IF NOT EXISTS search_index USING fts5(
    item_type,    -- project|task|conversation|command
    item_id UNINDEXED,
    project_id UNINDEXED,
    task_id UNINDEXED,
    title,
    keywords,
    tokenize='trigram'
);

CREATE VIRTUAL TABLE IF NOT EXISTS workspace_file_index USING fts5(
    workspace_id UNINDEXED,
    file_path,
    tokenize='trigram'
);
CREATE TABLE IF NOT EXISTS workspace_file_index_meta (
    workspace_id TEXT PRIMARY KEY,
    indexed_at INTEGER NOT NULL
);
```

---

## 12. Frontend architecture (React + Vite)

### Directory layout

```
app-frontend/
├── src/
│   ├── main.tsx                  // React root, app mount
│   ├── App.tsx                   // Shell: sidebar, tab bar, view area
│   ├── lib/
│   │   ├── ipc.ts                // Typed Tauri invoke + event wrappers
│   │   ├── events.ts             // Per-event listener hooks (useTaskCreated, etc.)
│   │   └── commands.ts           // Typed wrappers around tauri::invoke
│   ├── features/
│   │   ├── sidebar/              // Project tree, pinned tasks
│   │   ├── tasks/                // Task creation dialog, task tabs
│   │   ├── conversations/        // Conversation list, message input
│   │   ├── terminal/             // xterm.js wrapper component
│   │   ├── settings/             // Project settings panels
│   │   ├── command-palette/      // ⌘K overlay
│   │   └── onboarding/           // First-run flow
│   ├── components/               // Shared UI: Button, Modal, Input, etc.
│   └── styles/
│       └── globals.css           // Tailwind base
├── index.html
├── package.json
├── tsconfig.json
├── vite.config.ts
└── tailwind.config.js
```

### IPC patterns

```typescript
// lib/commands.ts — typed wrappers around Tauri invoke
import { invoke } from '@tauri-apps/api/core';

export interface ProjectDto {
  id: string;
  name: string;
  path: string;
}

export async function addProject(path: string): Promise<ProjectDto> {
  return invoke<ProjectDto>('add_project', { path });
}

export async function createTask(config: CreateTaskConfig): Promise<TaskDto> {
  return invoke<TaskDto>('create_task', { config });
}
```

```typescript
// lib/events.ts — typed event listeners
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface TaskCreatedPayload {
  id: string;
  project_id: string;
  name: string;
}

export function onTaskCreated(cb: (p: TaskCreatedPayload) => void): UnlistenFn {
  const unlisten = listen<TaskCreatedPayload>('task:created', (e) => cb(e.payload));
  // Return a cleanup function
  return () => { unlisten.then(fn => fn()); };
}
```

### State management

Use **Zustand** (lightweight, no boilerplate, React hooks-native). No MobX, no Redux.

```typescript
// features/tasks/taskStore.ts
import { create } from 'zustand';

interface TaskState {
  tasks: TaskDto[];
  addTask: (task: TaskDto) => void;
  updateStatus: (id: string, status: string) => void;
}

export const useTaskStore = create<TaskState>((set) => ({
  tasks: [],
  addTask: (task) => set((s) => ({ tasks: [...s.tasks, task] })),
  updateStatus: (id, status) =>
    set((s) => ({
      tasks: s.tasks.map((t) => (t.id === id ? { ...t, status } : t)),
    })),
}));
```

Store updates are driven by Tauri events:

```typescript
// In App.tsx useEffect:
onTaskCreated((p) => useTaskStore.getState().addTask(p));
onTaskStatusChanged((p) => useTaskStore.getState().updateStatus(p.id, p.new_status));
```

### Editor component (CodeMirror 6)

Use **CodeMirror 6** (chosen over Monaco — lighter, no native dependencies, has
`@codemirror/merge` for diff editing, the reference recommends it).

```typescript
// features/editor/FileEditor.tsx
import { EditorView, basicSetup } from 'codemirror';
import { EditorState } from '@codemirror/state';

function FileEditor({ content, filePath }: { content: string; filePath: string }) {
  const divRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);

  useEffect(() => {
    const state = EditorState.create({
      doc: content,
      extensions: [basicSetup, /* language extension based on file extension */],
    });
    const view = new EditorView({ state, parent: divRef.current! });
    viewRef.current = view;
    return () => view.destroy();
  }, []);

  return <div ref={divRef} style={{ height: '100%' }} />;
}
```

For diff view, use `@codemirror/merge`:

```typescript
import { MergeView } from '@codemirror/merge';

function DiffView({ original, modified }: { original: string; modified: string }) {
  // MergeView renders a side-by-side or unified diff with edit capabilities
}
```

### Frontend dependencies (package.json)

```json
{
  "dependencies": {
    "react": "^19",
    "react-dom": "^19",
    "zustand": "^5",
    "@xterm/xterm": "^5",
    "@xterm/addon-fit": "^0.10",
    "codemirror": "^6",
    "@codemirror/lang-javascript": "^6",
    "@codemirror/lang-rust": "^6",
    "@codemirror/merge": "^6",
    "@tauri-apps/api": "^2",
    "@tauri-apps/plugin-shell": "^2",
    "@tauri-apps/plugin-process": "^2"
  }
}

### Terminal component (xterm.js)

```typescript
// features/terminal/Terminal.tsx
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';

function PtyTerminal({ ptyId }: { ptyId: string }) {
  const termRef = useRef<XTerm | null>(null);
  const divRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const term = new XTerm({ cols: 80, rows: 24 });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(divRef.current!);
    fit.fit();
    termRef.current = term;

    // PTY output → terminal display
    const unlisten = onPtyOutput(ptyId, (data) => {
      term.write(new TextDecoder().decode(data));
    });

    // User input → PTY
    term.onData((input) => {
      invoke('pty_input', { ptyId, data: Array.from(new TextEncoder().encode(input)) });
    });

    return () => { unlisten(); term.dispose(); };
  }, [ptyId]);

  return <div ref={divRef} style={{ height: '100%' }} />;
}
```

---

## 13. Project-level chat (Phase 1 — architecture defined now)

> **Phase:** 1 (after Phase 0 core loop). **Phase 0 prep:** schema changes below are in the initial migration.

### Concept

Every project has one persistent "project conversation" — a chat with an agent that
runs in the project root directory. This agent is a coordinator: it reads the codebase,
inspects git history, creates sub-tasks (each on its own worktree), reviews their
output, and ships the results. Think of it as a lead developer overseeing the repo.

### Key decisions (from design grill)

| Decision | Answer |
|---|---|
| Where it runs | Project root directory (no separate worktree) |
| Delegation mechanism | Calls the same `create_task` backend API as the user's "Add Task" flow |
| Task visibility | Sub-tasks appear in the sidebar tree alongside user-created tasks |
| Sub-task access | Full: can read diffs, terminal output, conversation transcripts of sub-tasks it created |
| Direct capabilities | Git operations (status, diff, log, branch, commit, push, PR) + read-only repo access |
| Code generation | Delegated to sub-tasks — the project agent does not generate code directly |
| Number per project | Exactly one persistent conversation |
| Provider | User picks; defaults to project's `defaultAgent` setting |
| Sub-agent providers | Project agent specifies provider/model per sub-task when calling `create_task` |
| Initial context | Auto-generated project overview (repo structure, language detection, git status, recent commits) |
| Sub-task completion | Auto-post summary to project chat |
| Terminal | Yes — project view has the same conversation + terminal layout as tasks |
| UI | Clicking the project name in the sidebar opens the project chat. Clicking a task in the tree replaces the view with that task's tabs |

### Architecture

```
Sidebar                           Main view area
┌──────────────┐                  ┌─────────────────────────────────┐
│ 🏠 MyProject  │──click──▶       │ ┌─────────────────────────────┐ │
│   📁 add-auth │                  │ │ Project chat                 │ │
│   📁 fix-ci   │──click──▶       │ │ [agent messages + terminal]  │ │
│   📁 refactor │                  │ └─────────────────────────────┘ │
│              │                  │                                  │
│ [+ Add Task] │                  │  (clicking a sub-task replaces   │
│              │                  │   this with the task's tabs)     │
└──────────────┘                  └─────────────────────────────────┘
```

### How delegation works

1. User types "Add OAuth to the API" in the project chat.
2. Project agent reasons: this is a multi-file feature, spanning auth + middleware + tests.
3. Project agent calls its `create_task` tool:
   ```json
   {
     "name": "add-oauth",
     "prompt": "Implement OAuth 2.0 with PKCE. Add middleware, token storage, and tests.",
     "provider": "claude",
     "model": "claude-sonnet-4-20250514",
     "source_branch": "main"
   }
   ```
4. Backend creates a normal task + worktree + conversation, starts the agent.
5. Task appears in the sidebar as "add-oauth".
6. When the sub-agent finishes, a summary is posted to the project chat:
   > ✅ **add-oauth** completed. Diff: +340/-12 across 6 files. Exit code: 0.
7. Project agent reads the diff, reviews the changes, and can: merge, request fixes (spawn another sub-task), or push + create PR.

### Tool surface (what the project agent sees)

The project agent's provider gets these tools (in addition to the provider's built-in tools):

| Tool | Description |
|---|---|
| `create_task(name, prompt, provider?, model?, source_branch?)` | Create a sub-task on its own worktree |
| `list_tasks(status?)` | List tasks in this project |
| `read_task_output(task_id)` | Read the terminal output of a sub-task's agent |
| `read_task_diff(task_id)` | Read the git diff of a sub-task |
| `read_task_conversation(task_id)` | Read the sub-agent's conversation transcript |
| `git_status()` | Run `git status` in the project root |
| `git_diff(branch?)` | Run `git diff` in the project root |
| `git_log(limit?)` | Run `git log --oneline` |
| `git_branch_create(name)` | Create a branch |
| `git_commit(message)` | Commit staged changes |
| `git_push(branch?)` | Push to remote |
| `create_pr(title, body, base?, head?)` | Create a pull request |
| `read_file(path)` | Read a file in the project root |
| `search_code(query)` | Search the codebase |

### DB schema implications

Already applied in the Phase 0 initial migration (section 11):

- `conversations.scope` = `'task'` | `'project'`
- `conversations.task_id` is NULL for project-scoped conversations
- `tasks.created_by` = `'user'` | `'agent:<conversation_id>'` — tracks who created the task

### Phase 0 deliverables (prep only, no agent spawning)

1. **Schema** — `conversations.task_id` nullable, `conversations.scope` column, `tasks.created_by` column — in the initial migration.
2. **Sidebar** — clicking the project name dispatches a navigation event. The main view shows a stub: "Project chat — coming in Phase 1."
3. **`created_by`** — the "Add Task" flow sets `created_by = 'user'`. The field exists and is queryable; Phase 1 adds `'agent:<id>'` values.
4. **Conversation model** — `Conversation` struct has `scope: ConversationScope` enum and `task_id: Option<String>`. Project-scoped conversations can be created (manually in tests). No agent launch logic for them yet.

### Phase 1 deliverables (actual feature)

1. Project agent spawn: when the project chat opens and no agent is running, launch the default provider in the project root PTY.
2. Tool registration: register the `create_task` + git + read tools listed above.
3. Project overview generation: on first open, build a context prompt from repo structure + git log + language detection.
4. Sub-task completion hook: when a task with `created_by = 'agent:<conversation_id>'` finishes, post a summary to that conversation.
5. Full UI: clicking the project name opens the project chat (conversation + terminal).

---

## 14. Diff line comments (Phase 1 — alongside E4 diff view)

> **Phase:** 1 (bundled with E4 Git & diff view). **Dependencies:** E2-01 task model, E4 diff renderer, E14-01 keybinding registry.
> **Schema:** The `line_comments` table already exists in Phase 0's initial migration.

### Concept

When reviewing a diff (agent output, PR changes, or any git diff), the user selects
lines and a popover appears — like GitHub's review comments. They can either leave a
note for themselves, or create a task that sends the selected code + comment to an
agent to fix. Comments persist, link bidirectionally to tasks, and show live task
status inline.

### Key decisions (from design grill)

| Decision | Answer |
|---|---|
| Where | Diff view only (right sidebar: Changed/Staged/PR files) |
| On submit | Creates a new task (comment + selected code → agent prompt). Also supports "Add Note" (no agent). |
| Persistence | Line comments persist in SQLite (`line_comments` table). Survive restarts. Linked to tasks bidirectionally. |
| Selection | Contiguous lines within one file. Captures file path + line range + selected text. |
| Popover trigger | Appears on text selection, anchored near the selection. Floating "+ Comment" button → expands to text area. |
| Prompt context | Selected code + enclosing function/class + user comment + file path + branch/PR context. |
| Comment modes | "Add Note" (human-only, no agent) or "Create Task" (spawns an agent). Both persist. |
| Task linking | Comment stores `task_id`. Comment shows inline badge: "Task: fix-auth → in_progress". Task stores `source_comment_id`. Bidirectional. |
| Agent comments | Agents (project + sub) can call `add_line_comment` tool. Project agent reviewing sub-task diffs can leave specific feedback. |
| Before/after | Comments can target either side of a split diff. "Before" for understanding old code; "After" for suggesting changes. |
| Resolution | Manual by user. When linked task finishes, comment shows "→ done" but must be manually resolved. |
| Phase | Phase 1 — bundled with E4. Schema already exists in Phase 0. |

### UI flow

```
1. User opens diff view (right sidebar, Changed files tab)
2. Sees agent's changes in split diff (before | after)
3. Selects lines 42-56 on the "after" side
4. Floating button appears: ┌──────────┐
                             │ + Comment │
                             └──────────┘
5. Clicks it → popover expands:
   ┌─────────────────────────────────┐
   │ Comment on main.rs:42-56        │
   │ ┌─────────────────────────────┐ │
   │ │ This error handling should   │ │
   │ │ use Result<T,E> instead of   │ │
   │ │ unwrap().                    │ │
   │ └─────────────────────────────┘ │
   │                                 │
   │ [Add Note]    [Create Task ⚡]  │
   └─────────────────────────────────┘

6a. "Add Note" → saves comment, shows in diff gutter as 📝
6b. "Create Task" → opens a quick task dialog (pre-filled name, provider, model)
    → creates task + worktree + agent
    → comment shows: "📝 → Task: fix-error-handling ● in_progress"
    → clicking the badge jumps to the task
```

### Agent prompt template

When "Create Task" is clicked, the task's initial prompt is constructed as:

```
You are reviewing code in a git diff.

FILE: src/auth/middleware.rs
BRANCH: ade/fix-error-handling-a3f2
ENCLOSING FUNCTION: fn validate_token(token: &str) -> Result<Claims, AuthError>

SELECTED CODE (lines 42-56):
    let claims = validate_token(&token).unwrap();
    if claims.exp < now {
        panic!("token expired");
    }

COMMENT FROM REVIEWER:
This error handling should use Result<T,E> instead of unwrap() and panic!.
Propagate errors properly so the caller can handle them.

TASK:
Fix the code based on the comment above. Write the corrected implementation
and verify it compiles.
```

### Schema

The `line_comments` table already exists in the Phase 0 initial migration
(ported from the reference `0000_cuddly_scarecrow.sql`):

```sql
CREATE TABLE line_comments (
    id           TEXT PRIMARY KEY NOT NULL,
    task_id      TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    file_path    TEXT NOT NULL,
    line_number  INTEGER NOT NULL,
    line_content TEXT,
    content      TEXT NOT NULL,
    created_at   TEXT DEFAULT (datetime('now')) NOT NULL,
    updated_at   TEXT DEFAULT (datetime('now')) NOT NULL,
    sent_at      TEXT
);
CREATE INDEX idx_line_comments_task_file ON line_comments(task_id, file_path);
```

**Phase 1 additions** — extend this table with new columns:

```sql
-- Migration to add comment features:
ALTER TABLE line_comments ADD COLUMN source_side TEXT DEFAULT 'after';  -- 'before' | 'after'
ALTER TABLE line_comments ADD COLUMN line_end INTEGER;  -- for multi-line selections
ALTER TABLE line_comments ADD COLUMN linked_task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL;
ALTER TABLE line_comments ADD COLUMN resolved INTEGER DEFAULT 0;  -- 0=open, 1=resolved
ALTER TABLE line_comments ADD COLUMN resolved_at TEXT;
ALTER TABLE line_comments ADD COLUMN created_by TEXT DEFAULT 'user';  -- 'user' | 'agent:<id>'
```

**Corresponding task changes:**

```sql
ALTER TABLE tasks ADD COLUMN source_comment_id TEXT REFERENCES line_comments(id) ON DELETE SET NULL;
```

### Event flow

```
User selects code → popover → "Create Task"
  │
  ├─→ line_comments row created (id = "lc_xxx")
  │
  ├─→ task created (source_comment_id = "lc_xxx")
  │     └─→ sidebar updates, task appears
  │
  ├─→ InternalEvent::CommentCreated { id, task_id, file_path, line_number }
  │     └─→ diff view updates: comment marker appears in gutter
  │
  └─→ agent launched in task worktree with constructed prompt

... time passes, agent works ...

agent finishes
  │
  ├─→ InternalEvent::TaskStatusChanged { ... status: "review" }
  │     └─→ comment badge updates: "→ review" (not yet resolved)
  │
user verifies, clicks ✓ on comment
  │
  └─→ line_comments.resolved = 1
        └─→ InternalEvent::CommentResolved { id }
              └─→ comment marker changes to ✓ (green)
```

### Tool surface for agents

```rust
/// Agents can call this to leave review comments.
async fn add_line_comment(
    task_id: String,       // which task's diff to comment on
    file_path: String,
    line_start: u32,
    line_end: u32,         // same as line_start for single-line
    content: String,       // the comment text
    source_side: String,   // "before" | "after"
) -> Result<CommentDto, Error>;
```

### Phase 0 prep

1. **Schema** — the base `line_comments` table is in the initial migration (already documented in §11).
2. **No Phase 0 code** — the table exists but no UI or agent logic touches it until Phase 1.
3. **E4 ticket note** — when implementing the diff view (E4-04), build the gutter with a placeholder for future comment markers.

---

## 15. Start here — implementation order for Phase 0

1. **E0** — Workspace bootstrap. Everything below needs this.
2. **E1-01** — Database. The migration runner + schema are the foundation.
3. **E1-02** — Settings. Needed by every feature.
4. **E3-01** — Provider registry. Needed by agent launch. Can run in parallel with E1-03.
5. **E1-03** — Projects. First user-facing feature.
6. **E2-01 + E2-03** — Task model + naming. Can run in parallel.
7. **E2-02** — Worktrees. Needs E2-03 (branch names).
8. **E2-04** — Add Task flow. First end-to-end user flow.
9. **E2-05** — Conversation supervisor. Backbone for agent sessions.
10. **E3-02/03/04/08** — Provider dependencies, prompt strategies, auto-approve, env allowlist. Must be done before E2-06.
11. **E2-06** — Agent launch. The big one — first working agent.
12. **E14-01** — Keybinding registry. Needed by all UI below. Can start as early as after E1-02.
13. **E2-08** — Conversations UI.
14. **E1-04/05/06/07/08/09** — Sidebar, settings UI, lifecycle scripts, preserve patterns, onboarding, command palette. Parallelizable.
15. **E2-07** — Persistence/resume.
16. **E2-09/10** — Teardown + navigation.

---

## 16. Test patterns (required for every ticket)

Every ticket's merge gate includes `cargo test`. These are the patterns to use.

### DB integration tests

```rust
// ade-core/tests/db_integration.rs
use ade_core::db::SqliteDb;
use std::sync::Arc;

#[test]
fn test_migration_runner_idempotent() {
    // Use temp file — never touch the real DB path
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");

    let db = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();

    // Running init again on the same file is a no-op
    let db2 = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
    // Assert both connections work, same schema, no duplicate migration error
}

#[test]
fn test_versioned_json_corrupt_value_returns_none() {
    // Write a corrupt JSON blob to a versioned column
    // read_versioned returns None, never panics
}

#[test]
fn test_kv_roundtrip() {
    let db = SqliteDb::init_in_memory().unwrap();
    db.kv_set("test_key", "test_value").unwrap();
    assert_eq!(db.kv_get("test_key").unwrap(), Some("test_value".into()));
}
```

### Git/worktree integration tests

```rust
// ade-git/tests/worktree_integration.rs
use ade_git::GitOps;
use std::process::Command;

fn init_temp_repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let repo_path = tmp.path().join("repo");

    // Create a bare repo + clone it (so we have a remote to push to)
    let status = Command::new("git")
        .args(["init", "--bare", repo_path.to_str().unwrap()])
        .status().unwrap();
    assert!(status.success());

    // Clone it to a working copy
    let work_path = tmp.path().join("work");
    let status = Command::new("git")
        .args(["clone", repo_path.to_str().unwrap(), work_path.to_str().unwrap()])
        .status().unwrap();
    assert!(status.success());

    // Create initial commit (needed for worktrees)
    std::fs::write(work_path.join("README.md"), "# test").unwrap();
    let status = Command::new("git")
        .args(["-C", work_path.to_str().unwrap(), "add", "."])
        .status().unwrap();
    Command::new("git")
        .args(["-C", work_path.to_str().unwrap(), "commit", "-m", "init"])
        .status().unwrap();

    (tmp, work_path)
}

#[test]
fn test_worktree_create_and_prune() {
    let (_tmp, repo_path) = init_temp_repo();
    let git = CliGit::new(); // or Git2Ops::new()

    // Create a branch
    git.branch_create(&repo_path, "test-branch", "main").unwrap();

    // Create a worktree
    let wt_path = repo_path.parent().unwrap().join("wt-test");
    git.worktree_add(&repo_path, &wt_path, "test-branch").unwrap();

    assert!(wt_path.join(".git").exists());
    assert!(git.is_worktree(&wt_path).unwrap());

    // List worktrees
    let entries = git.worktree_list(&repo_path).unwrap();
    assert!(entries.iter().any(|e| e.path == wt_path));

    // Prune
    git.worktree_remove(&repo_path, &wt_path).unwrap();
    assert!(!wt_path.exists());
}
```

### PTY/agent integration test

```rust
// ade-terminal/tests/agent_launch.rs
use ade_terminal::PtyManager;

#[test]
fn test_spawn_shell_and_read_output() {
    let pty = PortablePtyManager::new();
    let handle = pty.spawn(PtyConfig {
        id: PtyId("test-shell".into()),
        cwd: std::env::current_dir().unwrap(),
        command: vec!["echo".into(), "hello".into()],
        env: Default::default(),
        cols: 80,
        rows: 24,
        use_tmux: false,
    }).unwrap();

    // Read output until process exits
    let mut output = Vec::new();
    let mut rx = handle.output;
    while let Ok(data) = rx.blocking_recv() {
        output.extend(data);
    }

    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("hello"));

    let exit_code = handle.exit.blocking_recv().unwrap();
    assert_eq!(exit_code, 0);
}

#[test]
fn test_env_allowlist_does_not_leak_secrets() {
    // Set a non-allowlisted var in the test process
    std::env::set_var("SECRET_TOKEN", "abc123");

    let env = build_agent_env(&ProviderId("test".into()), &HashMap::new(), None);
    assert!(!env.contains_key("SECRET_TOKEN"));
}
```

### Migration test (separate test binary)

```rust
// ade-core/tests/migrations.rs
use ade_core::db::SqliteDb;

#[test]
fn test_fresh_install_creates_all_tables() {
    let db = SqliteDb::init_in_memory().unwrap();
    let conn = db.conn().lock().unwrap();

    // Every table from schema §11 exists
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    for expected in &[
        "app_settings", "kv", "projects", "project_remotes",
        "project_settings", "tasks", "workspaces", "conversations",
        "terminals", "messages", "editor_buffers", "line_comments",
        "automations", "automation_runs", "ssh_connections",
        "provider_accounts", "migrations",
    ] {
        assert!(tables.contains(&expected.to_string()),
            "Missing table: {}", expected);
    }
}

#[test]
fn test_migration_upgrade_path() {
    // Start with an empty DB at version N-1, add a new migration,
    // verify it applies without errors and journal is updated.
}
```

### Test helper: in-memory DB

```rust
// ade-core/src/db/connection.rs
impl SqliteDb {
    /// Creates an in-memory database for tests.
    /// Runs all migrations, returns a fully-initialized DB.
    pub fn init_in_memory() -> Result<Arc<Self>, Error> {
        let conn = rusqlite::Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = MEMORY;")?;
        Self::run_migrations(&conn)?;
        Ok(Arc::new(Self {
            conn: std::sync::Mutex::new(conn),
            path: PathBuf::from(":memory:"),
        }))
    }
}
```

### Rules

1. **Every domain module has at least one integration test** that touches the DB or spawns a real process.
2. **Tests use `tempfile::tempdir()` or `:memory:`** — never the default DB path, never `~/Library/Application Support`.
3. **Git tests create real temp repos** with `git init` (shell out for setup, use git2 for the operations under test).
4. **PTY tests are `#[cfg(not(windows))]`** in Phase 0 — Windows PTY testing comes after E2-06 confirms ConPTY behavior.
5. **Migration tests are a separate binary** (`tests/migrations.rs`) — they run the full migration chain from scratch.

---

## 17. Stack constraints & known risks

### git2 (!Sync)

`git2::Repository` is `!Sync`. All git operations must be serialized. Use either:

- A dedicated `std::thread` with a channel receiving `Box<dyn FnOnce + Send>` closures (simplest, recommended for Phase 0), or
- `tokio::task::spawn_blocking` with a `std::sync::Mutex<Repository>` (if already on tokio).

The `GitOps` trait's implementor (`Git2Ops` or `CliGit`) handles this internally — consumers just call the trait methods, which block internally.

### portable-pty

| Platform | Status | Notes |
|---|---|---|
| macOS | ✅ Works | Uses native forkpty |
| Linux | ✅ Works | Uses native forkpty |
| Windows | ⚠️ Requires ConPTY | Windows 10 1809+ only. UTF-8 output may differ from node-pty. Test with cmd + PowerShell. |

PTY tests should be `#[cfg(not(windows))]` until Windows behavior is verified.

### rusqlite WAL mode

- `PRAGMA journal_mode=WAL` — enables concurrent reads (writers still serialize).
- `PRAGMA busy_timeout=5000` — 5s wait on lock contention instead of immediate error.
- `PRAGMA foreign_keys=ON` — enforced at the connection level; FKs are checked on every write.
- Connection is behind `std::sync::Mutex` — shared across threads via `Arc`.
- `init_in_memory()` uses `journal_mode=MEMORY` for speed in tests.

### Tauri 2 CSP (Content Security Policy)

Tauri 2's webview has a default CSP. For our frontend to load xterm.js addons, CodeMirror, and potentially external images:

```json
// tauri.conf.json
{
  "app": {
    "security": {
      "csp": "default-src 'self'; img-src 'self' asset: https:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self' https://api.github.com"
    }
  }
}
```

Adjust `connect-src` when GitHub API integration (E8) is added. No `eval()` in our code, so `script-src` stays strict.

### Tauri plugin allowlist

Only enable plugins we actually use. For Phase 0:

```json
{
  "plugins": {
    "shell": { "open": true },
    "process": { "relaunch": true },
    "updater": { "active": true },
    "os": {}
  }
}
```

No `fs` plugin — we read files through Rust commands, not the webview directly.
No `clipboard` plugin — xterm.js handles copy/paste internally.

### Build-time dependencies

- **git2** requires `libgit2` — needs `pkg-config` + `libgit2-dev` on Linux, `cmake` on macOS. These are in CI (apt-get install). For local dev, `brew install libgit2` (macOS) or `apt install libgit2-dev` (Linux).
- **rusqlite** with `bundled` feature compiles SQLite from source — no system dependency. This is the safest option for cross-platform consistency.
- **portable-pty** has no external deps on macOS/Linux (uses libc forkpty). On Windows it needs no extra deps beyond the ConPTY API.


---

## 18. Decision log (E1-01 → E2-03)

Each decision is also documented at its code site; this log is the scan-able
index. Numbered records (with context + rationale) live in `decisions/`
(0001–0004, see `decisions/README.md` for the convention). Items marked
*deviation* change this document or the ticket text.

| # | Ticket | Decision | Where it lives |
|---|---|---|---|
| D1 | E1-01 | Migration runner tracks progress by `MAX(created_at)` (journal `when`), records `sha256(sql)`, and **hash-verifies already-applied migrations on every init** (stricter than the reference, which records but never checks) — "hand-edit of a numbered migration is not possible". | `ade-core/src/db/migrations.rs` |
| D2 | E1-01 | FTS tables live **outside** migrations, version-gated via `kv` (`fts_version='3'`, `file_index_version='4'`) exactly as later tickets read them. | `ade-core/src/db/migrations.rs` |
| D3 | E1-01 | Legacy DB copy (`emdash4/3.db` → `ade.db` via `VACUUM INTO`, secrets cleared) is per-spec, but a copied reference DB is *not* schema-identical to Phase 0 — real data migration is a later-phase concern; init fails loudly rather than corrupting. | `ade-core/src/db/connection.rs` |
| D4 | E1-02 | **Deviation (§6.2):** `SettingsStore` is object-safe (JSON surface) so §7's `Arc<dyn SettingsStore>` works; typed `SettingKey<T>` wrappers live on `DbSettingsStore`. Trait extended with the project-settings surface for `projects`. | `ade-core/src/settings/service.rs` (§6.2 here) |
| D5 | E1-02 | App settings are **delta-vs-defaults**: updating to the default deletes the row; reads deep-merge defaults. Values validated by canonical round-trip (unknown keys stripped, zod-parse behavior). | `ade-core/src/settings/service.rs` |
| D6 | E1-02 | Effective project-settings precedence: `defaults < .ade.json < DB-shareable` (later wins, reference `mergeShareableProjectSettings`). `update_project_settings` is **full-replace** (reference `update()`), so callers read-modify-write. | `ade-core/src/settings/service.rs` |
| D7 | E1-02 | Legacy `.emdash.json` migration is a **one-shot at first access** (marked done even without a file, single marker covers base+shareable). Shareable merge is unconditional — the reference gates it on git-tracking (needs `ade-git`). | `ade-core/src/settings/service.rs` |
| D8 | E1-03 | **Deviation (§6.4):** `GitOps` trait lives in `ade-core::git` (leaf rule); `ade-git::CliGit` implements it and re-exports. Phase 0 is git **CLI** (Command arg arrays, no shell) — git2 bindings with E2-02. | `ade-core/src/git.rs`, `ade-git/src/lib.rs` (§6.4 here) |
| D9 | E1-03 | Base-ref resolution ports reference `computeBaseRef` `normalize()` exactly: slash-branches stay bare (`feature/x`), plain branches get the remote prefix; refinement derives the remote from the *detected* ref. `remote_head` is local-only (symbolic-ref) — the `git remote show` fallback (a network call that can hang) was dropped. | `ade-core/src/projects/mod.rs`, `ade-git/src/lib.rs` |
| D10 | E1-03 | `.ade/` git exclusion writes `.git/info/exclude` (never a tracked `.gitignore`); in linked worktrees the entry lands in the per-worktree exclude (reference writes the common dir — E2-02 can align). `worktree_remove` is `rm -rf`+prune until E2-02 switches to `git worktree remove`. | `ade-core/src/projects/provider.rs`, `ade-git/src/lib.rs` |
| D11 | E1-03 | `close_project` is a Phase 0 stub — session/workspace/preview teardown (tmux `detach` vs `terminate`) lands with E2-05/E2-02/E13. `RepoHostProvider` stubs GitHub repo creation (E8). | `ade-core/src/projects/provider.rs` |
| D12 | E2-01 | **No status-transition allowlist** (reference-faithful) — any lifecycle status change is allowed; guards are same-status no-op + not-found. `InvalidStatusTransition` reserved for a future state machine. | `ade-core/src/tasks/` (ADR-0005) |
| D13 | E2-01 | Create is **atomic** (task + workspace + initial conversation in one tx, rollback on failure); events fire post-commit, non-fatal. `tasks.workspace_intent` is legacy, never written. | `ade-core/src/tasks/mod.rs` (ADR-0005) |
| D14 | E2-01 | **Provision fast-path contract**: idempotent re-fire of `task:provisioned` + recency touch; real workspace bootstrap is E2-02. Delete is hard (FK cascade); archive is non-destructive. | `ade-core/src/tasks/mod.rs` (ADR-0005) |
| D15 | E2-03 | Random task names are `adjective-noun-verb` (vendored `human-id@4.2.0` word lists, exact combination order); title slugs implemented directly (nbranch semantics). | `ade-core/src/tasks/naming.rs` (ADR-0006) |
| D16 | E2-03 | Branch resolution is pure + faithful: Linear branch names verbatim; `ade/<name>-<5-char base36 suffix>` when `appendRandomBranchSuffix`; suffix entropy from uuid (no rand dep); settings read by the caller. | `ade-core/src/tasks/naming.rs` (ADR-0006) |

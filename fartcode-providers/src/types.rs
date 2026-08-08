//! Pure provider types (ticket E3-01).

/// The prompt delivery strategy for the provider's `behavior.prompt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptStrategy {
    /// Prompt passed on the command line; `flag` is the argv flag (empty for
    /// positional) or `None` when the strategy uses stdin/keystroke.
    Argv { flag: Option<String> },
    /// Prompt piped to stdin.
    StdinPipe,
    /// Prompt typed into the TUI after startup (E3-03).
    Keystroke,
}

/// Per-provider prompt behavior (reference `buildStandardCommand` args +
/// `prompt` capability).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptDescriptor {
    pub strategy: PromptStrategy,
    /// Optional argv flag that turns on auto-approve (e.g. claude's
    /// `--dangerously-skip-permissions`). `None` for providers that gate
    /// auto-approve via `auto_approve_env` instead.
    pub auto_approve_flag: Option<String>,
    /// Env vars that turn on auto-approve when the provider has no argv flag
    /// (reference `extraEnv`: mimocode → `MIMOCODE_PERMISSION`,
    /// opencode → `OPENCODE_PERMISSION`). Merged into the launch env.
    pub auto_approve_env: Option<Vec<(String, String)>>,
    pub initial_prompt_flag: Option<String>,
    pub resume_flag: Option<String>,
    pub session_id_flag: Option<String>,
    pub session_id_on_resume_only: bool,
    pub resume_without_session_flag: Option<String>,
    pub model_flag: Option<String>,
    /// E3-03 additions (reference buildStandardCommand spec).
    pub new_conversation_flag: Option<String>,
    pub session_id_always: bool,
    pub omit_auto_approve_on_resume: bool,
    pub initial_prompt_via_stdin_pipe: bool,
    pub deduplicate_flags: Vec<String>,
    /// Keystroke delivery: sequence typed to submit the prompt (default
    /// `\r`), and the delay before it when the payload must land first.
    pub submit_sequence: Option<String>,
    pub submit_delay_ms: Option<u64>,
    pub default_args: Vec<String>,
}

/// The 12 capability flags, ported exactly from the reference
/// `core/agents/plugins/capabilities/*` (kind → boolean: `supported`/
/// `selectable`/`config`/`plugin`/`resumable` are true, `none` is false).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
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

impl Capabilities {
    pub fn has(&self, capability: Capability) -> bool {
        match capability {
            Capability::Acp => self.acp,
            Capability::Auth => self.auth,
            Capability::AutoApprove => self.auto_approve,
            Capability::Effort => self.effort,
            Capability::Hooks => self.hooks,
            Capability::HostDependency => self.host_dependency,
            Capability::Mcp => self.mcp,
            Capability::Models => self.models,
            Capability::Plugins => self.plugins,
            Capability::Prompt => self.prompt,
            Capability::Sessions => self.sessions,
            Capability::Trust => self.trust,
        }
    }

    pub fn iter(&self) -> Vec<(&'static str, bool)> {
        vec![
            ("acp", self.acp),
            ("auth", self.auth),
            ("autoApprove", self.auto_approve),
            ("effort", self.effort),
            ("hooks", self.hooks),
            ("hostDependency", self.host_dependency),
            ("mcp", self.mcp),
            ("models", self.models),
            ("plugins", self.plugins),
            ("prompt", self.prompt),
            ("sessions", self.sessions),
            ("trust", self.trust),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Acp,
    Auth,
    AutoApprove,
    Effort,
    Hooks,
    HostDependency,
    Mcp,
    Models,
    Plugins,
    Prompt,
    Sessions,
    Trust,
}

/// How a provider authenticates (reference `auth.methods`, E3-07 "login
/// methods, API key registry").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethodKind {
    /// CLI-managed login (e.g. `claude auth login`): OAuth tokens live in
    /// the CLI's own credential store, NOT in the fartCode keyring. Uses
    /// the user's subscription (Claude Pro/Max) — no per-token API
    /// charges. fartCode must never pass an API-key env var in this mode:
    /// its mere presence flips the CLI to API-key billing.
    CliLogin,
    /// Pay-per-token env-var credential stored in the OS keyring (e.g.
    /// `ANTHROPIC_API_KEY`).
    ApiKey,
}

impl AuthMethodKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthMethodKind::CliLogin => "cli-login",
            AuthMethodKind::ApiKey => "api-key",
        }
    }
}

/// One authentication method a provider accepts (reference `auth.methods`
/// entries: claude exposes `cli-login` "Sign in with Claude Code" and
/// `api-key` "Use an Anthropic API key").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: AuthMethodKind,
    /// Env vars read for `ApiKey` methods (e.g. `ANTHROPIC_API_KEY`).
    pub env_vars: Vec<String>,
    /// CLI args that start the login flow (`CliLogin`), e.g.
    /// `["auth", "login"]`.
    pub login_args: Vec<String>,
    /// CLI args for the auth status probe (`CliLogin`), e.g.
    /// `["auth", "status"]` — the CLI prints JSON (claude does by
    /// default; `--json` is accepted on the exact versions we target).
    pub status_args: Vec<String>,
}

/// Everything the app knows about a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    pub website_url: Option<String>,
    pub capabilities: Capabilities,
    pub prompt: PromptDescriptor,
    /// Binary names for detection / launch (reference `hostDependency`
    /// `binaryNames`, defaulting to the provider id).
    pub binaries: Vec<String>,
    pub default_model: Option<String>,
    /// Auth env var names the provider reads (e.g. `ANTHROPIC_API_KEY`).
    /// For providers with `auth_methods`, this is the flattened api-key
    /// method's vars (legacy behavior, kept for callers that predate
    /// methods).
    pub env_vars: Vec<String>,
    /// Login + API-key methods (reference `auth.methods`). Empty for
    /// providers whose only auth surface is `env_vars`.
    pub auth_methods: Vec<AuthMethod>,
}

impl ProviderDescriptor {
    /// The auth method with `id`, if any.
    pub fn auth_method(&self, id: &str) -> Option<&AuthMethod> {
        self.auth_methods.iter().find(|m| m.id == id)
    }

    /// Preferred method: the first api-key method, else the first login
    /// method, else `None`. Drives `resolve_env` for accounts created
    /// before the method column existed (legacy rows behave as api-key).
    pub fn default_auth_method(&self) -> Option<&AuthMethod> {
        self.auth_methods
            .iter()
            .find(|m| m.kind == AuthMethodKind::ApiKey)
            .or_else(|| self.auth_methods.first())
    }

    /// The cli-login method, if the provider has one.
    pub fn login_method(&self) -> Option<&AuthMethod> {
        self.auth_methods
            .iter()
            .find(|m| m.kind == AuthMethodKind::CliLogin)
    }
}

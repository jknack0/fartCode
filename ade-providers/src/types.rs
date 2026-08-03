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
    pub auto_approve_flag: Option<String>,
    pub initial_prompt_flag: Option<String>,
    pub resume_flag: Option<String>,
    pub session_id_flag: Option<String>,
    pub session_id_on_resume_only: bool,
    pub resume_without_session_flag: Option<String>,
    pub model_flag: Option<String>,
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
    pub env_vars: Vec<String>,
}

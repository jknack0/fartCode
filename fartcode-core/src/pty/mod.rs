//! Prompt delivery strategies (ticket E3-03) + agent env allowlist (E3-08).
//!
//! Ports the reference prompt delivery layer: `buildStandardCommand` (argv /
//! stdin-pipe), the large-prompt spill to a temp `.md`, and the keystroke
//! injector (`keystroke-injection.ts`: inject after the TUI produces output
//! and stays quiet for a beat). The actual PTY (fartcode-terminal / E2-06) drives
//! the injector; this module is the delivery logic + the seam.

pub mod env_allowlist;
pub mod launcher;
pub mod sessions;
pub mod tmux;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use fartcode_providers::ProviderDescriptor;

/// Args passed into the command builder (reference `CommandContext`).
#[derive(Debug, Clone, Default)]
pub struct CommandContext {
    /// Absolute path to the CLI binary.
    pub cli: String,
    /// User-configured extra args.
    pub extra_args: Vec<String>,
    pub auto_approve: bool,
    pub initial_prompt: Option<String>,
    /// Emdash conversation UUID — the session token for providers that track
    /// their own session across the app lifetime.
    pub session_id: Option<String>,
    /// Provider-native session id (set by the agent classifier for resume).
    pub provider_session_id: Option<String>,
    pub is_resuming: bool,
    /// Empty string = the CLI's default model.
    pub model: String,
}

/// The built launch command (reference `AgentCommand`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCommand {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// Prompts above this length spill to a temp `.md` file (reference
/// maxLength default ~32KB for argv strategies).
/// Spill boundary: prompts of AT LEAST this length spill (>=).
pub const MAX_ARGV_PROMPT_LENGTH: usize = 32 * 1024;

/// Spill directory relative to the worktree (`.fartCode/prompts`).
pub const PROMPT_SPILL_DIR: &str = ".fartCode/prompts";

/// Auto-approve resolution (ticket E3-04): whether `ctx.auto_approve` should
/// be true for a launch.
///
/// - **Trust gating**: a trusted worktree makes auto-approve implicitly on
///   (reference: trust ⇒ no permission prompts). Callers MUST pass the
///   *resolved* trust state — E2-04's `should_auto_trust` / the trust row —
///   not the raw `autoTrustWorktrees` setting, or every launch would
///   auto-approve even on an explicitly untrusted worktree.
/// - **Conversation config**: the conversation's own `autoApprove` (e.g. the
///   dialog's "run with auto-approve" toggle).
/// - **Forced**: `tasks.autoApproveByDefault` always passes the flag.
pub fn resolve_auto_approve(
    conversation_auto_approve: bool,
    auto_approve_by_default: bool,
    worktree_is_trusted: bool,
) -> bool {
    worktree_is_trusted || conversation_auto_approve || auto_approve_by_default
}

/// The auto-approve launch mechanism for one provider (ADR-0013): the argv
/// flag or env vars that turn it on, gated on the provider's capability and
/// `omitAutoApproveOnResume` (kimi). Shared by [`build_command`] and the
/// app's raw agent-terminal open path so the mechanism has one definition.
pub fn auto_approve_mechanism(
    provider: &ProviderDescriptor,
    auto_approve: bool,
    is_resuming: bool,
) -> (Vec<String>, Vec<(String, String)>) {
    if !auto_approve || !provider.capabilities.auto_approve {
        return (Vec::new(), Vec::new());
    }
    let spec = &provider.prompt;
    if spec.omit_auto_approve_on_resume && is_resuming {
        return (Vec::new(), Vec::new());
    }
    let args = spec
        .auto_approve_flag
        .as_deref()
        .map(split_flag)
        .unwrap_or_default();
    let env = spec.auto_approve_env.clone().unwrap_or_default();
    (args, env)
}

// Convenience wrapper deliberately removed: a settings-group version of
// resolve_auto_approve would pass the raw autoTrustWorktrees setting
// (default true) and auto-approve every launch. E2-06 must pass the
// *resolved* trust state (E2-04's should_auto_trust / trust row) instead.
/// Spill a long prompt to `<worktree>/.fartCode/prompts/<uuid>.md` and return the
/// path. Callers must `cleanup_spilled_prompt` it when the agent exits
/// (acceptance 2: "file is cleaned on exit").
pub fn spill_prompt(prompt: &str, worktree_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = worktree_dir.join(PROMPT_SPILL_DIR);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.md", uuid::Uuid::new_v4()));
    std::fs::write(&path, prompt)?;
    Ok(path)
}

/// Removes a spilled prompt file (best-effort; the dir stays).
pub fn cleanup_spilled_prompt(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Builds the launch command for `provider` (reference `buildStandardCommand`
/// — ported faithfully: defaultArgs, session/resume flags, auto-approve,
/// model, extraArgs, initial prompt positional/flag/stdin-pipe, dedupe).
///
/// The initial prompt is only delivered on FRESH sessions (reference: resume
/// never re-sends the initial prompt).
pub fn build_command(ctx: &CommandContext, provider: &ProviderDescriptor) -> AgentCommand {
    let spec = &provider.prompt;
    let mut args: Vec<String> = Vec::new();
    let mut env: HashMap<String, String> = HashMap::new();

    // Default args (e.g. goose's ['run', '-s']).
    args.extend(spec.default_args.iter().cloned());

    // Session / resume logic.
    let raw_session_id = if spec.session_id_on_resume_only {
        ctx.provider_session_id.clone()
    } else {
        ctx.session_id.clone()
    };
    let valid_session_id = raw_session_id;

    if ctx.is_resuming {
        if spec.session_id_always && spec.session_id_flag.is_some() && ctx.session_id.is_some() {
            // Always inject session id on resume too (e.g. antigravity).
            append_flag_value(
                &mut args,
                spec.session_id_flag.as_deref().unwrap(),
                ctx.session_id.as_deref().unwrap(),
            );
        } else if let Some(resume_flag) = &spec.resume_flag {
            if let Some(sid) = valid_session_id {
                args.extend(split_flag(resume_flag));
                args.push(sid);
            } else if spec.session_id_flag.is_some() && !spec.session_id_on_resume_only {
                // Unreachable with `session_id` set (that value would have
                // been `valid_session_id` above); kept for branch-order
                // parity with the reference.
                if let Some(sid) = &ctx.session_id {
                    args.extend(split_flag(resume_flag));
                    args.push(sid.clone());
                }
            } else if let Some(without) = &spec.resume_without_session_flag {
                args.extend(split_flag(without));
            } else {
                args.extend(split_flag(resume_flag));
            }
        }
    } else {
        // Fresh session.
        if let Some(sid) = &spec.session_id_flag {
            if !spec.session_id_on_resume_only {
                if let Some(session_id) = &ctx.session_id {
                    append_flag_value(&mut args, sid, session_id);
                }
            } else if let Some(new_flag) = &spec.new_conversation_flag {
                args.push(new_flag.clone());
            }
        } else if let Some(new_flag) = &spec.new_conversation_flag {
            args.push(new_flag.clone());
        }
    }

    // Auto-approve (E3-04): the flag is added only when the provider
    // declares the autoApprove capability (reference: capability-gated) AND
    // `ctx.auto_approve` is set (which itself is resolved from the
    // conversation config + task settings via `resolve_auto_approve`).
    // `omitAutoApproveOnResume` (kimi) suppresses it on resume. Providers
    // without an argv flag gate auto-approve via env (reference `extraEnv`).
    let (aa_args, aa_env) = auto_approve_mechanism(provider, ctx.auto_approve, ctx.is_resuming);
    args.extend(aa_args);
    env.extend(aa_env);

    // Model selection.
    if let Some(model_flag) = &spec.model_flag {
        if !ctx.model.is_empty() {
            append_flag_value(&mut args, model_flag, &ctx.model);
        }
    }

    // User extra args.
    args.extend(ctx.extra_args.iter().cloned());

    // Initial prompt — only on fresh sessions.
    let mut via_stdin_pipe = false;
    if !ctx.is_resuming {
        if let Some(prompt) = ctx
            .initial_prompt
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            if spec.initial_prompt_via_stdin_pipe {
                via_stdin_pipe = true;
            } else if let Some(flag) = &spec.initial_prompt_flag {
                if flag.is_empty() {
                    args.push(prompt.to_string());
                } else {
                    args.extend(split_flag(flag));
                    args.push(prompt.to_string());
                }
            }
        }
    }

    let final_args = if spec.deduplicate_flags.is_empty() {
        args
    } else {
        dedupe_flags(args, &spec.deduplicate_flags)
    };

    let command = AgentCommand {
        command: ctx.cli.clone(),
        args: final_args,
        env,
    };

    if via_stdin_pipe {
        if let Some(prompt) = ctx
            .initial_prompt
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            return wrap_with_stdin_pipe(command, prompt);
        }
    }
    command
}

/// Builds the launch command and spills an oversized ARGV prompt to a temp
/// file in `worktree_dir` (acceptance 2): the prompt is passed as `@<path>`
/// and the caller cleans the returned file on agent exit. Stdin-pipe and
/// keystroke payloads never go through argv — they keep the raw prompt (no
/// spill, no `@path` corruption).
pub fn build_command_with_spill(
    ctx: &CommandContext,
    provider: &ProviderDescriptor,
    worktree_dir: &Path,
) -> Result<(AgentCommand, Option<PathBuf>), std::io::Error> {
    let mut ctx = ctx.clone();
    let mut spilled = None;
    if !ctx.is_resuming
        && matches!(
            provider.prompt.strategy,
            fartcode_providers::PromptStrategy::Argv { .. }
        )
    {
        if let Some(prompt) = ctx.initial_prompt.take() {
            let (deliverable, path) = maybe_spill_prompt(&prompt, worktree_dir)?;
            spilled = path;
            ctx.initial_prompt = Some(deliverable);
        }
    }
    let cmd = build_command(&ctx, provider);
    Ok((cmd, spilled))
}

/// `printf '%s\n' <prompt> | <cli> <args>` (reference `wrapWithStdinPipe`).
pub fn wrap_with_stdin_pipe(cmd: AgentCommand, prompt: &str) -> AgentCommand {
    let agent_line = std::iter::once(&cmd.command)
        .chain(cmd.args.iter())
        .map(|a| quote_shell_arg(a))
        .collect::<Vec<_>>()
        .join(" ");
    let shell_line = format!("printf '%s\\n' {} | {agent_line}", quote_shell_arg(prompt));
    AgentCommand {
        command: "bash".into(),
        args: vec!["-c".into(), shell_line],
        env: cmd.env,
    }
}

/// POSIX shell quoting (reference `quoteShellArg`).
pub fn quote_shell_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "''".into();
    }
    if !arg
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || ".,_:/@=+-".contains(c)))
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

/// Pipes a long argv prompt through the spill: if `prompt.len()` exceeds
/// `MAX_ARGV_PROMPT_LENGTH`, writes it to a temp file in the worktree and
/// returns `@<path>` (the `@file` prompt convention). Returns
/// `(deliverable, spilled_path)`.
pub fn maybe_spill_prompt(
    prompt: &str,
    worktree_dir: &Path,
) -> Result<(String, Option<PathBuf>), std::io::Error> {
    if prompt.len() >= MAX_ARGV_PROMPT_LENGTH {
        let path = spill_prompt(prompt, worktree_dir)?;
        Ok((format!("@{}", path.display()), Some(path)))
    } else {
        Ok((prompt.to_string(), None))
    }
}

fn split_flag(flag: &str) -> Vec<String> {
    flag.split_whitespace().map(String::from).collect()
}

fn append_flag_value(args: &mut Vec<String>, flag: &str, value: &str) {
    if let Some(stripped) = flag.strip_suffix('=') {
        args.push(format!("{stripped}={value}"));
    } else {
        args.extend(split_flag(flag));
        args.push(value.to_string());
    }
}

fn dedupe_flags(args: Vec<String>, flags: &[String]) -> Vec<String> {
    let singletons: std::collections::HashSet<&str> = flags.iter().map(String::as_str).collect();
    let mut seen = std::collections::HashSet::new();
    args.into_iter()
        .filter(|arg| {
            if !singletons.contains(arg.as_str()) {
                return true;
            }
            seen.insert(arg.clone())
        })
        .collect()
}

/// The keystroke injection state machine (reference
/// `keystroke-injection.ts`): inject the prompt only after the TUI produces
/// output AND then stays quiet for `quiet_period_ms` (fixed delays race
/// startup), with a `max_wait_ms` fallback. Skipped on resume.
///
/// The PTY owner (E2-06) calls `on_data` as output arrives and `on_exit`
/// when the child dies, each with a monotonic millisecond clock.
#[derive(Debug, Clone)]
pub struct PromptInjector {
    payload: String,
    submit_sequence: String,
    submit_delay_ms: Option<u64>,
    quiet_period_ms: u64,
    max_wait_ms: u64,
    /// Monotonic clock value at construction — max-wait is measured from
    /// here, so the caller can pass any process-lifetime clock.
    started_at: u64,
    state: InjectState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InjectState {
    /// No output yet; waiting for the first bytes.
    WaitingForOutput,
    /// Saw output; quiet timer running (deadline = last_output_at + quiet).
    Quiet {
        last_output_at: u64,
    },
    Injected,
}

/// What the owner must write to the PTY next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptInjection {
    /// Write the payload now; then write `submit_sequence` after
    /// `submit_delay_ms` (if any).
    Payload,
    /// Write the submit sequence now.
    Submit,
    /// Write payload + submit sequence together.
    PayloadAndSubmit,
}

impl PromptInjector {
    /// `now_ms` is the caller's monotonic clock at construction; max-wait is
    /// measured from it (any process-lifetime clock works).
    pub fn new(
        initial_prompt: &str,
        submit_sequence: Option<&str>,
        submit_delay_ms: Option<u64>,
        quiet_period_ms: u64,
        max_wait_ms: u64,
        now_ms: u64,
    ) -> Option<Self> {
        let payload = initial_prompt.trim().to_string();
        if payload.is_empty() {
            return None;
        }
        Some(Self {
            payload,
            submit_sequence: submit_sequence.unwrap_or("\r").to_string(),
            submit_delay_ms,
            quiet_period_ms,
            max_wait_ms,
            started_at: now_ms,
            state: InjectState::WaitingForOutput,
        })
    }

    /// Feed PTY output. `now_ms` is a monotonic clock.
    pub fn on_data(&mut self, now_ms: u64) -> Option<PromptInjection> {
        match &self.state {
            InjectState::Injected => None,
            InjectState::WaitingForOutput => {
                self.state = InjectState::Quiet {
                    last_output_at: now_ms,
                };
                None
            }
            InjectState::Quiet { last_output_at } => {
                if now_ms.saturating_sub(*last_output_at) >= self.quiet_period_ms {
                    self.inject()
                } else {
                    // More output while still in the quiet window.
                    self.state = InjectState::Quiet {
                        last_output_at: now_ms,
                    };
                    None
                }
            }
        }
    }

    /// Advance the max-wait clock (call periodically even with no output).
    /// `now_ms` must be from the same clock passed to `new`.
    pub fn on_tick(&mut self, now_ms: u64) -> Option<PromptInjection> {
        if let InjectState::WaitingForOutput | InjectState::Quiet { .. } = &self.state {
            if now_ms.saturating_sub(self.started_at) >= self.max_wait_ms {
                return self.inject();
            }
        }
        None
    }

    /// The PTY exited. Returns the pending injection if it never fired
    /// (owner should warn — the prompt was lost).
    pub fn on_exit(&mut self) -> Option<PromptInjection> {
        match self.state {
            InjectState::Injected => None,
            _ => {
                self.state = InjectState::Injected;
                Some(PromptInjection::PayloadAndSubmit)
            }
        }
    }

    fn inject(&mut self) -> Option<PromptInjection> {
        self.state = InjectState::Injected;
        if self.submit_delay_ms.is_some() {
            Some(PromptInjection::Payload)
        } else {
            Some(PromptInjection::PayloadAndSubmit)
        }
    }

    /// The configured submit delay (E2-06: the launcher owns the delayed
    /// submit when the injector's `Some` branch is active).
    pub fn submit_delay_ms(&self) -> Option<u64> {
        self.submit_delay_ms
    }

    pub fn payload(&self) -> &str {
        &self.payload
    }

    pub fn submit_sequence(&self) -> &str {
        &self.submit_sequence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> CommandContext {
        CommandContext {
            cli: "/usr/local/bin/claude".into(),
            ..Default::default()
        }
    }

    #[test]
    fn claude_fresh_with_prompt_positional() {
        let provider = fartcode_providers::get("claude").unwrap();
        let mut c = ctx();
        c.initial_prompt = Some("Fix the bug".into());
        c.session_id = Some("conv-1".into());
        c.model = "sonnet".into();
        let cmd = build_command(&c, provider);
        assert_eq!(cmd.command, "/usr/local/bin/claude");
        assert_eq!(
            cmd.args,
            vec!["--session-id", "conv-1", "--model", "sonnet", "Fix the bug"]
        );
    }

    #[test]
    fn claude_auto_approve_adds_flag() {
        let provider = fartcode_providers::get("claude").unwrap();
        let mut c = ctx();
        c.auto_approve = true;
        c.initial_prompt = Some("go".into());
        let cmd = build_command(&c, provider);
        assert!(cmd.args.contains(&"--dangerously-skip-permissions".into()));
    }

    #[test]
    fn claude_resume_uses_session_id() {
        let provider = fartcode_providers::get("claude").unwrap();
        let mut c = ctx();
        c.is_resuming = true;
        c.session_id = Some("conv-1".into());
        c.initial_prompt = Some("never sent on resume".into());
        let cmd = build_command(&c, provider);
        assert_eq!(cmd.args, vec!["--resume", "conv-1"]);
    }

    #[test]
    fn amp_receives_prompt_via_stdin_pipe() {
        let provider = fartcode_providers::get("amp").unwrap();
        let mut c = ctx();
        c.cli = "/usr/local/bin/amp".into();
        c.initial_prompt = Some("Do the thing".into());
        c.model = "ultra".into();
        let cmd = build_command(&c, provider);
        assert_eq!(cmd.command, "bash");
        let shell = cmd.args[1].as_str();
        assert!(
            shell.contains("printf '%s\\n' 'Do the thing' | /usr/local/bin/amp"),
            "{shell}"
        );
        assert!(shell.contains("-m ultra"), "{shell}");
    }

    #[test]
    fn codex_resume_without_session_uses_resume_last() {
        let provider = fartcode_providers::get("codex").unwrap();
        let mut c = ctx();
        c.is_resuming = true;
        c.session_id = Some("conv-1".into());
        // No provider-native session id + sessionIdOnResumeOnly → resume --last.
        let cmd = build_command(&c, provider);
        assert_eq!(cmd.args, vec!["resume", "--last"]);
    }

    #[test]
    fn letta_fresh_uses_new_conversation_flag() {
        let provider = fartcode_providers::get("letta").unwrap();
        let mut c = ctx();
        c.session_id = Some("conv-1".into());
        c.initial_prompt = Some("hi".into());
        let cmd = build_command(&c, provider);
        assert!(cmd.args.contains(&"--new".into()), "{:?}", cmd.args);
    }

    #[test]
    fn antigravity_injects_session_id_always() {
        let provider = fartcode_providers::get("antigravity").unwrap();
        let mut c = ctx();
        c.is_resuming = true;
        c.session_id = Some("conv-1".into());
        let cmd = build_command(&c, provider);
        assert!(
            cmd.args.iter().any(|a| a.contains("conv-1")),
            "{:?}",
            cmd.args
        );
    }

    #[test]
    fn kimi_omits_auto_approve_on_resume() {
        let provider = fartcode_providers::get("kimi").unwrap();
        assert_eq!(provider.prompt.auto_approve_flag.as_deref(), Some("--yolo"));
        let mut c = ctx();
        c.auto_approve = true;
        c.is_resuming = true;
        let cmd = build_command(&c, provider);
        assert!(
            !cmd.args.iter().any(|a| a == "--yolo"),
            "kimi must omit --yolo on resume: {:?}",
            cmd.args
        );
        // Fresh session with autoApprove still gets it.
        let mut c = ctx();
        c.auto_approve = true;
        let cmd = build_command(&c, provider);
        assert!(cmd.args.contains(&"--yolo".into()), "{:?}", cmd.args);
    }

    #[test]
    fn spill_writes_and_cleans_temp_file() {
        let tmp = tempfile::tempdir().unwrap();
        let big = "x".repeat(100 * 1024); // 100KB prompt
        let (deliverable, spilled) = maybe_spill_prompt(&big, tmp.path()).unwrap();
        let path = spilled.expect("spilled");
        assert!(path.starts_with(tmp.path().join(".fartCode/prompts")));
        assert_eq!(deliverable, format!("@{}", path.display()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), big);

        // Small prompts don't spill.
        let (deliverable, spilled) = maybe_spill_prompt("short", tmp.path()).unwrap();
        assert_eq!(deliverable, "short");
        assert!(spilled.is_none());

        cleanup_spilled_prompt(&path);
        assert!(!path.exists(), "cleaned on exit");
    }

    #[test]
    fn injector_fires_after_quiet_period_and_reports_lost_prompt_on_exit() {
        // Zero-based test clock: constructor anchors max-wait at t=0.
        let mut inj = PromptInjector::new("Do it", Some("\r"), None, 10, 1000, 0).unwrap();
        assert_eq!(inj.on_data(100), None); // first output
        assert_eq!(inj.on_data(104), None); // still in the quiet window
        assert_eq!(inj.on_data(115), Some(PromptInjection::PayloadAndSubmit));

        // Empty payload → no injector (caller also skips construction on
        // resume — nothing in the injector re-sends on resume).
        assert!(PromptInjector::new("", Some("\r"), None, 10, 1000, 0).is_none());

        // Exit before injection reports the pending prompt (single-shot).
        let mut inj = PromptInjector::new("Lost", Some("\r"), None, 10, 1000, 0).unwrap();
        assert_eq!(inj.on_exit(), Some(PromptInjection::PayloadAndSubmit));
        assert_eq!(inj.on_exit(), None, "single-shot");
    }

    #[test]
    fn injector_split_submit_delay() {
        let mut inj = PromptInjector::new("Do it", Some("\r"), Some(250), 10, 1000, 0).unwrap();
        assert_eq!(inj.on_data(0), None);
        assert_eq!(inj.on_data(20), Some(PromptInjection::Payload));
        // Owner writes the payload, then after 250ms writes the submit.
        assert_eq!(inj.submit_sequence(), "\r");
    }

    #[test]
    fn injector_max_wait_is_relative_to_construction() {
        // A process-lifetime clock (started at, say, 42_000 ms) must not fire
        // immediately: max-wait is elapsed-from-construction, not absolute.
        let mut inj = PromptInjector::new("Go", Some("\r"), None, 10, 500, 42_000).unwrap();
        assert_eq!(inj.on_tick(42_499), None, "not elapsed yet");
        assert_eq!(inj.on_tick(42_500), Some(PromptInjection::PayloadAndSubmit));
        assert_eq!(inj.on_tick(42_501), None, "already injected");
    }

    #[test]
    fn spill_composes_with_build_command_for_argv_providers() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = fartcode_providers::get("claude").unwrap();
        let big = "z".repeat(100 * 1024);
        let (cmd, spilled) = build_command_with_spill(
            &CommandContext {
                cli: "/usr/local/bin/claude".into(),
                initial_prompt: Some(big.clone()),
                ..Default::default()
            },
            provider,
            tmp.path(),
        )
        .unwrap();
        let path = spilled.expect("100KB prompt spills");
        assert!(cmd.args.last().unwrap().starts_with('@'), "{:?}", cmd.args);
        assert_eq!(
            cmd.args.last().unwrap().trim_start_matches('@'),
            path.to_str().unwrap()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), big);
        cleanup_spilled_prompt(&path);
        assert!(!path.exists());

        // Stdin-pipe providers: a long prompt must NOT spill — the raw
        // prompt is piped to stdin (assert the shell line carries it), and
        // no @path placeholder is ever passed.
        let amp = fartcode_providers::get("amp").unwrap();
        let big_prompt = "x".repeat(100 * 1024);
        let (cmd, spilled) = build_command_with_spill(
            &CommandContext {
                cli: "/usr/local/bin/amp".into(),
                initial_prompt: Some(big_prompt.clone()),
                ..Default::default()
            },
            amp,
            tmp.path(),
        )
        .unwrap();
        assert!(spilled.is_none(), "stdin-pipe payloads never spill");
        assert!(
            cmd.args[1].contains(&big_prompt),
            "raw prompt must reach stdin: {} bytes",
            cmd.args[1].len()
        );
    }

    #[test]
    fn quote_shell_arg_handles_metacharacters() {
        assert_eq!(quote_shell_arg("simple"), "simple");
        assert_eq!(quote_shell_arg(""), "''");
        assert_eq!(quote_shell_arg("it's"), "'it'\\''s'");
        assert_eq!(quote_shell_arg("a|b$c"), "'a|b$c'");
        assert_eq!(quote_shell_arg("say \"hi\""), "'say \"hi\"'");
    }

    #[test]
    fn auto_approve_resolver_combinations() {
        // Trust gating: default autoTrustWorktrees=true → implicitly on,
        // regardless of the conversation toggle or the by-default setting.
        assert!(resolve_auto_approve(false, false, true));
        // Explicit conversation auto-approve.
        assert!(resolve_auto_approve(true, false, false));
        // Forced by the task setting.
        assert!(resolve_auto_approve(false, true, false));
        // Nothing on → off.
        assert!(!resolve_auto_approve(false, false, false));
    }

    #[test]
    fn auto_approve_flag_is_capability_gated() {
        // A provider WITHOUT the autoApprove capability never gets the flag
        // even when ctx.auto_approve is set.
        let mut spec = synthetic_spec();
        spec.auto_approve_flag = Some("--yolo".into());
        let mut caps = synthetic_capabilities();
        caps.auto_approve = false;
        let provider = synthetic_provider(spec, caps);
        let mut c = ctx();
        c.auto_approve = true;
        c.initial_prompt = Some("go".into());
        let cmd = build_command(&c, &provider);
        assert!(!cmd.args.contains(&"--yolo".into()), "{:?}", cmd.args);

        // With the capability, the flag lands.
        let mut spec = synthetic_spec();
        spec.auto_approve_flag = Some("--yolo".into());
        let mut caps = synthetic_capabilities();
        caps.auto_approve = true;
        let provider = synthetic_provider(spec, caps);
        let cmd = build_command(&c, &provider);
        assert!(cmd.args.contains(&"--yolo".into()), "{:?}", cmd.args);
    }

    #[test]
    fn auto_approve_env_is_merged_when_capability_gated() {
        // mimocode/opencode gate auto-approve via env, not argv.
        let mut spec = synthetic_spec();
        spec.auto_approve_flag = None;
        spec.auto_approve_env = Some(vec![(
            "MIMOCODE_PERMISSION".into(),
            "{\"*\":\"allow\"}".into(),
        )]);
        let provider = synthetic_provider(spec, synthetic_capabilities());
        let mut c = ctx();
        c.auto_approve = true;
        c.initial_prompt = Some("go".into());
        let cmd = build_command(&c, &provider);
        assert_eq!(
            cmd.env.get("MIMOCODE_PERMISSION"),
            Some(&"{\"*\":\"allow\"}".to_string()),
            "{:?}",
            cmd.env
        );

        // Without the capability, the env is NOT merged.
        let mut spec = synthetic_spec();
        spec.auto_approve_flag = None;
        spec.auto_approve_env = Some(vec![(
            "MIMOCODE_PERMISSION".into(),
            "{\"*\":\"allow\"}".into(),
        )]);
        let mut caps = synthetic_capabilities();
        caps.auto_approve = false;
        let provider = synthetic_provider(spec, caps);
        let cmd = build_command(&c, &provider);
        assert!(!cmd.env.contains_key("MIMOCODE_PERMISSION"));
    }

    #[test]
    fn real_provider_auto_approve_mechanism_is_never_empty() {
        // Every provider that declares the autoApprove capability must have
        // exactly one mechanism (argv flag XOR env) — otherwise auto-approve
        // silently drops for it.
        for p in fartcode_providers::list() {
            if p.capabilities.auto_approve {
                let has_flag = p.prompt.auto_approve_flag.is_some();
                let has_env = p.prompt.auto_approve_env.is_some();
                assert!(
                    has_flag ^ has_env,
                    "{}: autoApprove capability needs exactly one of flag/env (flag={has_flag} env={has_env})",
                    p.id
                );
            }
        }
    }

    #[test]
    fn deduplicate_flags_keeps_first_occurrence() {
        // Synthetic spec: codex's dedup list never collides with its
        // autoApproveFlag today, so exercise the dedupe path directly.
        let mut spec = synthetic_spec();
        spec.deduplicate_flags = vec!["--dangerously-bypass-approvals-and-sandbox".into()];
        spec.default_args = vec!["--dangerously-bypass-approvals-and-sandbox".into()];
        let provider = synthetic_provider(spec, synthetic_capabilities());
        let mut c = ctx();
        c.auto_approve = true;
        c.initial_prompt = Some("go".into());
        let cmd = build_command(&c, &provider);
        assert_eq!(
            cmd.args
                .iter()
                .filter(|a| *a == "--dangerously-bypass-approvals-and-sandbox")
                .count(),
            1,
            "deduped to one occurrence: {:?}",
            cmd.args
        );
    }

    fn synthetic_spec() -> fartcode_providers::types::PromptDescriptor {
        fartcode_providers::types::PromptDescriptor {
            strategy: fartcode_providers::PromptStrategy::Argv {
                flag: Some(String::new()),
            },
            auto_approve_flag: Some("--dangerously-bypass-approvals-and-sandbox".into()),
            auto_approve_env: None,
            initial_prompt_flag: Some(String::new()),
            resume_flag: None,
            session_id_flag: None,
            session_id_on_resume_only: false,
            resume_without_session_flag: None,
            model_flag: None,
            new_conversation_flag: None,
            session_id_always: false,
            omit_auto_approve_on_resume: false,
            initial_prompt_via_stdin_pipe: false,
            deduplicate_flags: Vec::new(),
            submit_sequence: None,
            submit_delay_ms: None,
            default_args: Vec::new(),
        }
    }

    fn synthetic_capabilities() -> fartcode_providers::types::Capabilities {
        fartcode_providers::types::Capabilities {
            acp: false,
            auth: false,
            auto_approve: true,
            effort: false,
            hooks: false,
            host_dependency: false,
            mcp: false,
            models: false,
            plugins: false,
            prompt: true,
            sessions: false,
            trust: false,
        }
    }

    fn synthetic_provider(
        spec: fartcode_providers::types::PromptDescriptor,
        caps: fartcode_providers::types::Capabilities,
    ) -> ProviderDescriptor {
        ProviderDescriptor {
            id: "synthetic".into(),
            name: "Synthetic".into(),
            description: String::new(),
            website_url: None,
            capabilities: caps,
            prompt: spec,
            binaries: vec!["synthetic".into()],
            default_model: None,
            env_vars: Vec::new(),
            auth_methods: Vec::new(),
        }
    }
}

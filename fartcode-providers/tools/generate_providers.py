#!/usr/bin/env python3
import json, glob

def s(v):
    """JSON-escape a string for a Rust string literal (JSON escaping is compatible)."""
    return json.dumps(v) if v is not None else "None"

def opt_str(v):
    if v is None: return "None"
    return f'Some({s(v)}.to_string())'

def cap_bool(v):  # 'supported'/'selectable'/'resumable'/'config'/'plugin' -> true
    return str(v not in ("none", None)).lower()

def cap_flag(v):  # prompt 'argv' etc
    return str(v != "none").lower()

def prompt_arm(p):
    kind = p["kind"]
    flag = opt_str(p["flag"]) if kind == "argv" else None
    if kind == "stdin-pipe":
        return f'PromptStrategy::StdinPipe'
    if kind == "keystroke":
        return f'PromptStrategy::Keystroke'
    return f'PromptStrategy::Argv {{ flag: {flag} }}'

def opt_usize(v):
    return "None" if v is None else f"Some({int(v)})"

def env_str(v):  # {"K": "V", ...} -> Some(vec![("K".to_string(), "V".to_string())]) | None
    if not v:
        return "None"
    pairs = ", ".join(f"({s(k)}.to_string(), {s(vv)}.to_string())" for k, vv in v.items())
    return f'Some(vec![{pairs}])'

def prompt_desc(p):
    return (
        "PromptDescriptor {\n"
        f"            strategy: {prompt_arm(p)},\n"
        f"            auto_approve_flag: {opt_str(p['autoApproveFlag'])},\n"
        f"            auto_approve_env: {env_str(p.get('autoApproveEnv'))},\n"
        f"            initial_prompt_flag: {opt_str(p['initialPromptFlag'])},\n"
        f"            resume_flag: {opt_str(p['resumeFlag'])},\n"
        f"            session_id_flag: {opt_str(p['sessionIdFlag'])},\n"
        f"            session_id_on_resume_only: {str(bool(p['sessionIdOnResumeOnly'])).lower()},\n"
        f"            resume_without_session_flag: {opt_str(p['resumeWithoutSessionFlag'])},\n"
        f"            model_flag: {opt_str(p['modelFlag'])},\n"
        f"            new_conversation_flag: {opt_str(p.get('newConversationFlag'))},\n"
        f"            session_id_always: {str(bool(p.get('sessionIdAlways', False))).lower()},\n"
        f"            omit_auto_approve_on_resume: {str(bool(p.get('omitAutoApproveOnResume', False))).lower()},\n"
        f"            initial_prompt_via_stdin_pipe: {str(bool(p.get('initialPromptViaStdinPipe', False))).lower()},\n"
        f"            deduplicate_flags: {vec_str(p.get('deduplicateFlags') or [])},\n"
        f"            submit_sequence: {opt_str(p.get('submitSequence'))},\n"
        f"            submit_delay_ms: {opt_usize(p.get('submitDelayMs'))},\n"
        f"            default_args: vec![{', '.join(s(a) + '.to_string()' for a in p['defaultArgs'])}],\n"
        "        }"
    )

def vec_str(v):
    return "vec![" + ", ".join(f"{s(x)}.to_string()" for x in v) + "]"

entries = []
for f in sorted(glob.glob('fartcode-providers/extracted/t*/*.json')):
    d = json.load(open(f))
    c = d["capabilities"]
    cap = (
        "Capabilities {\n"
        f"            acp: {cap_bool(c.get('acp'))},\n"
        f"            auth: {cap_bool(c.get('auth'))},\n"
        f"            auto_approve: {cap_bool(c.get('autoApprove'))},\n"
        f"            effort: {cap_bool(c.get('effort'))},\n"
        f"            hooks: {cap_bool(c.get('hooks'))},\n"
        f"            host_dependency: {cap_bool(c.get('hostDependency'))},\n"
        f"            mcp: {cap_bool(c.get('mcp'))},\n"
        f"            models: {cap_bool(c.get('models'))},\n"
        f"            plugins: {cap_bool(c.get('plugins'))},\n"
        f"            prompt: {cap_flag(c.get('prompt'))},\n"
        f"            sessions: {cap_bool(c.get('sessions'))},\n"
        f"            trust: {cap_bool(c.get('trust'))},\n"
        "        }"
    )
    entry = (
        "ProviderDescriptor {\n"
        f"        id: {s(d['id'])}.to_string(),\n"
        f"        name: {s(d['name'])}.to_string(),\n"
        f"        description: {s(d['description'])}.to_string(),\n"
        f"        website_url: {opt_str(d['websiteUrl'])},\n"
        f"        capabilities: {cap},\n"
        f"        prompt: {prompt_desc(d['prompt'])},\n"
        f"        binaries: {vec_str(d['binaries'])},\n"
        f"        default_model: {opt_str(d['defaultModel'])},\n"
        f"        env_vars: {vec_str(d['envVars'])},\n"
        "    }"
    )
    entries.append(entry)

header = """//! GENERATED from `fartcode-providers/extracted/` (E3-01 registry data extraction).
//! Source of truth: the reference `packages/plugins/src/agents/impl/*/index.ts`.
//! To add a provider: add its descriptor here (one entry) — see `registry.rs`.

use std::sync::LazyLock;

use crate::types::{Capabilities, PromptDescriptor, PromptStrategy, ProviderDescriptor};

pub static PROVIDERS: LazyLock<Vec<ProviderDescriptor>> = LazyLock::new(|| vec![
"""
footer = "]);\n"
open('fartcode-providers/src/providers_data.rs', 'w').write(header + ",\n".join("    " + e for e in entries) + "\n" + footer)
print("wrote providers_data.rs with", len(entries), "entries")

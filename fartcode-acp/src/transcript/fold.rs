//! Item-level fold: normalized events → transcript items (ticket E2-11-4).
//!
//! Direct port of the reference `reducer/item-fold.ts`: message/thinking
//! chunk merging, tool-call lifecycle with status mapping, diff-driven file
//! operation items, the per-turn plan marker, read-batch grouping, nested
//! tool trees, and turn finalization.
//!
//! Deviation: the reference's enrich-hook event kinds (`subagent`, `search`,
//! `mcp_tool`, `web_fetch`, `subagent_update`) have no baseline producer, so
//! [`FoldEvent`] omits them and `upsertSpecialEvent` is not ported.

use std::collections::HashMap;

use crate::transcript::ids::{
    make_diff_id, make_message_id, make_plan_id, make_thinking_id, make_tool_group_id, make_tool_id,
};
use crate::transcript::models::{
    PlanEntryInput, PlanEntryStatus, Role, ThinkingStatus, ToolCallItem, ToolGroup, ToolItemKind,
    ToolNode, ToolStatus, TranscriptItem, TranscriptMessage, TranscriptThinking, SESSION_PLAN_ID,
};
use crate::transcript::normalize::{MessageRole, NormalizedToolStatus};

/// Events that reach the fold (reference `FoldEvent`) — message/thinking
/// carry a guaranteed message id after segmentation.
#[derive(Debug, Clone, PartialEq)]
pub enum FoldEvent {
    Message {
        role: MessageRole,
        message_id: String,
        text: String,
    },
    Thinking {
        message_id: String,
        text: String,
    },
    ToolCall {
        tool_call_id: String,
        title: String,
        tool_kind: Option<String>,
        status: Option<NormalizedToolStatus>,
        parent_tool_call_id: Option<String>,
        diffs: Vec<crate::transcript::normalize::NormalizedDiff>,
        input_summary: Option<String>,
        output_text: Option<String>,
        terminal_id: Option<String>,
    },
    ToolUpdate {
        tool_call_id: String,
        title: Option<String>,
        tool_kind: Option<String>,
        status: Option<NormalizedToolStatus>,
        parent_tool_call_id: Option<String>,
        diffs: Vec<crate::transcript::normalize::NormalizedDiff>,
        output_text: Option<String>,
        terminal_id: Option<String>,
    },
    Plan {
        entries: Vec<PlanEntryInput>,
    },
}

// -- status / kind helpers ---------------------------------------------------

pub fn map_tool_status(status: Option<NormalizedToolStatus>) -> Option<ToolStatus> {
    match status {
        Some(NormalizedToolStatus::Pending) | Some(NormalizedToolStatus::InProgress) => {
            Some(ToolStatus::Running)
        }
        Some(NormalizedToolStatus::Completed) => Some(ToolStatus::Done),
        Some(NormalizedToolStatus::Failed) => Some(ToolStatus::Error),
        None => None,
    }
}

fn is_subagent_kind(kind: Option<&str>) -> bool {
    matches!(kind, Some("subagent") | Some("task") | Some("agent"))
}

fn is_search_kind(kind: Option<&str>) -> bool {
    matches!(kind, Some("search") | Some("grep"))
}

fn search_query_from_title(title: &str) -> String {
    let lower = title.to_lowercase();
    if let Some(rest) = lower.strip_prefix("search ") {
        let start = title.len() - rest.len();
        title[start..].to_string()
    } else {
        title.to_string()
    }
}

fn is_read_kind(kind: Option<&str>, title: Option<&str>) -> bool {
    matches!(kind, Some("read") | Some("read_file"))
        || title.is_some_and(|t| t.starts_with("Read "))
}

fn is_edit_kind(kind: Option<&str>) -> bool {
    matches!(kind, Some("edit") | Some("write") | Some("apply_patch"))
}

fn is_execute_kind(kind: Option<&str>) -> bool {
    matches!(kind, Some("execute") | Some("terminal") | Some("bash"))
}

fn is_mcp_tool_kind(kind: Option<&str>) -> bool {
    matches!(kind, Some("mcp-tool") | Some("mcp_tool"))
}

fn is_web_fetch_kind(kind: Option<&str>) -> bool {
    matches!(kind, Some("web-fetch") | Some("web_fetch") | Some("fetch"))
}

fn infer_read_path(title: &str) -> Option<String> {
    let rest = title.strip_prefix("Read ")?;
    let end = rest.find(" (").unwrap_or(rest.len());
    let path = rest[..end].trim();
    (!path.is_empty()).then(|| path.to_string())
}

// -- structural helpers ------------------------------------------------------

fn strip_children(item: &ToolCallItem) -> ToolCallItem {
    let mut tool = item.clone();
    if tool.children.as_ref().is_some_and(|c| !c.is_empty()) {
        tool.children = None;
    }
    tool
}

/// Flatten nested tool trees into seq-sorted flat items (reference
/// `flattenItems`).
pub fn flatten_items(items: &[TranscriptItem]) -> Vec<TranscriptItem> {
    let mut flat: Vec<TranscriptItem> = Vec::new();
    fn visit(item: &TranscriptItem, flat: &mut Vec<TranscriptItem>) {
        match item {
            TranscriptItem::Group(group) => {
                for child in &group.children {
                    match child {
                        ToolNode::Tool(t) => flat.push(TranscriptItem::Tool(strip_children(t))),
                        ToolNode::Group(g) => {
                            for gc in &g.children {
                                match gc {
                                    ToolNode::Tool(t) => {
                                        flat.push(TranscriptItem::Tool(strip_children(t)))
                                    }
                                    ToolNode::Group(g2) => {
                                        visit(&TranscriptItem::Group(g2.clone()), flat)
                                    }
                                }
                            }
                        }
                    }
                }
            }
            TranscriptItem::Tool(t) => {
                flat.push(TranscriptItem::Tool(strip_children(t)));
                for child in t.children.iter().flatten() {
                    match child {
                        ToolNode::Tool(ct) => flat.push(TranscriptItem::Tool(strip_children(ct))),
                        ToolNode::Group(g) => visit(&TranscriptItem::Group(g.clone()), flat),
                    }
                }
            }
            other => flat.push(other.clone()),
        }
    }
    for item in items {
        visit(item, &mut flat);
    }
    flat.sort_by_key(|i| i.seq());
    flat
}

fn next_seq(items: &[TranscriptItem]) -> u64 {
    items.iter().map(|i| i.seq()).max().map_or(0, |m| m + 1)
}

/// Auto-finalize open thinking rows when non-thinking content arrives.
fn finalize_open_thinking(items: Vec<TranscriptItem>, now: u64) -> Vec<TranscriptItem> {
    let result: Vec<TranscriptItem> = items
        .into_iter()
        .map(|item| match item {
            TranscriptItem::Thinking(t) if t.status == ThinkingStatus::Thinking => {
                TranscriptItem::Thinking(TranscriptThinking {
                    status: ThinkingStatus::Done,
                    duration_ms: Some(now.saturating_sub(t.started_at)),
                    ..t
                })
            }
            other => other,
        })
        .collect();
    result
}

// -- tool-call construction --------------------------------------------------

#[derive(Debug)]
pub struct ToolParams {
    pub id: String,
    pub seq: u64,
    pub tool_call_id: String,
    pub title: String,
    pub tool_kind: Option<String>,
    pub status: Option<NormalizedToolStatus>,
    pub parent_tool_call_id: Option<String>,
    pub input_summary: Option<String>,
    pub output_text: Option<String>,
    pub terminal_id: Option<String>,
}

/// Build the kind-specific tool item (reference `createToolCallItem`).
pub fn create_tool_call_item(params: ToolParams) -> ToolCallItem {
    let status = map_tool_status(params.status).unwrap_or(ToolStatus::Running);
    let kind_str = params.tool_kind.as_deref();
    let mut base = ToolCallItem::base(
        ToolItemKind::UnknownToolCall, // replaced below
        params.id,
        params.seq,
        params.tool_call_id,
        params.title.clone(),
        status,
        params.parent_tool_call_id,
    );
    base.input_summary = params.input_summary;

    if is_subagent_kind(kind_str) {
        let mut tool = base;
        tool.kind = ToolItemKind::SpawnSubagentToolCall;
        tool.name = Some(params.title);
        return tool;
    }
    if is_search_kind(kind_str) {
        let mut tool = base;
        tool.kind = ToolItemKind::SearchToolCall;
        tool.query = Some(search_query_from_title(&params.title));
        return tool;
    }
    if is_mcp_tool_kind(kind_str) {
        let mut tool = base;
        tool.kind = ToolItemKind::McpToolCall;
        tool.tool = Some(params.title);
        return tool;
    }
    if is_web_fetch_kind(kind_str) {
        let mut tool = base;
        tool.kind = ToolItemKind::WebFetchToolCall;
        tool.url = Some(params.title);
        return tool;
    }
    if is_read_kind(kind_str, Some(&params.title)) {
        let mut tool = base;
        tool.kind = ToolItemKind::ReadToolCall;
        tool.path = infer_read_path(&params.title);
        return tool;
    }
    if is_execute_kind(kind_str) {
        let mut tool = base;
        tool.kind = ToolItemKind::ExecuteToolCall;
        tool.command = Some(params.title);
        tool.output_text = params.output_text;
        tool.terminal_id = params.terminal_id;
        return tool;
    }
    let mut tool = base;
    tool.kind = ToolItemKind::UnknownToolCall;
    tool.tool_kind = params.tool_kind;
    tool.name = Some(params.title);
    tool
}

/// Apply a `tool_call_update` to an existing item (reference
/// `updateToolCallItem`).
fn update_tool_call_item(
    item: &ToolCallItem,
    title: Option<&str>,
    status: Option<NormalizedToolStatus>,
    output_text: Option<String>,
    terminal_id: Option<String>,
) -> ToolCallItem {
    let mapped = map_tool_status(status);
    let next_title = title
        .map(str::to_string)
        .unwrap_or_else(|| item.title.clone());
    let mut tool = item.clone();
    if let Some(mapped) = mapped {
        tool.status = mapped;
    }
    if let Some(title) = title {
        tool.title = title.to_string();
    }
    match tool.kind {
        ToolItemKind::ExecuteToolCall => {
            if title.is_some() {
                tool.command = Some(next_title);
            }
            if let Some(out) = output_text {
                tool.output_text = Some(out);
            }
            if let Some(id) = terminal_id {
                tool.terminal_id = Some(id);
            }
        }
        ToolItemKind::ReadToolCall => {
            if title.is_some() {
                tool.path = infer_read_path(&next_title);
            }
        }
        ToolItemKind::SearchToolCall => {
            if title.is_some() {
                tool.query = Some(search_query_from_title(&next_title));
            }
        }
        ToolItemKind::McpToolCall => {
            if title.is_some() {
                tool.tool = Some(next_title);
            }
        }
        ToolItemKind::WebFetchToolCall => {
            if title.is_some() {
                tool.page_title = Some(next_title);
            }
        }
        ToolItemKind::SpawnSubagentToolCall | ToolItemKind::UnknownToolCall => {
            tool.name = title.is_some().then_some(next_title).or(tool.name);
        }
        _ => {}
    }
    tool
}

fn upsert_tool_call_item(items: Vec<TranscriptItem>, next: ToolCallItem) -> Vec<TranscriptItem> {
    let id = next.id.clone();
    let idx = items
        .iter()
        .position(|item| matches!(item, TranscriptItem::Tool(t) if t.id == id));
    match idx {
        Some(i) => {
            let mut result = items;
            result[i] = TranscriptItem::Tool(next);
            result
        }
        None => {
            let mut result = items;
            result.push(TranscriptItem::Tool(next));
            result
        }
    }
}

// -- file operations -----------------------------------------------------------

fn upsert_file_operations(
    items: Vec<TranscriptItem>,
    tool_id: &str,
    tool_call_id: &str,
    title: &str,
    parent_tool_call_id: Option<String>,
    diffs: &[crate::transcript::normalize::NormalizedDiff],
    status: Option<NormalizedToolStatus>,
) -> Vec<TranscriptItem> {
    let mut result = items;
    for diff in diffs {
        let id = make_diff_id(tool_id, &diff.path);
        let mapped = map_tool_status(status);
        let idx = result
            .iter()
            .position(|item| matches!(item, TranscriptItem::Tool(t) if t.id == id));
        match idx {
            Some(i) => {
                let existing = match &result[i] {
                    TranscriptItem::Tool(t) => t.clone(),
                    _ => unreachable!(),
                };
                let mut updated = existing;
                updated.title = title.to_string();
                updated.tool_call_id = tool_call_id.to_string();
                updated.path = Some(diff.path.clone());
                updated.parent_tool_call_id = parent_tool_call_id.clone();
                if let Some(mapped) = mapped {
                    updated.status = mapped;
                }
                if diff.old_text.is_none() {
                    updated.kind = ToolItemKind::CreateFileToolCall;
                    updated.content = Some(diff.new_text.clone());
                    updated.old_text = None;
                    updated.new_text = None;
                } else {
                    updated.kind = ToolItemKind::ModifyFileToolCall;
                    updated.content = None;
                    updated.old_text = diff.old_text.clone();
                    updated.new_text = Some(diff.new_text.clone());
                }
                result[i] = TranscriptItem::Tool(updated);
            }
            None => {
                let mut tool = ToolCallItem::base(
                    if diff.old_text.is_none() {
                        ToolItemKind::CreateFileToolCall
                    } else {
                        ToolItemKind::ModifyFileToolCall
                    },
                    id,
                    next_seq(&result),
                    tool_call_id.to_string(),
                    title.to_string(),
                    mapped.unwrap_or(ToolStatus::Running),
                    parent_tool_call_id.clone(),
                );
                tool.path = Some(diff.path.clone());
                if diff.old_text.is_none() {
                    tool.content = Some(diff.new_text.clone());
                } else {
                    tool.old_text = diff.old_text.clone();
                    tool.new_text = Some(diff.new_text.clone());
                }
                result.push(TranscriptItem::Tool(tool));
            }
        }
    }
    result
}

fn update_file_operation_statuses(
    items: Vec<TranscriptItem>,
    tool_call_id: &str,
    status: Option<NormalizedToolStatus>,
) -> Vec<TranscriptItem> {
    let Some(mapped) = map_tool_status(status) else {
        return items;
    };
    items
        .into_iter()
        .map(|item| match &item {
            TranscriptItem::Tool(t)
                if matches!(
                    t.kind,
                    ToolItemKind::CreateFileToolCall
                        | ToolItemKind::ModifyFileToolCall
                        | ToolItemKind::DeleteFileToolCall
                ) && t.tool_call_id == tool_call_id =>
            {
                let mut tool = t.clone();
                tool.status = mapped;
                TranscriptItem::Tool(tool)
            }
            _ => item,
        })
        .collect()
}

fn has_file_operations(items: &[TranscriptItem], tool_call_id: &str) -> bool {
    items.iter().any(|item| match item {
        TranscriptItem::Tool(t) => {
            matches!(
                t.kind,
                ToolItemKind::CreateFileToolCall
                    | ToolItemKind::ModifyFileToolCall
                    | ToolItemKind::DeleteFileToolCall
            ) && t.tool_call_id == tool_call_id
        }
        _ => false,
    })
}

// -- plan marker ---------------------------------------------------------------

fn plan_status(entries: &[PlanEntryInput]) -> ToolStatus {
    if entries
        .iter()
        .any(|e| e.status == PlanEntryStatus::InProgress)
    {
        ToolStatus::Running
    } else {
        ToolStatus::Done
    }
}

fn upsert_plan_tool_call(
    items: Vec<TranscriptItem>,
    turn_id: &str,
    entries: &[PlanEntryInput],
) -> Vec<TranscriptItem> {
    let id = make_plan_id(turn_id);
    let idx = items.iter().position(|item| {
        matches!(item, TranscriptItem::Tool(t) if t.kind == ToolItemKind::CreatePlanToolCall && t.id == id)
    });
    let status = plan_status(entries);
    match idx {
        Some(i) => {
            let mut result = items;
            if let TranscriptItem::Tool(existing) = &mut result[i] {
                existing.status = status;
                existing.title = "Plan updated".to_string();
                existing.plan_id = Some(SESSION_PLAN_ID.to_string());
            }
            result
        }
        None => {
            let mut tool = ToolCallItem::base(
                ToolItemKind::CreatePlanToolCall,
                id.clone(),
                next_seq(&items),
                id,
                "Plan updated".to_string(),
                status,
                None,
            );
            tool.plan_id = Some(SESSION_PLAN_ID.to_string());
            let mut result = items;
            result.push(TranscriptItem::Tool(tool));
            result
        }
    }
}

// -- tree building ---------------------------------------------------------------

fn read_group_status(children: &[ToolNode]) -> ToolStatus {
    if children.iter().any(|c| c.status() == ToolStatus::Running) {
        ToolStatus::Running
    } else if children.iter().any(|c| c.status() == ToolStatus::Error) {
        ToolStatus::Error
    } else {
        ToolStatus::Done
    }
}

fn wrap_read_groups_nodes(items: Vec<ToolNode>) -> Vec<ToolNode> {
    let mut result: Vec<ToolNode> = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let is_read =
            matches!(&items[i], ToolNode::Tool(t) if t.kind == ToolItemKind::ReadToolCall);
        if !is_read {
            result.push(items[i].clone());
            i += 1;
            continue;
        }
        let mut run: Vec<ToolNode> = vec![items[i].clone()];
        let mut j = i + 1;
        while j < items.len()
            && matches!(&items[j], ToolNode::Tool(t) if t.kind == ToolItemKind::ReadToolCall)
        {
            run.push(items[j].clone());
            j += 1;
        }
        if run.len() > 1 {
            let first = match &run[0] {
                ToolNode::Tool(t) => t.clone(),
                _ => unreachable!(),
            };
            let group = ToolGroup::read_batch(
                make_tool_group_id(&first.id),
                first.seq,
                format!("{} file reads", run.len()),
                read_group_status(&run),
                run,
            );
            result.push(ToolNode::Group(group));
        } else {
            result.push(run.pop().unwrap());
        }
        i = j;
    }
    result
}

fn wrap_read_groups_items(items: Vec<TranscriptItem>) -> Vec<TranscriptItem> {
    let mut result: Vec<TranscriptItem> = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let is_read =
            matches!(&items[i], TranscriptItem::Tool(t) if t.kind == ToolItemKind::ReadToolCall);
        if !is_read {
            result.push(items[i].clone());
            i += 1;
            continue;
        }
        let mut run: Vec<ToolNode> = Vec::new();
        let mut j = i;
        while j < items.len() {
            match &items[j] {
                TranscriptItem::Tool(t) if t.kind == ToolItemKind::ReadToolCall => {
                    run.push(ToolNode::Tool(t.clone()));
                    j += 1;
                }
                _ => break,
            }
        }
        if run.len() > 1 {
            let first = match &run[0] {
                ToolNode::Tool(t) => t.clone(),
                _ => unreachable!(),
            };
            let group = ToolGroup::read_batch(
                make_tool_group_id(&first.id),
                first.seq,
                format!("{} file reads", run.len()),
                read_group_status(&run),
                run,
            );
            result.push(TranscriptItem::Group(group));
        } else {
            result.push(items[i].clone());
        }
        i = j;
    }
    result
}

/// Rebuild the nested tool tree from flat items (reference `buildTree`).
fn build_tree(flat_items: Vec<TranscriptItem>, turn_id: &str) -> Vec<TranscriptItem> {
    let mut tool_by_id: HashMap<String, ToolCallItem> = HashMap::new();
    let mut children_by_parent: HashMap<String, Vec<ToolCallItem>> = HashMap::new();
    let mut top_level: Vec<TranscriptItem> = Vec::new();

    for item in &flat_items {
        if let TranscriptItem::Tool(t) = item {
            tool_by_id.insert(t.id.clone(), strip_children(t));
        }
    }

    for item in flat_items {
        let TranscriptItem::Tool(tool) = item else {
            top_level.push(item);
            continue;
        };
        let parent_item_id = tool
            .parent_tool_call_id
            .as_deref()
            .map(|p| make_tool_id(turn_id, p));
        match parent_item_id {
            Some(pid) if tool_by_id.contains_key(&pid) => {
                children_by_parent
                    .entry(pid)
                    .or_default()
                    .push(strip_children(&tool));
            }
            _ => top_level.push(TranscriptItem::Tool(strip_children(&tool))),
        }
    }

    fn attach_children(
        tool: ToolCallItem,
        children_by_parent: &HashMap<String, Vec<ToolCallItem>>,
    ) -> ToolCallItem {
        let raw_children = children_by_parent.get(&tool.id);
        let Some(raw_children) = raw_children.filter(|c| !c.is_empty()) else {
            return strip_children(&tool);
        };
        let mut children: Vec<ToolNode> = raw_children
            .iter()
            .map(|c| ToolNode::Tool(attach_children(c.clone(), children_by_parent)))
            .collect();
        children.sort_by_key(|c| match c {
            ToolNode::Tool(t) => t.seq,
            ToolNode::Group(g) => g.seq,
        });
        let children = wrap_read_groups_nodes(children);
        let mut tool = strip_children(&tool);
        tool.children = Some(children);
        tool
    }

    let mut attached: Vec<TranscriptItem> = top_level
        .into_iter()
        .map(|item| match item {
            TranscriptItem::Tool(t) => {
                TranscriptItem::Tool(attach_children(t, &children_by_parent))
            }
            other => other,
        })
        .collect();
    attached.sort_by_key(|i| i.seq());
    wrap_read_groups_items(attached)
}

fn normalize_tool_structure(items: Vec<TranscriptItem>, turn_id: &str) -> Vec<TranscriptItem> {
    build_tree(flatten_items(&items), turn_id)
}

// -- the fold ------------------------------------------------------------------

/// Apply one fold event to a turn's item list, returning the updated list.
/// `turn_id` scopes all synthesized ids (reference `foldItem`).
pub fn fold_item(
    items: &[TranscriptItem],
    event: &FoldEvent,
    turn_id: &str,
    at: u64,
) -> Vec<TranscriptItem> {
    let flat_items = flatten_items(items);
    match event {
        FoldEvent::Message {
            role,
            message_id,
            text,
        } => {
            let id = make_message_id(
                turn_id,
                message_id,
                match role {
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                },
            );
            let base = finalize_open_thinking(flat_items, at);
            let idx = base
                .iter()
                .position(|item| matches!(item, TranscriptItem::Message(m) if m.id == id));
            match idx {
                Some(i) => {
                    let mut result = base;
                    if let TranscriptItem::Message(msg) = &mut result[i] {
                        msg.text.push_str(text);
                    }
                    normalize_tool_structure(result, turn_id)
                }
                None => {
                    let msg = TranscriptMessage::new(
                        id,
                        next_seq(&base),
                        match role {
                            MessageRole::User => Role::User,
                            MessageRole::Assistant => Role::Assistant,
                        },
                        text.clone(),
                    );
                    let mut result = base;
                    result.push(TranscriptItem::Message(msg));
                    normalize_tool_structure(result, turn_id)
                }
            }
        }

        FoldEvent::Thinking { message_id, text } => {
            let id = make_thinking_id(turn_id, message_id);
            let idx = flat_items.iter().position(|item| {
                matches!(item, TranscriptItem::Thinking(t) if t.id == id && t.status == ThinkingStatus::Thinking)
            });
            match idx {
                Some(i) => {
                    let mut result = flat_items;
                    if let TranscriptItem::Thinking(t) = &mut result[i] {
                        t.text.push_str(text);
                    }
                    normalize_tool_structure(result, turn_id)
                }
                None => {
                    let thinking = TranscriptThinking::new(
                        id,
                        next_seq(&flat_items),
                        message_id.clone(),
                        text.clone(),
                        at,
                    );
                    let base = finalize_open_thinking(flat_items, at);
                    let mut result = base;
                    result.push(TranscriptItem::Thinking(thinking));
                    normalize_tool_structure(result, turn_id)
                }
            }
        }

        FoldEvent::ToolCall {
            tool_call_id,
            title,
            tool_kind,
            status,
            parent_tool_call_id,
            diffs,
            input_summary,
            output_text,
            terminal_id,
        } => {
            let tool_id = make_tool_id(turn_id, tool_call_id);
            let base = finalize_open_thinking(flat_items, at);
            if !diffs.is_empty() {
                let next = upsert_file_operations(
                    base,
                    &tool_id,
                    tool_call_id,
                    title,
                    parent_tool_call_id.clone(),
                    diffs,
                    *status,
                );
                return normalize_tool_structure(next, turn_id);
            }
            if is_edit_kind(tool_kind.as_deref()) {
                // Edit-kind calls without diffs carry no renderable content —
                // the diffs are the content (reference behavior).
                return normalize_tool_structure(base, turn_id);
            }
            let tool = create_tool_call_item(ToolParams {
                id: tool_id,
                seq: next_seq(&base),
                tool_call_id: tool_call_id.clone(),
                title: title.clone(),
                tool_kind: tool_kind.clone(),
                status: *status,
                parent_tool_call_id: parent_tool_call_id.clone(),
                input_summary: input_summary.clone(),
                output_text: output_text.clone(),
                terminal_id: terminal_id.clone(),
            });
            let next = upsert_tool_call_item(base, tool);
            normalize_tool_structure(next, turn_id)
        }

        FoldEvent::ToolUpdate {
            tool_call_id,
            title,
            tool_kind,
            status,
            parent_tool_call_id,
            diffs,
            output_text,
            terminal_id,
        } => {
            let tool_id = make_tool_id(turn_id, tool_call_id);
            let base = finalize_open_thinking(flat_items, at);
            if !diffs.is_empty() {
                let next = upsert_file_operations(
                    base,
                    &tool_id,
                    tool_call_id,
                    title.as_deref().unwrap_or("Edit file"),
                    parent_tool_call_id.clone(),
                    diffs,
                    *status,
                );
                return normalize_tool_structure(next, turn_id);
            }
            let idx = base
                .iter()
                .position(|item| matches!(item, TranscriptItem::Tool(t) if t.id == tool_id));
            let next = if let Some(i) = idx {
                let tool = match &base[i] {
                    TranscriptItem::Tool(t) => t.clone(),
                    _ => unreachable!(),
                };
                let updated = update_tool_call_item(
                    &tool,
                    title.as_deref(),
                    *status,
                    output_text.clone(),
                    terminal_id.clone(),
                );
                let mut result = base;
                result[i] = TranscriptItem::Tool(updated);
                result
            } else if has_file_operations(&base, tool_call_id) || is_edit_kind(tool_kind.as_deref())
            {
                // No renderable row for edit-kind calls without diffs (the
                // diffs ARE the content) or updates to already-file-folded
                // tool calls — reference behavior.
                base
            } else {
                let tool = create_tool_call_item(ToolParams {
                    id: tool_id,
                    seq: next_seq(&base),
                    tool_call_id: tool_call_id.clone(),
                    title: title.clone().unwrap_or_else(|| "unknown".to_string()),
                    tool_kind: tool_kind.clone(),
                    status: *status,
                    parent_tool_call_id: parent_tool_call_id.clone(),
                    input_summary: None,
                    output_text: output_text.clone(),
                    terminal_id: terminal_id.clone(),
                });
                upsert_tool_call_item(base, tool)
            };
            let next = update_file_operation_statuses(next, tool_call_id, *status);
            normalize_tool_structure(next, turn_id)
        }

        FoldEvent::Plan { entries } => {
            let base = finalize_open_thinking(flat_items, at);
            normalize_tool_structure(upsert_plan_tool_call(base, turn_id, entries), turn_id)
        }
    }
}

// -- finalization ----------------------------------------------------------------

/// Settle all in-progress states for a committed turn (reference
/// `finalizeItems`): thinking → done + duration, running tools → done.
pub fn finalize_items(items: &[TranscriptItem], at: u64) -> Vec<TranscriptItem> {
    fn finalize_node(node: ToolNode) -> ToolNode {
        match node {
            ToolNode::Group(group) => {
                let children: Vec<ToolNode> =
                    group.children.into_iter().map(finalize_node).collect();
                let status = read_group_status(&children);
                ToolNode::Group(ToolGroup {
                    status,
                    children,
                    ..group
                })
            }
            ToolNode::Tool(mut tool) => {
                if tool.status == ToolStatus::Running {
                    tool.status = ToolStatus::Done;
                }
                if let Some(children) = tool.children.take() {
                    let children: Vec<ToolNode> = children.into_iter().map(finalize_node).collect();
                    if !children.is_empty() {
                        tool.children = Some(children);
                    }
                }
                ToolNode::Tool(tool)
            }
        }
    }

    items
        .iter()
        .map(|item| match item {
            TranscriptItem::Thinking(t) if t.status == ThinkingStatus::Thinking => {
                TranscriptItem::Thinking(TranscriptThinking {
                    status: ThinkingStatus::Done,
                    duration_ms: Some(at.saturating_sub(t.started_at)),
                    ..t.clone()
                })
            }
            TranscriptItem::Tool(t) => {
                TranscriptItem::Tool(match finalize_node(ToolNode::Tool(t.clone())) {
                    ToolNode::Tool(tool) => tool,
                    _ => unreachable!(),
                })
            }
            TranscriptItem::Group(g) => {
                TranscriptItem::Group(match finalize_node(ToolNode::Group(g.clone())) {
                    ToolNode::Group(group) => group,
                    _ => unreachable!(),
                })
            }
            other => other.clone(),
        })
        .collect()
}

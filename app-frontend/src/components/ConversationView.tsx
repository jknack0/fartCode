// Conversation tab (E2-11-6): the structured-chat surface for an ACP
// conversation — transcript (two-tier: memoized settled turns + live active
// turn), stick-to-bottom scrolling, docked plan strip, queued prompts,
// composer-docked permission prompts, and the empty/starting/stopped/error
// states. Reads the conversations store; the backend streams live-model
// snapshots over acp:transcript and the view rehydrates from acp_history on
// mount (restart/restore path). ADR-0031.
//
// Keyboard: the composer is a native textarea, so the editor scope owns it
// (E14 — no separate conversation-view scope). Enter sends, Shift+Enter
// breaks the line; ⌘Enter keeps working through the global send-context
// command, which routes to this same conversation.
import { useEffect, useRef, useState } from "react";
import { useConversations, type PermissionPrompt } from "../store/conversations";
import type { LiveModels, PlanEntry } from "../lib/tauri";
import { SettledTurn, TurnBlock, NO_AWAITING } from "./TranscriptItems";

const COMPOSER_MAX_ROWS = 8;

/** Stop reasons worth a notice (reference stopReasonNotice mapping). */
const STOP_NOTICE: Record<string, string> = {
  max_turn_requests: "Turn limit reached.",
  max_tokens: "Response truncated — the model hit its token limit.",
  refusal: "The agent refused this request.",
};

const PLAN_GLYPH: Record<PlanEntry["status"], string> = {
  pending: "○",
  in_progress: "◐",
  completed: "●",
};

function PlanStrip({ plan }: { plan: NonNullable<LiveModels["plan"]> }) {
  const [open, setOpen] = useState(true);
  const done = plan.entries.filter((e) => e.status === "completed").length;
  return (
    <div className="chat-plan">
      <button
        className="chat-plan-header"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
      >
        <span className={`chat-chevron${open ? " open" : ""}`}>›</span>
        Plan
        <span className="chat-plan-count">
          {done}/{plan.entries.length} done
        </span>
      </button>
      {open && (
        <ul className="chat-plan-entries">
          {plan.entries.map((entry) => (
            <li
              key={entry.id}
              className={`chat-plan-entry ${entry.status}${
                entry.priority === "low" ? " low" : ""
              }`}
            >
              <span className="chat-plan-glyph">{PLAN_GLYPH[entry.status]}</span>
              {entry.content}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function PermissionBand({
  prompt,
  queueLength,
}: {
  prompt: PermissionPrompt;
  queueLength: number;
}) {
  const [resolving, setResolving] = useState(false);
  const title =
    prompt.pending.toolCall.title ?? prompt.pending.toolCall.toolCallId ?? "this action";
  const resolve = (optionId: string) => {
    setResolving(true);
    void useConversations
      .getState()
      .resolvePermission(prompt.conversationId, prompt.requestId, optionId)
      .catch((e) => console.error("permission resolve failed", e))
      .finally(() => setResolving(false));
  };
  return (
    <div className="chat-permission" data-request-id={prompt.requestId}>
      <div className="chat-permission-label">
        <span className="chat-status awaiting">◆</span>
        <span>
          Allow <span className="chat-permission-title">{title}</span>?
          {queueLength > 1 && (
            <span className="chat-permission-count"> (1 of {queueLength})</span>
          )}
        </span>
      </div>
      <div className="chat-permission-actions">
        {prompt.pending.options.map((option) => (
          <button
            key={option.optionId}
            className={
              option.kind.startsWith("allow")
                ? "primary"
                : option.kind.startsWith("reject")
                  ? "danger"
                  : ""
            }
            disabled={resolving}
            onClick={() => resolve(option.optionId)}
          >
            {option.name}
          </button>
        ))}
      </div>
    </div>
  );
}

export default function ConversationView({
  conversationId,
  taskId,
  active,
}: {
  conversationId: string;
  taskId: string;
  active: boolean;
}) {
  const conversationsLoaded = useConversations((s) => s.byTask[taskId] != null);
  const conversation = useConversations(
    (s) => (s.byTask[taskId] ?? []).find((c) => c.id === conversationId) ?? null,
  );
  const models = useConversations((s) => s.models[conversationId]);
  const prompts = useConversations((s) => s.permissions[conversationId]);
  const draft = useConversations((s) => s.drafts[conversationId] ?? "");
  const [sendError, setSendError] = useState<string | null>(null);

  const scrollRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const atBottomRef = useRef(true);
  const [atBottom, setAtBottom] = useState(true);

  // Restore path: a restart (or a tab restored from view-state) has no live
  // snapshot yet — pull the reduced history if the session is running.
  useEffect(() => {
    void useConversations.getState().ensure(taskId).catch(() => {});
    void useConversations
      .getState()
      .hydrate(conversationId)
      .catch((e) => console.warn("history hydrate failed:", e));
  }, [conversationId, taskId]);

  // Stick to bottom: follow streaming output unless the user scrolled up.
  useEffect(() => {
    const el = scrollRef.current;
    if (el && atBottomRef.current) el.scrollTop = el.scrollHeight;
  }, [models]);

  // Activation focus mirrors TerminalView: switching back to this tab puts
  // the keyboard in the composer.
  useEffect(() => {
    if (active) textareaRef.current?.focus();
  }, [active]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const next = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
    atBottomRef.current = next;
    setAtBottom(next);
  };

  const scrollToBottom = () => {
    const el = scrollRef.current;
    if (!el) return;
    atBottomRef.current = true;
    setAtBottom(true);
    el.scrollTop = el.scrollHeight;
  };

  const send = () => {
    const text = draft.trim();
    if (!text) return;
    setSendError(null);
    void useConversations
      .getState()
      .sendPrompt(conversationId, text)
      .catch((e) => setSendError(String(e)));
  };

  const cancel = () => {
    void useConversations
      .getState()
      .cancel(conversationId)
      .catch((e) => setSendError(String(e)));
  };

  // The conversation row is gone (task teardown races the tab) — a dead-end
  // state rather than a crash; ⌘W closes the tab.
  if (conversationsLoaded && !conversation) {
    return (
      <div className="conversation-view">
        <div className="chat-hero">
          <h2>Conversation no longer exists</h2>
          <p className="muted">Close this tab with ⌘W.</p>
        </div>
      </div>
    );
  }

  const session = models?.sessionState;
  const committed = models?.committed ?? [];
  const activeTurn = models?.activeTurn ?? null;
  const isEmpty = committed.length === 0 && activeTurn == null;
  const isGenerating = session?.isGenerating ?? false;
  const queuedPrompts = session?.queuedPrompts ?? [];
  const pending = prompts ?? [];
  const awaitingIds =
    pending.length > 0
      ? new Set(
          pending
            .map((p) => p.pending.toolCall.toolCallId)
            .filter((id): id is string => id != null),
        )
      : NO_AWAITING;

  const stopNotice =
    !isGenerating && session?.lastStopReason
      ? STOP_NOTICE[session.lastStopReason]
      : undefined;
  const rows = Math.min(COMPOSER_MAX_ROWS, Math.max(1, draft.split("\n").length));

  return (
    <div className="conversation-view">
      <div className="chat-scroll" ref={scrollRef} onScroll={onScroll}>
        {isEmpty ? (
          <div className="chat-hero">
            <h2>Structured chat</h2>
            <p className="muted">
              {session?.lifecycle === "starting"
                ? "Starting the agent…"
                : "Send a prompt to start working with the agent."}
            </p>
          </div>
        ) : (
          <div className="chat-transcript">
            {committed.map((turn) => (
              <SettledTurn key={turn.id} turn={turn} awaitingIds={NO_AWAITING} />
            ))}
            {activeTurn && (
              <TurnBlock turn={activeTurn} awaitingIds={awaitingIds} />
            )}
            {isGenerating && (
              <div className="chat-working">
                <span className="chat-status running" />
                Working…
              </div>
            )}
          </div>
        )}
      </div>

      {!atBottom && (
        <button className="chat-jump" onClick={scrollToBottom}>
          ↓ Latest
        </button>
      )}

      <div className="chat-dock">
        {models?.plan && models.plan.entries.length > 0 && (
          <PlanStrip plan={models.plan} />
        )}
        {queuedPrompts.length > 0 && (
          <div className="chat-queued">
            {queuedPrompts.map((q, i) => (
              <div key={q.id} className="chat-queued-row">
                <span className="chat-queued-index">{i + 1}</span>
                <span className="chat-queued-text">{q.text}</span>
              </div>
            ))}
          </div>
        )}
        {pending.length > 0 && (
          <PermissionBand prompt={pending[0]} queueLength={pending.length} />
        )}
        {session?.lifecycle === "starting" && !isEmpty && (
          <div className="chat-notice">Resuming session…</div>
        )}
        {session?.lifecycle === "closed" && (
          <div className="chat-notice">
            Session stopped — sending a prompt starts it again.
          </div>
        )}
        {stopNotice && <div className="chat-notice">{stopNotice}</div>}
        {sendError && (
          <div className="chat-notice error">
            {sendError}
            <button className="chat-notice-dismiss" onClick={() => setSendError(null)}>
              Dismiss
            </button>
          </div>
        )}
        <div className="chat-composer">
          <textarea
            ref={textareaRef}
            rows={rows}
            placeholder="Prompt the agent — Enter to send, Shift+Enter for a new line"
            value={draft}
            onChange={(e) =>
              useConversations.getState().setDraft(conversationId, e.target.value)
            }
            onKeyDown={(e) => {
              if (
                e.key === "Enter" &&
                !e.shiftKey &&
                !e.metaKey &&
                !e.ctrlKey &&
                !e.nativeEvent.isComposing
              ) {
                e.preventDefault();
                send();
              }
            }}
          />
          <div className="chat-composer-actions">
            {models?.usage && (
              <span className="chat-usage" title="Context used">
                {Math.round(models.usage.contextUsed / 1000)}k /{" "}
                {Math.round(models.usage.contextSize / 1000)}k
              </span>
            )}
            {isGenerating && (
              <button className="chat-stop danger" onClick={cancel}>
                Stop
              </button>
            )}
            <button
              className="chat-send primary"
              disabled={draft.trim().length === 0}
              onClick={send}
            >
              {isGenerating ? "Queue" : "Send"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

// Tab bar (E2-10): one per task pane. Tabs are the task's conversations
// (the registered tab kind); click to activate, × to close (⌘W), + to start
// a new conversation (⌘T).
import { useTabs, type PaneId } from "../store/tabs";
import { useConversations } from "../store/conversations";

export default function TabBar({
  taskId,
  projectId,
  pane,
}: {
  taskId: string;
  projectId: string;
  pane: PaneId;
}) {
  const panes = useTabs((s) => s.panesByTask[taskId]);
  const activePane = useTabs((s) => s.activePaneByTask[taskId] ?? "left");
  const paneState = panes?.[pane];
  if (!paneState) return null;

  const newConversation = () => {
    void (async () => {
      const conv = await useConversations
        .getState()
        .create(taskId, projectId, undefined, "New conversation", undefined);
      useTabs.getState().addTab(taskId, pane, {
        id: conv.id,
        kind: "conversation",
        title: conv.title,
      });
    })();
  };

  const activate = (tabId: string) => {
    useTabs.getState().setActiveTab(taskId, pane, tabId);
    useConversations.getState().setActive(tabId);
  };

  return (
    <div className={`tab-bar${activePane === pane ? " active" : ""}`}>
      {paneState.tabs.map((tab) => (
        <div
          key={tab.id}
          className={`tab${tab.id === paneState.activeId ? " active" : ""}`}
          onClick={() => activate(tab.id)}
          title={`${tab.title} — ${tab.id === paneState.activeId ? "⌘W to close" : "click to activate"}`}
        >
          <span className="tab-title">{tab.title}</span>
          <button
            className="tab-close"
            title="Close tab (⌘W)"
            onClick={(e) => {
              e.stopPropagation();
              useTabs.getState().closeTab(taskId, pane, tab.id);
            }}
          >
            &times;
          </button>
        </div>
      ))}
      <button
        className="tab-add"
        title="New conversation (⌘T)"
        onClick={newConversation}
      >
        +
      </button>
    </div>
  );
}

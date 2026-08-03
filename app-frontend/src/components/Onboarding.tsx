// Onboarding (E1-08): skip-able steps — add a project, (optional) install an
// agent, (optional) sign in. Offline-OK: everything can be skipped and the
// app lands on the (empty) project view. Completion is recorded in
// view-state so it shows once.
import { useEffect, useState } from "react";
import { getViewState, setViewState } from "../lib/tauri";
import { useSidebar } from "../store/sidebar";

const ONBOARDING_KEY = "view-state:app:onboarding";

type Step = "welcome" | "add-project" | "agent" | "signin" | "done";

export default function Onboarding() {
  const [step, setStep] = useState<Step | null>(null);
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);
  const createProject = useSidebar((s) => s.createProject);

  const addProject = async () => {
    setError(null);
    try {
      await createProject(path.trim());
      setStep("agent");
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    getViewState(ONBOARDING_KEY)
      .then((v) => {
        if ((v as { done?: boolean } | null)?.done !== true) {
          setStep("welcome");
        }
      })
      .catch(() => setStep("welcome"));
  }, []);

  if (step === null || step === "done") return null;

  const finish = () => {
    setViewState(ONBOARDING_KEY, { done: true }).catch(() => {});
    setStep("done");
  };

  return (
    <div className="modal-backdrop">
      <div className="modal onboarding">
        {step === "welcome" && (
          <>
            <h2>Welcome to ade</h2>
            <p className="muted">
              ade runs coding agents in isolated worktrees. Three quick steps
              — all optional, all skippable.
            </p>
            <div className="modal-actions">
              <button onClick={finish}>Skip</button>
              <button className="primary" onClick={() => setStep("add-project")}>
                Get started
              </button>
            </div>
          </>
        )}

        {step === "add-project" && (
          <>
            <h2>Add a project</h2>
            <p className="muted">Path to a local git repository.</p>
            <input
              autoFocus
              value={path}
              onChange={(e) => setPath(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && addProject()}
              placeholder="/path/to/repo"
            />
            {error && <p className="error">{error}</p>}
            <div className="modal-actions">
              <button onClick={() => setStep("agent")}>Skip</button>
              <button
                className="primary"
                disabled={!path.trim()}
                onClick={addProject}
              >
                Add project
              </button>
            </div>
          </>
        )}

        {step === "agent" && (
          <>
            <h2>Install an agent?</h2>
            <p className="muted">
              Agent installs (Claude Code, Codex, …) arrive with the E3
              dependency tickets. You can skip and install later.
            </p>
            <div className="modal-actions">
              <button onClick={() => setStep("signin")}>Skip for now</button>
              <button className="primary" onClick={() => setStep("signin")}>
                Later
              </button>
            </div>
          </>
        )}

        {step === "signin" && (
          <>
            <h2>Connect GitHub?</h2>
            <p className="muted">
              Issue linking and remote signing arrive with Phase 1. Offline
              mode works fine.
            </p>
            <div className="modal-actions">
              <button onClick={finish}>Skip</button>
              <button className="primary" onClick={finish}>
                Done
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

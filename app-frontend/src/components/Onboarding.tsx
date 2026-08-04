// Onboarding (E1-08): skip-able steps — add a project, (optional) install an
// agent, (optional) sign in. Offline-OK: everything can be skipped and the
// app lands on the (empty) project view. Completion is recorded in
// view-state so it shows once. The modal registry (E14-01) tracks it.
import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getViewState, setViewState } from "../lib/tauri";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

const ONBOARDING_KEY = "view-state:app:onboarding";

type Step = "welcome" | "add-project" | "agent" | "signin";

export default function Onboarding() {
  const openFlag = useUi((s) => s.onboardingOpen);
  const setOpenFlag = useUi((s) => s.setOnboardingOpen);
  const [step, setStep] = useState<Step>("welcome");
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
        const done = (v as { done?: boolean } | null)?.done === true;
        setOpenFlag(!done);
      })
      .catch(() => setOpenFlag(true));
  }, [setOpenFlag]);

  const finish = () => {
    setViewState(ONBOARDING_KEY, { done: true }).catch(() => {});
    setOpenFlag(false);
  };

  if (!openFlag) return null;

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
            <div className="path-picker">
              <input
                autoFocus
                value={path}
                onChange={(e) => setPath(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && addProject()}
                placeholder="/path/to/repo"
              />
              <button
                type="button"
                onClick={async () => {
                  try {
                    const selected = await open({ directory: true, multiple: false });
                    if (selected) setPath(selected as string);
                  } catch (e) {
                    setError("Dialog failed: " + String(e));
                  }
                }}
              >
                Browse…
              </button>
            </div>
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

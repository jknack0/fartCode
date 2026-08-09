// Onboarding (E1-08): skip-able steps — add a project, (optional) install an
// agent, (optional) sign in. Offline-OK: everything can be skipped and the
// app lands on the (empty) project view. Completion is recorded in
// view-state so it shows once. The modal registry (E14-01) tracks it.
// Styling (design_handoff_v2 7d card): system grammar — 15px/600 titles,
// 13px copy, mono key-labelled actions — no legacy .modal plate.
import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getViewState, setViewState } from "../lib/tauri";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";
import AgentsList from "./AgentsList";

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

  // ↵ advances the current step (the actions are key-labelled). Inputs and
  // focused buttons keep their own Enter behaviour.
  useEffect(() => {
    if (!openFlag) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Enter") return;
      const t = e.target as HTMLElement | null;
      if (t && ["INPUT", "TEXTAREA", "SELECT", "BUTTON"].includes(t.tagName)) return;
      e.preventDefault();
      if (step === "welcome") setStep("add-project");
      else if (step === "add-project") {
        if (path.trim()) void addProject();
      } else if (step === "agent") setStep("signin");
      else finish();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openFlag, step, path]);

  if (!openFlag) return null;

  return (
    <div className="modal-backdrop">
      <div className="fc-onboard">
        {step === "welcome" && (
          <>
            <div className="fc-onboard-title">Welcome to fartCode</div>
            <p className="fc-onboard-copy">
              fartCode runs coding agents in isolated worktrees. Three quick steps
              — all optional, all skippable.
            </p>
            <div className="fc-onboard-actions">
              <button className="fc-onboard-action" onClick={finish}>
                skip
              </button>
              <button
                className="fc-onboard-action main"
                onClick={() => setStep("add-project")}
              >
                ↵ get started
              </button>
            </div>
          </>
        )}

        {step === "add-project" && (
          <>
            <div className="fc-onboard-title">Add a project</div>
            <p className="fc-onboard-copy">Path to a local git repository.</p>
            <div className="fc-onboard-path">
              <input
                autoFocus
                value={path}
                onChange={(e) => setPath(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && addProject()}
                placeholder="/path/to/repo"
              />
              <button
                type="button"
                className="fc-onboard-action"
                onClick={async () => {
                  try {
                    const selected = await open({ directory: true, multiple: false });
                    if (selected) setPath(selected as string);
                  } catch (e) {
                    setError("Dialog failed: " + String(e));
                  }
                }}
              >
                browse…
              </button>
            </div>
            {error && <p className="fc-set-error">{error}</p>}
            <div className="fc-onboard-actions">
              <button className="fc-onboard-action" onClick={() => setStep("agent")}>
                skip
              </button>
              <button
                className="fc-onboard-action main"
                disabled={!path.trim()}
                onClick={addProject}
              >
                ↵ add project
              </button>
            </div>
          </>
        )}

        {step === "agent" && (
          <>
            <div className="fc-onboard-title">Agents on this machine</div>
            <p className="fc-onboard-copy">
              no agents found means the composer can&apos;t start anything
            </p>
            <div className="fc-onboarding-agents">
              <AgentsList />
            </div>
            <div className="fc-onboard-actions">
              <button className="fc-onboard-action" onClick={() => setStep("signin")}>
                skip
              </button>
              <button
                className="fc-onboard-action main"
                onClick={() => setStep("signin")}
              >
                ↵ continue
              </button>
            </div>
          </>
        )}

        {step === "signin" && (
          <>
            <div className="fc-onboard-title">Connect GitHub?</div>
            <p className="fc-onboard-copy">
              Issue linking and remote signing arrive with Phase 1. Offline
              mode works fine.
            </p>
            <div className="fc-onboard-actions">
              <button className="fc-onboard-action" onClick={finish}>
                skip
              </button>
              <button className="fc-onboard-action main" onClick={finish}>
                ↵ done
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

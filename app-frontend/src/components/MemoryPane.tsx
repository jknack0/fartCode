// Memory value dashboard (#76, handoff v3 §8g; ADR-0038 item 7).
//
// The one inviolable rule, inherited from fartcode-telemetry's types: the
// signals arrive as Unknown / Insufficient / Estimated / NoData /
// SinglePoint / Trend states and are RENDERED as those states, never
// coerced to numbers. An empty project shows four honest blanks, not four
// zeroes; a single landing gets no sparkline and no arrow; and time-to-land
// never renders without the caveat the payload welds to it (`caveat` comes
// from the payload verbatim — it is deliberately not hard-coded here).

import { useEffect, useState } from "react";
import { telemetryMemoryValue } from "../lib/tauri";
import type { MemoryValueDto, TimeToLandKindDto } from "../lib/tauri";

const SPARK_GLYPHS = "▁▂▃▄▅▆▇█";
/** Sparkline cap: the last N landings. Enough to show a shape; a settings
 * row is not a chart surface. */
const SPARK_MAX = 24;

/** Mono block-glyph sparkline, min-max normalized. An all-equal series is a
 * flat run of the mid glyph — no invented slope. */
export function sparkline(hours: number[]): string {
  const tail = hours.slice(-SPARK_MAX);
  if (tail.length === 0) return "";
  const min = Math.min(...tail);
  const max = Math.max(...tail);
  if (max === min) return SPARK_GLYPHS[3].repeat(tail.length);
  return tail
    .map((h) => SPARK_GLYPHS[Math.round(((h - min) / (max - min)) * (SPARK_GLYPHS.length - 1))])
    .join("");
}

/** Hours under 48 read as hours ("30.0h"), otherwise days ("4.2d"). */
export function formatDuration(hours: number): string {
  return hours < 48 ? `${hours.toFixed(1)}h` : `${(hours / 24).toFixed(1)}d`;
}

function formatTokens(n: number): string {
  const abs = Math.abs(n);
  if (abs >= 10_000) return `${Math.round(abs / 1_000)}k`;
  if (abs >= 1_000) return `${(abs / 1_000).toFixed(1)}k`;
  return String(abs);
}

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

function timeToLandValue(kind: TimeToLandKindDto): string {
  switch (kind.kind) {
    case "noData":
      return "no landed feature with both created and merged breadcrumbs yet";
    case "singlePoint":
      return `${formatDuration(kind.hours)} · one landing — a point, not a trend`;
    case "trend":
      return `${formatDuration(kind.earlierMedianHours)} → ${formatDuration(
        kind.laterMedianHours,
      )} · ${sparkline(kind.landedHours)} · ${kind.landed} landed`;
  }
}

export function MemoryPane({ projectId }: { projectId: string }) {
  const [value, setValue] = useState<MemoryValueDto | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    setValue(null);
    setError(null);
    telemetryMemoryValue(projectId)
      .then((v) => {
        if (alive) setValue(v);
      })
      .catch((e) => {
        if (alive) setError(String(e));
      });
    return () => {
      alive = false;
    };
  }, [projectId]);

  if (error) {
    return (
      <div className="fc-set-pane-body">
        <p className="fc-set-error">{error}</p>
      </div>
    );
  }
  if (!value) {
    return (
      <div className="fc-set-pane-body">
        <div className="fc-set-loading">loading…</div>
      </div>
    );
  }

  const { citations, reAsk, tokensSaved, timeToLand } = value;

  // Headline: a count only when the re-ask signal was actually observed.
  // Unknown is an honest headline, NOT "0 re-explanations avoided".
  const headline =
    reAsk.kind === "observed"
      ? `${plural(reAsk.memoryAnswered, "re-explanation")} avoided`
      : "no memory signal yet";

  // Citations row.
  const conclusive = citations.citedRead + citations.citedMention + citations.notCited;
  const cited = citations.citedRead + citations.citedMention;
  const citationsValue =
    conclusive > 0
      ? `${Math.round((cited / conclusive) * 100)}% · ${cited} of ${conclusive} steps`
      : `unknown · ${plural(citations.sessions, "session")}, none attributable`;
  const citationsSubParts: string[] = [];
  if (conclusive > 0) {
    citationsSubParts.push(
      `${citations.citedRead} through a read or search, ${citations.citedMention} named it only`,
    );
    if (citations.wroteWithoutReading > 0) {
      citationsSubParts.push(
        `${citations.wroteWithoutReading} wrote their section without reading the file`,
      );
    }
    if (citations.unknown > 0) {
      // The excluded-but-cited count travels with the exclusion (its whole
      // reason to exist in the payload): the rate is a floor when some of
      // the sessions it could not count did name the dossier.
      citationsSubParts.push(
        `${plural(citations.unknown, "session")} excluded as unattributable${
          citations.unknownWithHit > 0
            ? ` (${citations.unknownWithHit} of them did name it)`
            : ""
        }`,
      );
    }
  }

  // Re-ask row.
  const reAskValue =
    reAsk.kind === "observed"
      ? `${Math.round(
          (reAsk.humanAsked / (reAsk.humanAsked + reAsk.memoryAnswered)) * 100,
        )}% re-asked · ${reAsk.humanAsked} to you, ${reAsk.memoryAnswered} from memory · ${plural(
          reAsk.stepsTagged,
          "tagged step",
        )}`
      : `unknown · no tagged clarifications in ${plural(reAsk.stepsScanned, "readable step")}`;
  const reAskSub =
    reAsk.stepsUnreadable > 0 ? `${plural(reAsk.stepsUnreadable, "step")} could not be read` : null;

  // Tokens row.
  const tokensValue =
    tokensSaved.kind === "estimated"
      ? `≈${formatTokens(tokensSaved.windowTotal)} ${
          tokensSaved.windowTotal >= 0 ? "lower" : "higher"
        } · ${tokensSaved.citingSessions} vs ${tokensSaved.notCitingSessions} sessions`
      : `not enough data · ${tokensSaved.citingWithUsage} citing / ${tokensSaved.notCitingWithUsage} non-citing of ${tokensSaved.neededPerArm} needed each`;
  const tokensSub =
    tokensSaved.kind === "estimated"
      ? "estimated from a context-window gauge, not a billing figure; observational"
      : null;

  // Honesty footer.
  const footerParts = [
    `${plural(value.sessionsObserved, "session")} observed`,
    `${plural(value.dossiersScanned, "dossier")} scanned`,
  ];
  if (value.sectionsUndated > 0) {
    footerParts.push(`${plural(value.sectionsUndated, "undated section")} excluded`);
  }
  if (value.cyclesOutsideWindow > 0) {
    footerParts.push(`${plural(value.cyclesOutsideWindow, "landing")} outside the window`);
  }
  if (value.clipped) {
    footerParts.push("input clipped — figures are a floor");
  }

  return (
    <div className="fc-set-pane-body">
      <div className="fc-mem-head">
        <div className="fc-mem-headline">{headline}</div>
        <div className="fc-mem-subline">
          last {value.windowDays} days · computed locally, never leaves this machine
        </div>
      </div>

      <div className="fc-mem-rows">
        <div className="fc-mem-row-wrap">
          <div className="fc-mem-row">
            <span className="fc-set-label">Memory citations</span>
            <span className="fc-set-value">{citationsValue}</span>
          </div>
          {citationsSubParts.length > 0 && (
            <div className="fc-mem-sub">{citationsSubParts.join(" · ")}</div>
          )}
        </div>

        <div className="fc-mem-row-wrap">
          <div className="fc-mem-row">
            <span className="fc-set-label">Re-ask rate</span>
            <span className="fc-set-value">{reAskValue}</span>
          </div>
          {reAskSub && <div className="fc-mem-sub">{reAskSub}</div>}
        </div>

        <div className="fc-mem-row-wrap">
          <div className="fc-mem-row">
            <span className="fc-set-label">Context tokens saved</span>
            <span className="fc-set-value">{tokensValue}</span>
          </div>
          {tokensSub && <div className="fc-mem-sub">{tokensSub}</div>}
        </div>

        <div className="fc-mem-row-wrap">
          <div className="fc-mem-row">
            <span className="fc-set-label">Time to land</span>
            <span className="fc-set-value">{timeToLandValue(timeToLand.kind)}</span>
          </div>
          {/* The caveat row is unconditional whenever the row renders — the
              text is the payload's, verbatim, never hard-coded. */}
          <div className="fc-mem-caveat">{timeToLand.caveat}</div>
        </div>
      </div>

      <div className="fc-set-spacer" />
      <div className="fc-mem-footer">{footerParts.join(" · ")}</div>
    </div>
  );
}

export default MemoryPane;

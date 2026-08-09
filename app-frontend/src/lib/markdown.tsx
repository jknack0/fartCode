// Minimal markdown → React renderer for ticket bodies (headings, lists,
// fenced/inline code, bold/italic, links). Builds React elements — never
// innerHTML — so arbitrary agent/user text is safe by construction.
// ponytail: no tables/blockquotes/nesting; extend when a real ticket needs it.

import { open } from "@tauri-apps/plugin-shell";
import type { ReactNode } from "react";

const INLINE = /(`[^`\n]+`)|(\*\*[^*\n]+\*\*)|(\*[^*\n]+\*)|(\[[^\]\n]+\]\((?:https?:\/\/)[^)\s]+\))/g;

function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  for (const m of text.matchAll(INLINE)) {
    const idx = m.index!;
    if (idx > last) out.push(text.slice(last, idx));
    const tok = m[0];
    const key = `${idx}`;
    if (tok.startsWith("`")) {
      out.push(<code key={key}>{tok.slice(1, -1)}</code>);
    } else if (tok.startsWith("**")) {
      out.push(<strong key={key}>{inline(tok.slice(2, -2))}</strong>);
    } else if (tok.startsWith("*")) {
      out.push(<em key={key}>{inline(tok.slice(1, -1))}</em>);
    } else {
      const link = tok.match(/^\[([^\]]+)\]\(([^)]+)\)$/)!;
      out.push(
        <a
          key={key}
          href={link[2]}
          onClick={(e) => {
            e.preventDefault();
            void open(link[2]).catch(() => {});
          }}
        >
          {link[1]}
        </a>,
      );
    }
    last = idx + tok.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** Render a markdown string as a list of block-level React nodes. */
export function renderMarkdown(src: string): ReactNode[] {
  const lines = src.split("\n");
  const out: ReactNode[] = [];
  let para: string[] = [];
  let list: { ordered: boolean; items: string[] } | null = null;
  let code: string[] | null = null;

  const flushPara = () => {
    if (para.length) {
      out.push(<p key={out.length}>{inline(para.join(" "))}</p>);
      para = [];
    }
  };
  const flushList = () => {
    if (list) {
      const items = list.items.map((it, i) => <li key={i}>{inline(it)}</li>);
      out.push(
        list.ordered ? <ol key={out.length}>{items}</ol> : <ul key={out.length}>{items}</ul>,
      );
      list = null;
    }
  };

  for (const line of lines) {
    if (code !== null) {
      if (/^```/.test(line)) {
        out.push(
          <pre key={out.length}>
            <code>{code.join("\n")}</code>
          </pre>,
        );
        code = null;
      } else {
        code.push(line);
      }
      continue;
    }
    const heading = line.match(/^(#{1,4})\s+(.*)$/);
    const bullet = line.match(/^\s*[-*]\s+(?:\[[ xX]\]\s+)?(.*)$/);
    const numbered = line.match(/^\s*\d+[.)]\s+(.*)$/);
    if (/^```/.test(line)) {
      flushPara();
      flushList();
      code = [];
    } else if (heading) {
      flushPara();
      flushList();
      const level = Math.min(heading[1].length + 2, 6); // # → h3 … within the sheet
      const H = `h${level}` as "h3" | "h4" | "h5" | "h6";
      out.push(<H key={out.length}>{inline(heading[2])}</H>);
    } else if (bullet) {
      flushPara();
      if (!list || list.ordered) {
        flushList();
        list = { ordered: false, items: [] };
      }
      list.items.push(bullet[1]);
    } else if (numbered) {
      flushPara();
      if (!list || !list.ordered) {
        flushList();
        list = { ordered: true, items: [] };
      }
      list.items.push(numbered[1]);
    } else if (line.trim() === "") {
      flushPara();
      flushList();
    } else {
      flushList();
      para.push(line.trim());
    }
  }
  if (code !== null)
    out.push(
      <pre key={out.length}>
        <code>{code.join("\n")}</code>
      </pre>,
    );
  flushPara();
  flushList();
  return out;
}

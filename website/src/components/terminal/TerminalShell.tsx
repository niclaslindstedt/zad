import { useState } from "react";
import type { TerminalDemo } from "../../data/terminalDemos";
import { useTerminalAnimation } from "../../hooks/useTerminalAnimation";
import TerminalLine from "./TerminalLine";

interface TerminalShellProps {
  tabs: TerminalDemo[];
  className?: string;
}

export default function TerminalShell({ tabs, className }: TerminalShellProps) {
  const [activeId, setActiveId] = useState(tabs[0]?.id ?? "");
  const active = tabs.find((t) => t.id === activeId) ?? tabs[0];
  const rendered = useTerminalAnimation({
    lines: active.lines,
    resetKey: active.id,
  });

  return (
    <div
      className={`overflow-hidden rounded-xl border border-border bg-surface-alt shadow-2xl shadow-accent/5 ${className ?? ""}`}
    >
      {/* Window chrome */}
      <div className="flex items-center justify-between border-b border-border bg-surface px-4 py-2.5">
        <div className="flex items-center gap-1.5">
          <span className="h-3 w-3 rounded-full bg-[#ff5f57]" />
          <span className="h-3 w-3 rounded-full bg-[#febc2e]" />
          <span className="h-3 w-3 rounded-full bg-[#28c840]" />
        </div>
        <div className="hidden text-xs text-text-dim sm:block">zad</div>
        <div className="flex items-center gap-1">
          {tabs.map((tab) => (
            <button
              key={tab.id}
              onClick={() => setActiveId(tab.id)}
              className={`cursor-pointer rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                tab.id === active.id
                  ? "bg-surface-hover text-text-primary"
                  : "text-text-dim hover:text-text-secondary"
              }`}
            >
              {tab.label}
            </button>
          ))}
        </div>
      </div>

      {/* Body */}
      <div className="min-h-[260px] overflow-x-auto px-5 py-4 text-left font-mono text-[13px] leading-6 sm:text-sm">
        {rendered.length === 0 ? (
          <div className="text-text-dim">
            <span className="animate-blink-cursor" aria-hidden />
          </div>
        ) : (
          rendered.map((line, idx) => (
            <TerminalLine
              key={`${active.id}-${idx}`}
              kind={line.kind}
              text={line.text}
              typed={line.typed}
              prompt={line.kind === "prompt" ? active.prompt : undefined}
              showCursor={
                idx === rendered.length - 1 &&
                (line.kind === "prompt" ? !line.done : true)
              }
            />
          ))
        )}
      </div>
    </div>
  );
}

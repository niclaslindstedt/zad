import type { LineKind } from "../../data/terminalDemos";

interface TerminalLineProps {
  kind: LineKind;
  text: string;
  typed: number;
  showCursor: boolean;
  prompt?: string;
}

const colorByKind: Record<LineKind, string> = {
  prompt:  "text-text-primary",
  output:  "text-text-secondary",
  comment: "text-text-dim",
};

export default function TerminalLine({
  kind,
  text,
  typed,
  showCursor,
  prompt,
}: TerminalLineProps) {
  const visible = text.slice(0, typed);
  return (
    <div className={`whitespace-pre-wrap break-words ${colorByKind[kind]}`}>
      {kind === "prompt" && prompt && (
        <span className="text-accent">{prompt}</span>
      )}
      <span>{visible}</span>
      {showCursor && <span className="animate-blink-cursor" aria-hidden />}
    </div>
  );
}

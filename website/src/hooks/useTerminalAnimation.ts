import { useEffect, useRef, useState } from "react";
import type { DemoLine } from "../data/terminalDemos";

interface RenderedLine {
  kind: DemoLine["kind"];
  text: string;
  /** How much of `text` has been "typed" so far. */
  typed: number;
  done: boolean;
}

interface UseTerminalAnimationOptions {
  /** Source script for the active demo. */
  lines: DemoLine[];
  /** Milliseconds per typed character for prompt lines. */
  typeSpeedMs?: number;
  /** Pause before the loop restarts. */
  loopRestartMs?: number;
  /** When set, restarts the typewriter from scratch. */
  resetKey: string;
}

// A tiny animation state machine: type prompts character-by-character,
// dump output and comments instantly, dwell for `delayAfter`, then move
// on. When the script finishes, wait `loopRestartMs` and replay.
export function useTerminalAnimation({
  lines,
  typeSpeedMs = 32,
  loopRestartMs = 4000,
  resetKey,
}: UseTerminalAnimationOptions): RenderedLine[] {
  const [state, setState] = useState<RenderedLine[]>([]);
  const timerRef = useRef<number | null>(null);

  useEffect(() => {
    setState([]);
    let cancelled = false;
    let lineIdx = 0;
    let typedSoFar = 0;
    const acc: RenderedLine[] = [];

    function clearTimer() {
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    }

    function step() {
      if (cancelled) return;

      if (lineIdx >= lines.length) {
        // Loop.
        timerRef.current = window.setTimeout(() => {
          if (cancelled) return;
          lineIdx = 0;
          typedSoFar = 0;
          acc.length = 0;
          setState([]);
          step();
        }, loopRestartMs);
        return;
      }

      const line = lines[lineIdx];

      if (line.kind === "prompt") {
        if (typedSoFar === 0) {
          acc.push({ kind: line.kind, text: line.text, typed: 0, done: false });
        }
        typedSoFar = Math.min(typedSoFar + 1, line.text.length);
        acc[acc.length - 1] = {
          ...acc[acc.length - 1],
          typed: typedSoFar,
          done: typedSoFar === line.text.length,
        };
        setState([...acc]);

        if (typedSoFar < line.text.length) {
          timerRef.current = window.setTimeout(step, typeSpeedMs);
        } else {
          const after = line.delayAfter ?? 220;
          lineIdx += 1;
          typedSoFar = 0;
          timerRef.current = window.setTimeout(step, after);
        }
      } else {
        // output / comment — print instantly, then dwell.
        acc.push({
          kind: line.kind,
          text: line.text,
          typed: line.text.length,
          done: true,
        });
        setState([...acc]);
        const after = line.delayAfter ?? 80;
        lineIdx += 1;
        timerRef.current = window.setTimeout(step, after);
      }
    }

    step();
    return () => {
      cancelled = true;
      clearTimer();
    };
  }, [lines, typeSpeedMs, loopRestartMs, resetKey]);

  return state;
}

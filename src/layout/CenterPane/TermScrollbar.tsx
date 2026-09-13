//! Persistent terminal scrollbar backed by xterm's public buffer and scroll APIs. xterm 6 no longer
//! exposes a native scrolling viewport; subscribing to terminal lifecycle also covers delayed font loading.

import { useCallback, useEffect, useRef, useState } from "react";
import { getTerminal, watchTerminal } from "../../terminal/registry";

export function TermScrollbar({ sessionId, containerRef, hidden }: {
  sessionId: string;
  containerRef: React.RefObject<HTMLDivElement | null>;
  hidden: boolean;
}) {
  const [thumb, setThumb] = useState<{ top: number; height: number } | null>(null);
  const [hovered, setHovered] = useState(false);
  const dragging = useRef(false);
  const trackRef = useRef<HTMLDivElement | null>(null);
  const stopDragRef = useRef<(() => void) | null>(null);

  const update = useCallback(() => {
    const term = getTerminal(sessionId);
    const el = containerRef.current;
    if (!term || !el) { setThumb(null); return; }
    const buffer = term.buffer.active;
    // The track is a sibling of the scaled terminal. Measure their unscaled parent even before the
    // track first mounts, excluding the three-pixel inset at each end.
    const height = Math.max(0, (el.parentElement?.clientHeight ?? el.clientHeight) - 6);
    if (!height || !buffer.baseY) { setThumb(null); return; }
    const thumbHeight = Math.min(height, Math.max(24, height * term.rows / (buffer.baseY + term.rows)));
    const top = buffer.viewportY / buffer.baseY * (height - thumbHeight);
    setThumb((previous) => previous?.top === top && previous.height === thumbHeight
      ? previous : { top, height: thumbHeight });
  }, [containerRef, sessionId]);

  useEffect(() => {
    if (hidden) { setThumb(null); return; }
    const el = containerRef.current;
    if (!el) return;
    let raf = 0;
    let subscriptions: { dispose(): void }[] = [];
    const schedule = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(update);
    };
    const unwatch = watchTerminal(sessionId, (term) => {
      subscriptions.forEach((s) => s.dispose());
      subscriptions = term ? [term.onScroll(schedule), term.onWriteParsed(schedule), term.onResize(schedule)] : [];
      schedule();
    });
    const ro = new ResizeObserver(schedule);
    ro.observe(el);
    return () => {
      unwatch();
      subscriptions.forEach((s) => s.dispose());
      ro.disconnect();
      cancelAnimationFrame(raf);
      stopDragRef.current?.();
    };
  }, [containerRef, hidden, sessionId, update]);

  if (!thumb || hidden) return null;

  const scrollTo = (fraction: number) => {
    const term = getTerminal(sessionId);
    if (!term) return;
    term.scrollToLine(Math.round(Math.max(0, Math.min(1, fraction)) * term.buffer.active.baseY));
    containerRef.current?.dispatchEvent(new Event("vlx-terminal-user-scroll"));
  };

  const onThumbMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const track = trackRef.current;
    if (!track) return;
    stopDragRef.current?.();
    dragging.current = true;
    const rect = track.getBoundingClientRect();
    const grabOffset = e.clientY - rect.top - thumb.top;
    const onMove = (me: MouseEvent) => scrollTo((me.clientY - rect.top - grabOffset) / Math.max(1, rect.height - thumb.height));
    const onUp = () => {
      dragging.current = false;
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      stopDragRef.current = null;
    };
    stopDragRef.current = onUp;
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  };

  return <div
    ref={trackRef}
    data-terminal-scrollbar
    onMouseEnter={() => setHovered(true)}
    onMouseLeave={() => !dragging.current && setHovered(false)}
    onClick={(e) => {
      if (dragging.current || e.target !== e.currentTarget) return;
      const rect = e.currentTarget.getBoundingClientRect();
      scrollTo((e.clientY - rect.top - thumb.height / 2) / Math.max(1, rect.height - thumb.height));
    }}
    style={{ position: "absolute", top: 3, right: 6, width: 8, bottom: 3, borderRadius: 4,
      background: hovered ? "var(--border-strong)" : "var(--border)", zIndex: 4,
      cursor: "pointer", transition: "background .15s" }}
  >
    <div onMouseDown={onThumbMouseDown} style={{ position: "absolute", top: thumb.top, left: 0, right: 0,
      height: thumb.height, borderRadius: 4, background: hovered ? "var(--text-dim)" : "var(--text-faint)",
      transition: "background .15s" }} />
  </div>;
}

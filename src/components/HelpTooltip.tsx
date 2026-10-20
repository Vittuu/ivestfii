import { useCallback, useId, useLayoutEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import '../styles/help.css';

let closeActiveHelp: (() => void) | null = null;

/** Explicação de um rótulo, disponível com mouse, teclado e toque. */
export default function HelpTooltip({ text, children }: { text: string; children: ReactNode }) {
  const id = useId();
  const anchor = useRef<HTMLSpanElement>(null);
  const bubble = useRef<HTMLDivElement>(null);
  const hovered = useRef(false);
  const hoveredBubble = useRef(false);
  const focused = useRef(false);
  const pinned = useRef(false);
  const dismissed = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [open, setOpen] = useState(false);
  const cancelTimer = useCallback(() => { clearTimeout(timer.current); }, []);
  const close = useCallback(() => {
    cancelTimer(); pinned.current = false; hoveredBubble.current = false;
    setOpen(false);
    if (closeActiveHelp === close) closeActiveHelp = null;
  }, [cancelTimer]);
  const show = () => {
    cancelTimer(); dismissed.current = false;
    if (closeActiveHelp && closeActiveHelp !== close) closeActiveHelp();
    closeActiveHelp = close;
    setOpen(true);
  };
  const closeWhenIdle = () => {
    cancelTimer();
    timer.current = setTimeout(() => {
      if (!hovered.current && !hoveredBubble.current && !focused.current && !pinned.current) close();
    }, 120);
  };

  useLayoutEffect(() => {
    if (!open) return;
    const element = bubble.current;
    const trigger = anchor.current;
    if (!element || !trigger) return;
    // A manual popover escapes overflow clipping and participates in the top layer.
    // Inside a modal it is also portaled into that dialog, preserving its active scope.
    try { element.showPopover?.(); } catch { /* Older browsers retain the fixed portal. */ }
    const position = () => {
      if (!trigger.getClientRects().length) { close(); return; }
      const rect = trigger.getBoundingClientRect();
      if (rect.bottom <= 0 || rect.top >= innerHeight || rect.right <= 0 || rect.left >= innerWidth) { close(); return; }
      const size = element.getBoundingClientRect();
      const edge = 12;
      const left = Math.max(edge, Math.min(rect.left, innerWidth - size.width - edge));
      const below = rect.bottom + 8;
      const above = rect.top - size.height - 8;
      const preferred = below + size.height <= innerHeight - edge ? below : above;
      const top = Math.max(edge, Math.min(preferred, innerHeight - size.height - edge));
      element.style.left = `${left}px`;
      element.style.top = `${top}px`;
    };
    position();
    const outside = (event: Event) => {
      if (event.target instanceof Node && !trigger.contains(event.target) && !element.contains(event.target)) close();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault(); event.stopImmediatePropagation(); close(); dismissed.current = true;
    };
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('click', outside, true);
    document.addEventListener('keydown', escape, true);
    document.addEventListener('toggle', position, true);
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(position);
    observer?.observe(element);
    return () => {
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('click', outside, true);
      document.removeEventListener('keydown', escape, true);
      document.removeEventListener('toggle', position, true);
      window.removeEventListener('resize', position);
      window.removeEventListener('scroll', position, true);
      observer?.disconnect();
    };
  }, [open, close]);

  useLayoutEffect(() => () => {
    cancelTimer();
    if (closeActiveHelp === close) closeActiveHelp = null;
  }, [cancelTimer, close]);

  const container = anchor.current?.closest('dialog[open]') ?? document.body;
  return <>
    <span ref={anchor} className="def" role="button" tabIndex={0} aria-expanded={open} aria-describedby={open ? id : undefined}
      onPointerEnter={() => { hovered.current = true; if (!dismissed.current) show(); }}
      onPointerLeave={() => { hovered.current = false; dismissed.current = false; closeWhenIdle(); }}
      onFocus={() => { focused.current = true; show(); }}
      onBlur={() => { focused.current = false; closeWhenIdle(); }}
      onClick={(event) => { event.preventDefault(); event.stopPropagation(); if (pinned.current) { close(); dismissed.current = true; } else { pinned.current = true; show(); } }}
      onKeyDown={(event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault(); event.stopPropagation();
        if (pinned.current) { close(); dismissed.current = true; } else { pinned.current = true; show(); }
      }}
    >{children}</span>
    {open && createPortal(<div ref={bubble} id={id} className="help-tooltip" role="tooltip" popover="manual"
      onPointerEnter={() => { hoveredBubble.current = true; cancelTimer(); }}
      onPointerLeave={() => { hoveredBubble.current = false; closeWhenIdle(); }}
    >{text}</div>, container)}
  </>;
}

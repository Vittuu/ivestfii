import { useCallback, useId, useLayoutEffect, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { Bell, X } from 'lucide-react';
import type { NotificationItem, NotificationRoute, NotificationTone } from '../lib/notifications';
import '../styles/notifications.css';

export interface NotificationCenterProps {
  items: NotificationItem[];
  unreadCount: number;
  onMarkRead: (id: string) => void;
  onMarkAllRead: () => void;
  onDismiss: (id: string) => void;
  onNavigate: (route: NotificationRoute, conversationId?: string, walletId?: string) => void;
}

const toneLabels: Record<NotificationTone, string> = { info: 'Informação', success: 'Concluído', warning: 'Atenção', error: 'Erro' };
function stamp(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString('pt-BR', { day: '2-digit', month: '2-digit', hour: '2-digit', minute: '2-digit' });
}
function focusable(element: Element): element is HTMLElement {
  return element instanceof HTMLElement && element.tabIndex >= 0 && !element.closest('[inert], [hidden]') && !element.matches(':disabled') && element.getClientRects().length > 0 && getComputedStyle(element).visibility !== 'hidden';
}

export default function NotificationCenter({ items, unreadCount, onMarkRead, onMarkAllRead, onDismiss, onNavigate }: NotificationCenterProps) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const dismissButtons = useRef(new Map<string, HTMLButtonElement>());
  const close = useCallback((restoreFocus: boolean) => {
    setOpen(false);
    if (restoreFocus) requestAnimationFrame(() => {
      if (trigger.current && focusable(trigger.current) && !document.querySelector('dialog[open]')) trigger.current.focus({ preventScroll: true });
    });
  }, []);

  useLayoutEffect(() => {
    if (!open) return;
    const content = panel.current;
    const button = trigger.current;
    if (!content || !button) return;
    const position = () => {
      const anchor = button.getBoundingClientRect();
      const width = content.getBoundingClientRect().width;
      const edge = 12;
      content.style.left = `${Math.max(edge, Math.min(anchor.right - width, innerWidth - width - edge))}px`;
      content.style.top = `${Math.max(edge, Math.min(anchor.bottom + 8, innerHeight - 120))}px`;
      content.style.maxHeight = `${Math.max(100, innerHeight - Math.max(edge, Math.min(anchor.bottom + 8, innerHeight - 120)) - edge)}px`;
    };
    const outside = (event: Event) => {
      if (event.target instanceof Node && !content.contains(event.target) && !button.contains(event.target)) close(false);
    };
    const escape = (event: globalThis.KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault(); event.stopPropagation(); close(true);
    };
    position();
    closeButton.current?.focus({ preventScroll: true });
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('focusin', outside, true);
    document.addEventListener('keydown', escape, true);
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    return () => {
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('focusin', outside, true);
      document.removeEventListener('keydown', escape, true);
      window.removeEventListener('resize', position);
      window.removeEventListener('scroll', position, true);
    };
  }, [open, close]);

  // A nonmodal panel lets Tab leave. Its logical next target follows the bell,
  // rather than jumping to the browser chrome because the portal is last in body.
  const tabOut = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Tab' || !panel.current) return;
    const controls = Array.from(panel.current.querySelectorAll('button, a[href], [tabindex]:not([tabindex="-1"])')).filter(focusable);
    const edge = event.shiftKey ? controls[0] : controls[controls.length - 1];
    if (document.activeElement !== edge) return;
    event.preventDefault();
    close(false);
    if (event.shiftKey) { trigger.current?.focus({ preventScroll: true }); return; }
    const targets = Array.from(document.querySelectorAll('button, a[href], input, select, textarea, [tabindex]:not([tabindex="-1"])')).filter((element) => !panel.current?.contains(element) && focusable(element)) as HTMLElement[];
    const index = targets.indexOf(trigger.current!);
    (targets[index + 1] || trigger.current)?.focus({ preventScroll: true });
  };
  const dismiss = (item: NotificationItem, index: number) => {
    const next = items[index + 1]?.id || items[index - 1]?.id;
    onDismiss(item.id);
    requestAnimationFrame(() => (next ? dismissButtons.current.get(next) : closeButton.current)?.focus({ preventScroll: true }));
  };

  return <>
    <button ref={trigger} className="icon-button notification-trigger" type="button" aria-label="Notificações" aria-haspopup="dialog" aria-expanded={open} aria-controls={open ? `${id}-panel` : undefined} aria-describedby={`${id}-count`} onClick={() => open ? close(false) : setOpen(true)}>
      <Bell size={18} aria-hidden="true" />
      {unreadCount > 0 && <span className="notification-badge" aria-hidden="true">{unreadCount > 99 ? '99+' : unreadCount}</span>}
    </button>
    <span id={`${id}-count`} className="sr-only" aria-live="polite">{unreadCount ? `${unreadCount} ${unreadCount === 1 ? 'notificação não lida' : 'notificações não lidas'}` : 'Nenhuma notificação não lida'}</span>
    {open && createPortal(<div ref={panel} id={`${id}-panel`} className="notification-panel" role="dialog" aria-labelledby={`${id}-title`} onKeyDown={tabOut}>
      <div className="notification-head"><h2 id={`${id}-title`}>Notificações</h2><button ref={closeButton} className="icon-button" type="button" aria-label="Fechar notificações" onClick={() => close(true)}><X size={16} /></button></div>
      {items.length ? <>
        <div className="notification-toolbar"><span>{unreadCount} não {unreadCount === 1 ? 'lida' : 'lidas'}</span><button className="text-link" type="button" disabled={!unreadCount} onClick={() => { onMarkAllRead(); closeButton.current?.focus({ preventScroll: true }); }}>Marcar todas como lidas</button></div>
        <ul className="notification-list">{items.map((item, index) => <li key={item.id} data-notification-id={item.id} data-notification-code={item.code} className={`notification-item notification-${item.tone}${item.readAt ? '' : ' is-unread'}`}>
          <div className="notification-item-head"><h3>{item.title}</h3><button ref={(element) => { if (element) dismissButtons.current.set(item.id, element); else dismissButtons.current.delete(item.id); }} className="icon-button" type="button" aria-label={`Dispensar notificação: ${item.title}`} onClick={() => dismiss(item, index)}><X size={14} /></button></div>
          <p>{item.message}</p>
          <div className="notification-meta"><span className="notification-tone">{toneLabels[item.tone]}</span><time dateTime={item.createdAt} title={new Date(item.createdAt).toLocaleString('pt-BR')}>{stamp(item.createdAt)}</time>{!item.readAt && <span>Não lida</span>}</div>
          <div className="notification-item-actions">{item.action && <button className="text-link" type="button" onClick={() => { onMarkRead(item.id); close(false); onNavigate(item.action!.route, item.action!.conversationId, item.action!.walletId); }}>{item.action.label}</button>}{!item.readAt && <button className="text-link" type="button" aria-label={`Marcar como lida: ${item.title}`} onClick={() => { onMarkRead(item.id); requestAnimationFrame(() => dismissButtons.current.get(item.id)?.focus({ preventScroll: true })); }}>Marcar como lida</button>}</div>
        </li>)}</ul>
      </> : <div className="notification-empty"><p>Nenhuma notificação.</p><span>Os avisos das ações do aplicativo aparecem aqui.</span></div>}
    </div>, document.body)}
  </>;
}

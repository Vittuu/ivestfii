import { createContext, useContext, useEffect, useId, useRef } from 'react';
import type { ReactNode, FormEvent, KeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { X, LoaderCircle } from 'lucide-react';
import HelpTooltip from './components/HelpTooltip';

/** Destino das ações da tela, renderizado na barra superior pelo App. */
export const PageActionsSlot = createContext<HTMLElement | null>(null);

/** Coloca botões de ação da tela na barra superior, ao lado do título. */
export function PageActions({ children }: { children: ReactNode }) {
  const target = useContext(PageActionsSlot);
  return target ? createPortal(children, target) : null;
}

export function Empty({ title, children, action }: { title: string; children: ReactNode; action?: ReactNode }) {
  return <div className="empty-state"><h3>{title}</h3><p>{children}</p>{action}</div>;
}

export function Loading() {
  return <div className="loading" role="status"><LoaderCircle className="spin" size={16} /> Carregando dados…</div>;
}

export function Modal({ title, description, children, onClose, busy = false }: { title: string; description?: string; children: ReactNode; onClose: () => void; busy?: boolean }) {
  const ref = useRef<HTMLDialogElement>(null);
  const id = useId();
  const returnFocus = useRef(document.activeElement instanceof HTMLElement ? document.activeElement : null);
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (!dialog.open) dialog.showModal();
    // showModal() runs after React's autoFocus, so focus the intended field again.
    dialog.querySelector<HTMLElement>('[autofocus], [data-autofocus], form :is(input, select, textarea):not(:disabled)')?.focus();
    return () => {
      if (dialog.open) dialog.close();
      queueMicrotask(() => {
        // StrictMode immediately reopens the same dialog after its trial cleanup.
        if ((dialog.isConnected && dialog.open) || document.querySelector('dialog[open]')) return;
        const target = returnFocus.current;
        if (target?.isConnected && !target.closest('[inert]') && target.getClientRects().length) target.focus({ preventScroll: true });
        else document.getElementById('main-content')?.focus({ preventScroll: true });
      });
    };
  }, []);
  const close = () => { if (!busy) onClose(); };
  return <dialog ref={ref} className="modal" onCancel={(event) => { event.preventDefault(); close(); }} onClick={(event) => { if (event.target === event.currentTarget) close(); }} aria-labelledby={`${id}-title`} aria-describedby={description ? `${id}-description` : undefined} aria-busy={busy || undefined}>
    <div className="modal-inner"><header className="modal-header"><div><h2 id={`${id}-title`}>{title}</h2>{description && <p id={`${id}-description`}>{description}</p>}</div><button className="icon-button" type="button" aria-label="Fechar" onClick={close} disabled={busy}><X size={16} /></button></header>{children}</div>
  </dialog>;
}

export function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: string }) {
  return <label className="field"><span>{hint ? <Def text={hint}>{label}</Def> : label}</span>{children}</label>;
}

export function Submit({ busy, children }: { busy: boolean; children: ReactNode }) {
  return <button className="button primary" type="submit" disabled={busy}>{busy && <LoaderCircle className="spin" size={16} />}{busy ? 'Salvando…' : children}</button>;
}

/** Cabeçalho de bloco: título curto, nota opcional em cinza e ações à direita. */
export function SectionHeading({ title, meta, children, level = 2 }: { title: string; meta?: ReactNode; children?: ReactNode; level?: 2 | 3 }) {
  const Heading = level === 2 ? 'h2' : 'h3';
  return <div className="block-head"><Heading>{title}</Heading>{meta && <span className="block-meta">{meta}</span>}{children && <div className="block-actions">{children}</div>}</div>;
}

/** Rótulo com explicação acessível por mouse, teclado ou toque. */
export function Def({ text, children }: { text: ReactNode; children: ReactNode }) {
  return <HelpTooltip text={text}>{children}</HelpTooltip>;
}

/** Abas de seção. Setas, Home e End trocam de aba; só a aba ativa entra na ordem do Tab. Os painéis usam os ids `${id}-panel-${aba}`. */
export function Tabs<T extends string>({ id, label, tabs, value, onChange }: { id: string; label: string; tabs: { id: T; label: string }[]; value: T; onChange: (tab: T) => void }) {
  const buttons = useRef(new Map<T, HTMLButtonElement>());
  const move = (event: KeyboardEvent<HTMLDivElement>) => {
    const index = tabs.findIndex((tab) => tab.id === value);
    const targets: Partial<Record<string, number>> = { ArrowRight: (index + 1) % tabs.length, ArrowLeft: (index - 1 + tabs.length) % tabs.length, Home: 0, End: tabs.length - 1 };
    const next = targets[event.key];
    if (next === undefined) return;
    event.preventDefault();
    onChange(tabs[next].id);
    buttons.current.get(tabs[next].id)?.focus();
  };
  return <div className="tabs" role="tablist" aria-label={label} onKeyDown={move}>{tabs.map((tab) => <button key={tab.id} ref={(element) => { if (element) buttons.current.set(tab.id, element); else buttons.current.delete(tab.id); }} type="button" role="tab" className="tab" id={`${id}-tab-${tab.id}`} aria-controls={`${id}-panel-${tab.id}`} aria-selected={tab.id === value} tabIndex={tab.id === value ? 0 : -1} onClick={() => onChange(tab.id)}>{tab.label}</button>)}</div>;
}

export function TextLink({ children, onClick }: { children: ReactNode; onClick: () => void }) {
  return <button type="button" className="text-link" onClick={onClick}>{children}</button>;
}

/** Barra fina de participação, escalada pelo maior valor da série. */
export function ShareBar({ value, max, label }: { value: number; max: number; label: string }) {
  const width = max > 0 ? Math.max(0, Math.min(100, value / max * 100)) : 0;
  return <span className="share"><span className="num">{label}</span><span className="share-track" aria-hidden="true"><span className="share-fill" style={{ width: `${width}%` }} /></span></span>;
}

export function handleForm(handler: () => Promise<void>) {
  return (event: FormEvent) => { event.preventDefault(); void handler(); };
}

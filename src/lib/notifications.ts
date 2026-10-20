import { useCallback, useEffect, useState } from 'react';

export type NotificationRoute = 'overview' | 'portfolio' | 'cash' | 'market' | 'projections' | 'assistant' | 'settings';
export type NotificationTone = 'info' | 'success' | 'warning' | 'error';
export type NotificationCode = 'wallet-saved' | 'import-completed' | 'import-warning' | 'export-completed' | 'cash-saved' | 'chat-completed' | 'chat-failed' | 'settings-saved' | 'operation-failed' | 'notice';

export interface NotificationAction {
  label: string;
  route: NotificationRoute;
  /** Kept only in memory; never written to browser storage. */
  conversationId?: string;
  /** Wallet context is also session-only. */
  walletId?: string;
}

export interface NotificationInput {
  id?: string;
  title: string;
  message: string;
  tone?: NotificationTone;
  action?: NotificationAction;
  code?: NotificationCode;
}

export interface NotificationItem {
  id: string;
  title: string;
  message: string;
  tone: NotificationTone;
  action?: NotificationAction;
  code: NotificationCode;
  createdAt: string;
  readAt: string | null;
}

interface NotificationRecord extends NotificationItem { fingerprint: string }
interface Dismissed { id: string; at: string }
interface NotificationState { items: NotificationRecord[]; dismissed: Dismissed[]; writable: boolean }

const STORAGE_KEY = 'ivestfii.notifications.v1';
const MAX_ITEMS = 40;
const MAX_DISMISSED = 80;
const MAX_STORAGE_SIZE = 48 * 1024;
const DUPLICATE_WINDOW_MS = 60_000;
const DISMISS_RETENTION_MS = 90 * 24 * 60 * 60 * 1000;
const routes: NotificationRoute[] = ['overview', 'portfolio', 'cash', 'market', 'projections', 'assistant', 'settings'];
const tones: NotificationTone[] = ['info', 'success', 'warning', 'error'];
const catalog: Record<NotificationCode, { title: string; message: string }> = {
  'wallet-saved': { title: 'Carteira salva', message: 'As informações da carteira foram salvas neste computador.' },
  'import-completed': { title: 'Importação concluída', message: 'Os dados importados estão disponíveis no aplicativo.' },
  'import-warning': { title: 'Confira a importação', message: 'A importação tem observações. Abra a tela relacionada para conferir.' },
  'export-completed': { title: 'Exportação concluída', message: 'A exportação da carteira foi concluída.' },
  'cash-saved': { title: 'Caixinha salva', message: 'O cadastro da caixinha foi salvo neste computador.' },
  'chat-completed': { title: 'Resposta do assistente disponível', message: 'Abra o assistente para consultar a conversa.' },
  'chat-failed': { title: 'O assistente não concluiu a resposta', message: 'Abra o assistente para conferir o estado da conversa.' },
  'settings-saved': { title: 'Preferências salvas', message: 'As preferências do aplicativo foram salvas.' },
  'operation-failed': { title: 'Uma ação não foi concluída', message: 'Abra a tela relacionada para conferir o aviso e tentar novamente.' },
  notice: { title: 'Aviso do aplicativo', message: 'Há um registro de atividade. Abra a tela relacionada para conferir.' },
};
const actionLabels: Record<NotificationRoute, string> = {
  overview: 'Abrir visão geral', portfolio: 'Abrir FIIs e posições', cash: 'Abrir caixinhas', market: 'Abrir Mercado', projections: 'Abrir projeções', assistant: 'Abrir assistente', settings: 'Abrir configurações',
};

function text(value: unknown, maximum: number): string {
  return typeof value === 'string' ? value.replace(/[\u0000-\u0008\u000b-\u001f\u007f]/g, '').trim().slice(0, maximum) : '';
}
function isRoute(value: unknown): value is NotificationRoute { return routes.includes(value as NotificationRoute); }
function isTone(value: unknown): value is NotificationTone { return tones.includes(value as NotificationTone); }
function isCode(value: unknown): value is NotificationCode { return typeof value === 'string' && Object.prototype.hasOwnProperty.call(catalog, value); }
function isObject(value: unknown): value is Record<string, unknown> { return value !== null && typeof value === 'object' && !Array.isArray(value); }
function validTime(value: unknown): value is string {
  if (typeof value !== 'string' || value.length > 32) return false;
  const time = Date.parse(value);
  return Number.isFinite(time) && time >= 0 && time <= Date.now() + 24 * 60 * 60 * 1000;
}
function validId(value: unknown): value is string { return typeof value === 'string' && /^n-(?:e-[a-f0-9]{16}|r-[a-f0-9]{32})$/.test(value); }

/** Only an opaque deduplication key is retained, never the caller's ID or text. */
function fingerprint(value: string): string {
  let hash = 0xcbf29ce484222325n;
  for (let index = 0; index < value.length; index += 1) hash = BigInt.asUintN(64, (hash ^ BigInt(value.charCodeAt(index))) * 0x100000001b3n);
  return hash.toString(16).padStart(16, '0');
}
function newId(): string {
  const random = globalThis.crypto?.randomUUID?.().replace(/-/g, '') ?? Array.from({ length: 4 }, () => Math.floor(Math.random() * 0x100000000).toString(16).padStart(8, '0')).join('');
  return `n-r-${random}`;
}

function readState(): NotificationState {
  const empty: NotificationState = { items: [], dismissed: [], writable: true };
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw || raw.length > MAX_STORAGE_SIZE) return empty;
    const stored: unknown = JSON.parse(raw);
    if (!isObject(stored)) return empty;
    // A future schema belongs to a newer app; do not overwrite it.
    if (stored.version !== 1) return { ...empty, writable: false };
    const dismissed: Dismissed[] = [];
    const now = Date.now();
    if (Array.isArray(stored.dismissed)) for (const item of stored.dismissed.slice(0, MAX_DISMISSED)) {
      if (isObject(item) && validId(item.id) && validTime(item.at) && now - Date.parse(item.at) < DISMISS_RETENTION_MS) dismissed.push({ id: item.id, at: item.at });
    }
    const seen = new Set(dismissed.map((item) => item.id));
    const items: NotificationRecord[] = [];
    if (Array.isArray(stored.items)) for (const item of stored.items.slice(0, MAX_ITEMS)) {
      if (!isObject(item) || !validId(item.id) || seen.has(item.id) || !isCode(item.code) || !isTone(item.tone) || !validTime(item.createdAt) || !(item.readAt === null || validTime(item.readAt)) || typeof item.fingerprint !== 'string' || !/^[a-f0-9]{16}$/.test(item.fingerprint)) continue;
      seen.add(item.id);
      const copy = catalog[item.code];
      items.push({ id: item.id, fingerprint: item.fingerprint, code: item.code, tone: item.tone, createdAt: item.createdAt, readAt: item.readAt, ...copy, ...(isRoute(item.route) ? { action: { route: item.route, label: actionLabels[item.route] } } : {}) });
    }
    items.sort((left, right) => right.createdAt.localeCompare(left.createdAt));
    return { items, dismissed, writable: true };
  } catch { return empty; }
}

/** Stores only fixed catalog codes, route enums, opaque keys and timestamps. */
function persist(state: NotificationState) {
  if (!state.writable) return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify({ version: 1, items: state.items.slice(0, MAX_ITEMS).map((item) => ({ id: item.id, fingerprint: item.fingerprint, code: item.code, tone: item.tone, createdAt: item.createdAt, readAt: item.readAt, ...(item.action ? { route: item.action.route } : {}) })), dismissed: state.dismissed.slice(0, MAX_DISMISSED) }));
  } catch { /* Storage can be blocked or full; notifications still work in memory. */ }
}

export function useNotifications() {
  const [state, setState] = useState<NotificationState>(readState);
  useEffect(() => { persist(state); }, [state]);

  const notify = useCallback((input: NotificationInput) => {
    const code = isCode(input.code) ? input.code : 'notice';
    const tone = isTone(input.tone) ? input.tone : 'info';
    const title = text(input.title, 120) || catalog[code].title;
    const message = text(input.message, 360) || catalog[code].message;
    const action = input.action && isRoute(input.action.route) ? {
      route: input.action.route,
      label: text(input.action.label, 64) || actionLabels[input.action.route],
      ...(input.action.route === 'assistant' && typeof input.action.conversationId === 'string' && /^[a-zA-Z0-9_-]{1,128}$/.test(input.action.conversationId) ? { conversationId: input.action.conversationId } : {}),
      ...(typeof input.action.walletId === 'string' && /^[a-zA-Z0-9_-]{1,128}$/.test(input.action.walletId) ? { walletId: input.action.walletId } : {}),
    } : undefined;
    const key = fingerprint(JSON.stringify([code, tone, title, message, action?.route, action?.conversationId, action?.walletId]));
    const stableId = text(input.id, 512);
    const id = stableId ? `n-e-${fingerprint(stableId)}` : newId();
    const createdAt = new Date().toISOString();
    setState((old) => {
      if (old.dismissed.some((item) => item.id === id) || old.items.some((item) => item.id === id || (!stableId && item.fingerprint === key && Date.parse(createdAt) - Date.parse(item.createdAt) < DUPLICATE_WINDOW_MS))) return old;
      return { ...old, items: [{ id, fingerprint: key, code, title, message, tone, action, createdAt, readAt: null }, ...old.items].slice(0, MAX_ITEMS) };
    });
  }, []);
  const markRead = useCallback((id: string) => {
    setState((old) => old.items.some((item) => item.id === id && !item.readAt) ? { ...old, items: old.items.map((item) => item.id === id && !item.readAt ? { ...item, readAt: new Date().toISOString() } : item) } : old);
  }, []);
  const markAllRead = useCallback(() => {
    const at = new Date().toISOString();
    setState((old) => old.items.some((item) => !item.readAt) ? { ...old, items: old.items.map((item) => item.readAt ? item : { ...item, readAt: at }) } : old);
  }, []);
  const dismiss = useCallback((id: string) => {
    setState((old) => old.items.some((item) => item.id === id) ? { ...old, items: old.items.filter((item) => item.id !== id), dismissed: [{ id, at: new Date().toISOString() }, ...old.dismissed.filter((item) => item.id !== id)].slice(0, MAX_DISMISSED) } : old);
  }, []);

  return { items: state.items as NotificationItem[], unreadCount: state.items.filter((item) => !item.readAt).length, notify, markRead, markAllRead, dismiss };
}

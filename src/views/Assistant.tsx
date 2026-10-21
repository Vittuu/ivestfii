import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { ArrowUpRight, ChevronDown, LoaderCircle, Plus, X } from 'lucide-react';
import { call, desktop, errorMessage } from '../lib/bridge';
import type { ChatMessage, ChatRun, Conversation, Provider, ToolEvent } from '../lib/bridge';
import { Def, Loading, PageActions } from '../components';
import AssistantAnalysisBar from '../components/AssistantAnalysisBar';
import { contributionAnalysisPrompt, investmentAnalysisPrompt } from '../lib/investmentProfile';

const storagePrefix = 'ivestfii.assistant.';
const providerNames: Record<string, string> = { codex: 'Codex', claude: 'Claude', antigravity: 'Antigravity' };
const toolNames: Record<string, string> = {
  carteira_snapshot: 'Consultar carteira',
  projetar_renda: 'Projetar renda',
  recuperar_contexto: 'Recuperar histórico',
  preparar_analise_carteira: 'Preparar análise da carteira',
};
const statusNames: Record<ChatRun['status'], string> = { running: 'Respondendo', completed: 'Concluído', failed: 'Não foi possível concluir', cancelled: 'Resposta interrompida' };
const toolStatusNames: Record<ToolEvent['status'], string> = { running: 'Executando', completed: 'Concluído', failed: 'Falhou', cancelled: 'Cancelado' };

export type AssistantView = 'page' | 'drawer' | 'hidden';
interface Props {
  walletId?: string;
  walletNames?: Record<string, string>;
  view?: AssistantView;
  onOpenPage?: () => void;
  onClose?: () => void;
  requestedConversation?: { id: string; token: number };
  onRunNotice?: (run: ChatRun) => void;
  onBusyChange?: (busy: boolean) => void;
}

function readPreference(key: string): string {
  try { return localStorage.getItem(storagePrefix + key) || ''; } catch { return ''; }
}

function savePreference(key: string, value: string) {
  try { if (value) localStorage.setItem(storagePrefix + key, value); else localStorage.removeItem(storagePrefix + key); } catch { /* Preferences are optional; chat history lives in SQLite. */ }
}

/* Datas curtas em mono. Lista: "21:40" hoje, "12 out" no ano, "10 set 25" antes.
   Mensagens e ferramentas: "21:40" hoje, "15/10, 21:40" em outros dias. */
const pad = (value: number) => String(value).padStart(2, '0');
const monthAbbr = ['jan', 'fev', 'mar', 'abr', 'mai', 'jun', 'jul', 'ago', 'set', 'out', 'nov', 'dez'];
function toDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}
function sameDay(left: Date, right: Date) {
  return left.getFullYear() === right.getFullYear() && left.getMonth() === right.getMonth() && left.getDate() === right.getDate();
}
function clock(date: Date, seconds = false) {
  return `${pad(date.getHours())}:${pad(date.getMinutes())}${seconds ? `:${pad(date.getSeconds())}` : ''}`;
}
function day(date: Date, withYear: 'short' | 'full' | false) {
  const base = `${pad(date.getDate())}/${pad(date.getMonth() + 1)}`;
  if (!withYear) return base;
  return `${base}/${withYear === 'full' ? date.getFullYear() : String(date.getFullYear()).slice(2)}`;
}
function listDate(value: string) {
  const date = toDate(value);
  if (!date) return value;
  const now = new Date();
  if (sameDay(date, now)) return clock(date);
  const base = `${pad(date.getDate())} ${monthAbbr[date.getMonth()]}`;
  return date.getFullYear() === now.getFullYear() ? base : `${base} ${String(date.getFullYear()).slice(2)}`;
}
function stamp(value: string, seconds = false) {
  const date = toDate(value);
  if (!date) return value;
  const now = new Date();
  if (sameDay(date, now)) return clock(date, seconds);
  return `${day(date, date.getFullYear() === now.getFullYear() ? false : 'short')}, ${clock(date, seconds)}`;
}
function fullDate(value: string) {
  const date = toDate(value);
  return date ? `${day(date, 'full')}, ${clock(date)}` : value;
}
function fullDay(value: string) {
  const date = toDate(value);
  return date ? day(date, 'full') : value;
}
function toolDuration(tool: ToolEvent) {
  if (!tool.completedAt) return '';
  const elapsed = Date.parse(tool.completedAt) - Date.parse(tool.startedAt);
  if (!Number.isFinite(elapsed) || elapsed < 0) return '';
  if (elapsed < 1000) return '< 1 s';
  const seconds = Math.round(elapsed / 1000);
  return seconds < 60 ? `${seconds} s` : `${Math.floor(seconds / 60)} min ${seconds % 60} s`;
}

function latestRun(current: ChatRun | null, incoming: ChatRun): ChatRun {
  if (!current) return incoming;
  if (current.id === incoming.id && current.status !== 'running' && incoming.status === 'running') return current;
  if (Date.parse(current.createdAt) > Date.parse(incoming.createdAt) || (current.id === incoming.id && Date.parse(current.updatedAt) > Date.parse(incoming.updatedAt))) return current;
  if (current.id === incoming.id && incoming.status === 'running' && incoming.text.length < current.text.length) return current;
  return incoming;
}

function mergeMessages(current: ChatMessage[], incoming: ChatMessage[]): ChatMessage[] {
  const byId = new Map(current.map((message) => [message.id, message]));
  for (const message of incoming) byId.set(message.id, message);
  return Array.from(byId.values()).sort((left, right) => Date.parse(left.createdAt) - Date.parse(right.createdAt));
}

function mergeTools(current: ToolEvent[], incoming: ToolEvent[]): ToolEvent[] {
  const byId = new Map(current.map((tool) => [tool.id, tool]));
  for (const tool of incoming) {
    const stored = byId.get(tool.id);
    if (!stored || stored.status === 'running' || tool.status !== 'running') byId.set(tool.id, tool);
  }
  return Array.from(byId.values()).sort((left, right) => Date.parse(left.startedAt) - Date.parse(right.startedAt));
}

function formatPayload(value: unknown, maxLength: number) {
  const text = typeof value === 'string' ? value : JSON.stringify(value, null, 2) ?? '';
  return text.length > maxLength ? `${text.slice(0, maxLength)}\n… Prévia limitada; o registro completo permanece no histórico local.` : text;
}

function providerLabel(provider: string, model?: string | null) {
  const name = providerNames[provider] || provider;
  return model ? <Def text={`Modelo utilizado: ${model}.`}>{name}</Def> : name;
}

function Message({ message }: { message: ChatMessage }) {
  return <article className={`message message-${message.role}`}>
    <header className="message-head"><strong>{message.role === 'user' ? 'Você' : message.role === 'system' ? 'Sistema' : 'Assistente'}</strong>
      {message.provider && <span>{providerLabel(message.provider, message.model)}</span>}
      <time className="num" dateTime={message.createdAt} title={fullDate(message.createdAt)}>{stamp(message.createdAt)}</time></header>
    <p className="message-text">{message.content}</p>
  </article>;
}

function ToolCall({ tool }: { tool: ToolEvent }) {
  const shortName = tool.toolName.split(/__|\//).at(-1) || tool.toolName;
  const duration = toolDuration(tool);
  return <details className={`chat-tool tool-${tool.status}`}>
    <summary>
      <span className="chat-tool-name">{toolNames[shortName] || shortName}</span>
      <span className="chat-tool-status">{tool.status === 'running' && <LoaderCircle size={12} className="spin" aria-hidden="true" />}MCP · {toolStatusNames[tool.status]}</span>
      {duration && <span className="chat-tool-duration num" title="Duração">{duration}</span>}
      <time className="num" dateTime={tool.startedAt} title={fullDate(tool.startedAt)}>{stamp(tool.startedAt, true)}</time>
    </summary>
    <dl className="chat-tool-details">
      <div><dt>Ferramenta</dt><dd><code>{tool.toolName}</code></dd></div>
      <div><dt>Entrada</dt><dd><pre>{formatPayload(tool.input, 4000)}</pre></dd></div>
      {tool.output !== null && <div><dt>Resultado</dt><dd><pre>{formatPayload(tool.output, 16000)}</pre></dd></div>}
    </dl>
  </details>;
}

/** Mounted once by App: page, drawer and hidden share this state and the same local history. */
export default function Assistant({ walletId, walletNames, view = 'page', onOpenPage, onClose, requestedConversation, onRunNotice, onBusyChange }: Props) {
  const conversationListId = useId();
  const [initialized, setInitialized] = useState(view !== 'hidden');
  const [showConversations, setShowConversations] = useState(false);
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [selected, setSelected] = useState(() => readPreference('conversation'));
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [tools, setTools] = useState<ToolEvent[]>([]);
  const [run, setRun] = useState<ChatRun | null>(null);
  const [providers, setProviders] = useState<Provider[]>([]);
  const [checkingProviders, setCheckingProviders] = useState(desktop);
  const [provider, setProvider] = useState(() => readPreference('provider') || 'codex');
  const [models, setModels] = useState<Record<string, string>>({});
  const [draft, setDraft] = useState('');
  const [error, setError] = useState('');
  const [sending, setSending] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [loading, setLoading] = useState(false);
  const [context, setContext] = useState('');
  const [inspecting, setInspecting] = useState(false);
  const stageRef = useRef<HTMLDivElement>(null);
  const draftRef = useRef<HTMLTextAreaElement>(null);
  const previewRef = useRef<HTMLDetailsElement>(null);
  const connectionRef = useRef<HTMLDetailsElement>(null);
  const selectedRef = useRef(selected);
  const selectedHandlers = useRef<{ conversationId: string; applyRun: (snapshot: ChatRun) => void; refresh: () => Promise<void> } | null>(null);
  const selectionVersion = useRef(0);
  const sendGuard = useRef(false);
  const nearBottom = useRef(true);
  const mounted = useRef(true);
  const observedRuns = useRef(new Map<string, ChatRun>());
  const watchedRuns = useRef(new Set<string>());
  const notifiedRuns = useRef(new Set<string>());
  const runNoticeRef = useRef(onRunNotice);
  const busyChangeRef = useRef(onBusyChange);
  const requestedToken = useRef<number | undefined>(undefined);
  runNoticeRef.current = onRunNotice;
  busyChangeRef.current = onBusyChange;

  useEffect(() => { if (view !== 'hidden') setInitialized(true); }, [view]);

  const observeRun = useCallback((incoming: ChatRun, startedHere = false) => {
    const previous = observedRuns.current.get(incoming.id) ?? null;
    const latest = latestRun(previous, incoming);
    observedRuns.current.set(incoming.id, latest);
    if (latest.status === 'running' || startedHere) watchedRuns.current.add(incoming.id);
    if (latest.status !== 'running' && watchedRuns.current.has(latest.id) && !notifiedRuns.current.has(latest.id)) {
      notifiedRuns.current.add(latest.id);
      runNoticeRef.current?.(latest);
    }
  }, []);

  const selectConversation = useCallback((id: string) => {
    selectionVersion.current += 1;
    selectedRef.current = id;
    setSelected(id);
    savePreference('conversation', id);
    setError('');
  }, []);

  useEffect(() => {
    if (!requestedConversation || requestedToken.current === requestedConversation.token) return;
    requestedToken.current = requestedConversation.token;
    selectConversation(requestedConversation.id);
    setShowConversations(false);
  }, [requestedConversation, selectConversation]);

  const refreshList = useCallback(async () => {
    const list = await call<Conversation[]>('list_conversations');
    if (mounted.current) setConversations((current) => {
      const byId = new Map(current.map((item) => [item.id, item]));
      for (const item of list) {
        const stored = byId.get(item.id);
        if (!stored || Date.parse(item.updatedAt) >= Date.parse(stored.updatedAt)) byId.set(item.id, item);
      }
      return Array.from(byId.values()).sort((left, right) => Date.parse(right.updatedAt) - Date.parse(left.updatedAt));
    });
    return list;
  }, []);

  useEffect(() => {
    let cancelled = false;
    const initialSelectionVersion = selectionVersion.current;
    mounted.current = true;
    if (!desktop || !initialized) return () => { mounted.current = false; };
    void refreshList().then((list) => {
      if (cancelled || !mounted.current || selectionVersion.current !== initialSelectionVersion) return;
      const saved = selectedRef.current;
      if (!saved || !list.some((item) => item.id === saved)) selectConversation(list[0]?.id || '');
    }).catch((err) => { if (!cancelled && mounted.current) setError(errorMessage(err)); });
    void call<Provider[]>('get_providers').then((result) => {
      if (cancelled || !mounted.current) return;
      setProviders(result);
      const saved = readPreference('provider');
      setProvider(result.some((item) => item.id === saved) ? saved : result.find((item) => item.ready ?? item.installed)?.id || result[0]?.id || 'codex');
    }).catch((err) => { if (!cancelled && mounted.current) setError(errorMessage(err)); }).finally(() => { if (!cancelled && mounted.current) setCheckingProviders(false); });
    return () => { cancelled = true; mounted.current = false; };
  }, [initialized, refreshList, selectConversation]);

  useEffect(() => {
    if (!desktop || !initialized) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen<ChatRun>('chat-run', (event) => {
      if (cancelled) return;
      observeRun(event.payload);
      const handlers = selectedHandlers.current;
      if (event.payload.conversationId === selectedRef.current && handlers?.conversationId === selectedRef.current) {
        handlers.applyRun(event.payload);
        if (event.payload.status !== 'running') void handlers.refresh();
      } else if (event.payload.status !== 'running') {
        void refreshList().catch(() => { /* Other conversations never replace the selected transcript. */ });
      }
    }).then((cleanup) => { if (cancelled) cleanup(); else unlisten = cleanup; }).catch((err) => {
      if (!cancelled && mounted.current) setError(errorMessage(err));
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [initialized, observeRun, refreshList]);

  useEffect(() => {
    let cancelled = false;
    let refreshing = false;
    let refreshedTerminal = '';
    setContext(''); setMessages([]); setTools([]); setRun(null); setCancelling(false);
    nearBottom.current = true;
    if (!selected || !desktop || !initialized) { setLoading(false); return; }
    setLoading(true);

    const applyRun = (snapshot: ChatRun | null) => {
      if (cancelled || selectedRef.current !== selected || (snapshot && snapshot.conversationId !== selected)) return;
      if (snapshot) observeRun(snapshot);
      setRun((current) => snapshot ? latestRun(current, snapshot) : current);
      if (snapshot && snapshot.status !== 'running' && refreshedTerminal !== snapshot.id) {
        refreshedTerminal = snapshot.id;
        void refreshList().catch(() => { /* The next list refresh can retry independently. */ });
      }
    };
    const refresh = async () => {
      if (!selected || cancelled || refreshing) return;
      refreshing = true;
      const results = await Promise.allSettled([
        call<ChatMessage[]>('get_messages', { conversationId: selected }),
        call<ToolEvent[]>('get_tool_events', { conversationId: selected }),
        call<ChatRun | null>('get_chat_run', { conversationId: selected }),
      ]);
      if (!cancelled && selectedRef.current === selected) {
        if (results[0].status === 'fulfilled') { const incoming = results[0].value; setMessages((current) => mergeMessages(current, incoming)); }
        if (results[1].status === 'fulfilled') { const incoming = results[1].value; setTools((current) => mergeTools(current, incoming)); }
        if (results[2].status === 'fulfilled') applyRun(results[2].value);
        const failure = results.find((result) => result.status === 'rejected');
        if (failure?.status === 'rejected') setError(errorMessage(failure.reason));
        setLoading(false);
      }
      refreshing = false;
    };
    const handlers = { conversationId: selected, applyRun, refresh };
    selectedHandlers.current = handlers;
    void refresh();
    const interval = window.setInterval(() => { void refresh(); }, 1000);
    return () => {
      cancelled = true;
      if (selectedHandlers.current === handlers) selectedHandlers.current = null;
      window.clearInterval(interval);
    };
  }, [initialized, selected, refreshList, observeRun]);

  useEffect(() => {
    if (nearBottom.current && stageRef.current) stageRef.current.scrollTop = stageRef.current.scrollHeight;
  }, [view, messages, tools, run?.text, run?.status]);

  useEffect(() => { if (run?.status !== 'running') setCancelling(false); }, [run?.status]);

  /* A prévia do contexto fica no fim da transcrição; traz para a vista mesmo se o usuário rolou para cima. */
  useEffect(() => { if (context && view !== 'hidden') previewRef.current?.scrollIntoView({ block: 'nearest' }); }, [context, view]);

  const sendQuestion = async (preparedContent?: string) => {
    const content = (preparedContent ?? draft).trim();
    if (!desktop || !providerReady || loading || !content || sendGuard.current || run?.status === 'running') return;
    sendGuard.current = true; setSending(true); setError(''); setContext('');
    let id = selected;
    try {
      if (!id) {
        const conversation = await call<Conversation>('create_conversation', { title: content.slice(0, 64) });
        id = conversation.id;
        if (mounted.current) selectConversation(id);
      }
      const started = await call<ChatRun>('start_chat_turn', { conversationId: id, walletId: walletId || undefined, provider, model: models[provider]?.trim() || null, content });
      if (!mounted.current) return;
      observeRun(started, true);
      if (preparedContent === undefined) setDraft('');
      if (selectedRef.current === id) {
        setRun((current) => latestRun(current, started));
        const updated = await call<ChatMessage[]>('get_messages', { conversationId: id });
        if (mounted.current && selectedRef.current === id) setMessages((current) => mergeMessages(current, updated));
      }
      await refreshList();
    } catch (err) { if (mounted.current) { if (id) selectConversation(id); setError(errorMessage(err)); } }
    finally { sendGuard.current = false; if (mounted.current) setSending(false); }
  };

  const cancel = async () => {
    if (!run || run.status !== 'running' || cancelling) return;
    const conversationId = run.conversationId;
    setCancelling(true); setError('');
    try { await call('cancel_chat_turn', { runId: run.id }); }
    catch (err) { if (mounted.current && selectedRef.current === conversationId) { setError(errorMessage(err)); setCancelling(false); } }
  };

  const inspectContext = async () => {
    if (!selected || inspecting) return;
    const id = selected;
    setInspecting(true);
    try {
      const result = await call<unknown>('build_chat_context', { conversationId: id, maxChars: 18000 });
      if (mounted.current && selectedRef.current === id) setContext(JSON.stringify(result, null, 2));
    } catch (err) { if (mounted.current && selectedRef.current === id) setError(errorMessage(err)); }
    finally { if (mounted.current) setInspecting(false); }
  };

  const newConversation = () => { selectConversation(''); setDraft(''); };
  const planContributions = () => {
    if (!walletId) return;
    const prompt = contributionAnalysisPrompt();
    setDraft((current) => current.includes(prompt) ? current : current.trim() ? `${current.trimEnd()}\n\n${prompt}` : prompt);
    window.requestAnimationFrame(() => draftRef.current?.focus());
  };
  const selectedProvider = providers.find((item) => item.id === provider);
  const providerReady = selectedProvider?.ready ?? selectedProvider?.installed ?? false;
  const active = run?.status === 'running';
  useEffect(() => { busyChangeRef.current?.(active || sending); }, [active, sending]);
  const persistedResponse = run && messages.some((message) => message.role === 'assistant' && message.provider === run.provider && message.content === run.text && Date.parse(message.createdAt) >= Date.parse(run.createdAt));
  const visibleRun = run && !persistedResponse && (active || !!run.text);
  const timeline = messages.map((message) => ({ id: `message-${message.id}`, date: message.createdAt, content: <Message message={message} /> }))
    .sort((left, right) => Date.parse(left.date) - Date.parse(right.date));
  const current = conversations.find((item) => item.id === selected);
  const cliState = checkingProviders ? 'checking' : providerReady ? 'ready' : 'missing';
  const cliTitle = checkingProviders ? 'Conferindo as versões das CLIs instaladas.' : `${selectedProvider?.error || [selectedProvider?.version, selectedProvider?.executable].filter(Boolean).join(' · ') || 'CLI não detectada'}\nRequer CLI autenticada na sua assinatura.`;
  const providerName = providerNames[provider] || selectedProvider?.name || provider;
  const runName = providerNames[run?.provider || ''] || run?.provider;
  const toolsNote = 'MCP: carteira, projeções, histórico e Mercado (só leitura)';
  const notice = !desktop ? 'Conversa disponível no aplicativo Windows'
    : active ? (run?.text && run.statusMessage) || `${runName} respondendo`
    : checkingProviders ? 'Verificando conexão…'
    : !providerReady ? `Requer a CLI do ${providerName} autenticada na sua assinatura`
    : 'Pronto para conversar';

  const chooseConversation = (id: string) => {
    selectConversation(id);
    setShowConversations(false);
    if (view === 'drawer') window.requestAnimationFrame(() => draftRef.current?.focus({ preventScroll: true }));
  };
  const responseWallet = run?.walletId && run.walletId !== walletId ? walletNames?.[run.walletId] : undefined;

  return <div className={`page assistant-page${view === 'drawer' ? ' assistant-drawer' : ''}`} data-assistant-view={view} hidden={view === 'hidden'}>
    {view === 'page' && <PageActions><button type="button" className="button" onClick={newConversation} disabled={sending}><Plus size={16} />Nova conversa</button></PageActions>}
    {view === 'drawer' && <header className="assistant-drawer-head">
      <h2>Assistente</h2><div className="assistant-drawer-actions">
        <button type="button" className="icon-button" onClick={newConversation} disabled={sending} aria-label="Nova conversa" title="Nova conversa"><Plus size={17} /></button>
        {onOpenPage && <button type="button" className="icon-button" onClick={onOpenPage} aria-label="Abrir Assistente na página" title="Abrir na página"><ArrowUpRight size={17} /></button>}
        {onClose && <button type="button" className="icon-button" data-assistant-close onClick={onClose} aria-label="Fechar assistente" title="Fechar assistente"><X size={17} /></button>}
      </div>
    </header>}
    {view === 'drawer' && <div className="assistant-drawer-history"><button type="button" className="text-link" aria-expanded={showConversations} aria-controls={conversationListId} onClick={() => { if (!showConversations && connectionRef.current) connectionRef.current.open = false; setShowConversations((current) => !current); }}>Conversas{conversations.length ? ` (${conversations.length})` : ''}<ChevronDown size={14} aria-hidden="true" /></button><span>Mesmo histórico da aba Assistente</span></div>}
    <div className="modular assistant-layout">
      <aside id={conversationListId} className="block conversation-panel" aria-label="Conversas" hidden={view === 'drawer' && !showConversations}>
        <div className="block-head"><h2>Conversas</h2>{conversations.length > 0 && <span className="block-meta num">{conversations.length}</span>}</div>
        <div className="conversation-list">{conversations.length ? <ul>{conversations.map((item) => <li key={item.id}>
          <button type="button" disabled={sending} className={`conversation-item ${selected === item.id ? 'active' : ''}`} onClick={() => chooseConversation(item.id)} aria-current={selected === item.id ? 'true' : undefined} title={item.title}>
            <span className="conversation-title">{item.title}</span>
            <time className="num" dateTime={item.updatedAt} title={fullDate(item.updatedAt)}>{listDate(item.updatedAt)}</time>
          </button>
        </li>)}</ul> : <p className="conversation-empty">Nenhuma conversa ainda. A primeira pergunta abre uma.</p>}</div>
        <p className="block-foot">{desktop ? <Def text="O histórico desta conversa é salvo neste computador e acompanha a troca de provedor.">Histórico local</Def> : 'Disponível no aplicativo Windows'}</p>
      </aside>

      <section className="block chat-panel" aria-label="Chat com assistente">
        <div className="block-head chat-head">
          <h2 title={current?.title}>{current?.title || 'Nova conversa'}</h2>
        </div>
        {run?.walletId && run.walletId !== walletId && <p className="chat-wallet-notice" role="status">Esta resposta se refere {responseWallet ? <>à carteira <strong>{responseWallet}</strong></> : 'à carteira selecionada no envio'}. Novas perguntas usam a carteira selecionada agora.</p>}
        <AssistantAnalysisBar walletId={walletId} walletName={walletId ? walletNames?.[walletId] : undefined} view={view} disabled={active || sending || loading || !providerReady} onAnalyze={() => { if (walletId) void sendQuestion(investmentAnalysisPrompt()); }} onPlanContributions={planContributions} />

        <div className="chat-toolbar">
          <div className="chat-field"><Def text="Escolha a CLI autenticada na sua assinatura. A conversa mantém seu histórico ao trocar de provedor.">Provedor</Def>
            <span className="chat-select"><select aria-label="Provedor de IA" value={provider} disabled={active || sending} onChange={(event) => { setProvider(event.target.value); savePreference('provider', event.target.value); setError(''); }}>
              {providers.length ? providers.map((item) => <option key={item.id} value={item.id}>{item.name}{item.installed ? item.ready === false ? ' · CLI indisponível' : '' : ' · sem CLI'}</option>) : Object.entries(providerNames).map(([id, name]) => <option value={id} key={id}>{name}</option>)}
            </select></span>
          </div>
          <details ref={connectionRef} className="extra-details chat-connection" onToggle={(event) => { if (view === 'drawer' && event.currentTarget.open) setShowConversations(false); }}><summary>Detalhes da conexão</summary><div className="chat-connection-body">
          <div className="chat-field model-input"><Def text="Identificador de modelo aceito pelo provedor escolhido. Deixe vazio para usar o padrão da CLI.">Modelo</Def><input type="text" value={models[provider] || ''} disabled={active || sending} onChange={(event) => setModels((old) => ({ ...old, [provider]: event.target.value }))} placeholder="Padrão do provedor" aria-label="Modelo do provedor" maxLength={120} /></div>
          <span className={`cli-status cli-${cliState}`} title={cliTitle}>
            {checkingProviders && <LoaderCircle size={12} className="spin" aria-hidden="true" />}
            {checkingProviders ? 'Verificando CLIs…' : providerReady ? 'CLI disponível' : selectedProvider?.installed ? 'CLI indisponível' : 'CLI não detectada'}
            {providerReady && selectedProvider?.version && <span className="num">{selectedProvider.version}</span>}
          </span>
          <p className="chat-connection-note">{toolsNote}. As fontes públicas são consultadas automaticamente quando necessário, com limites; você também pode pedir a consulta pelo chat. Cotações exigem chave brapi; informes exigem vínculo CNPJ.</p>
          <p className="chat-connection-note">{cliTitle}</p>
          {current && <p className="chat-connection-note">Conversa iniciada em <time className="num" dateTime={current.createdAt}>{fullDay(current.createdAt)}</time></p>}
          <button type="button" className="text-link" onClick={() => void inspectContext()} disabled={!selected || inspecting || sending}>{inspecting && <LoaderCircle size={14} className="spin" />}Ver contexto</button>
          </div></details>
        </div>

        <div className="chat-stage" ref={stageRef} onScroll={() => { const stage = stageRef.current; if (stage) nearBottom.current = stage.scrollHeight - stage.scrollTop - stage.clientHeight < 100; }}>
          {loading ? <Loading /> : timeline.length || visibleRun ? <div className="messages">
            {timeline.map((item) => <div key={item.id} className="chat-timeline-item timeline-message">{item.content}</div>)}
            {visibleRun && run && <article className={`message message-assistant message-streaming ${active ? 'is-active' : ''}`}>
              <header className="message-head"><strong>Assistente</strong><span>{providerLabel(run.provider, run.model)}</span><time className="num" dateTime={run.createdAt} title={fullDate(run.createdAt)}>{stamp(run.createdAt)}</time></header>
              {run.text ? <p className="message-text">{run.text}{active && <span className="stream-caret" aria-hidden="true" />}</p> : <p className="response-pending"><LoaderCircle size={14} className="spin" />{run.statusMessage || 'Conectando ao provedor e preparando o contexto…'}</p>}
              {!active && run.status !== 'completed' && <small className="partial-label">Resposta parcial</small>}
            </article>}
          </div> : <div className="chat-empty">
            <h3>{selected ? 'Nenhuma mensagem nesta conversa' : 'Nenhuma mensagem ainda'}</h3>
            <p>{desktop ? 'Pergunte sobre a carteira, os proventos ou uma projeção. O histórico acompanha a troca de provedor.' : 'Abra o aplicativo Windows para conversar com seu provedor e acessar a carteira local.'}</p>
          </div>}
          {run && run.status !== 'running' && run.status !== 'completed' && <div className={`alert ${run.status === 'failed' ? 'error' : 'warning'} chat-run-notice`} role="status"><span><strong>{statusNames[run.status]}.</strong> {run.error || (run.status === 'cancelled' ? 'O histórico foi mantido. Você pode continuar com qualquer provedor.' : 'Confira a autenticação da CLI e tente novamente.')}</span></div>}
          {tools.length > 0 && <details className="extra-details tool-audit"><summary>Atividade das ferramentas<span className="tool-audit-count">{tools.length}{tools.some((tool) => tool.status === 'failed') ? ' · com falhas' : ''}</span></summary><div className="chat-tool-list">{tools.map((tool) => <ToolCall tool={tool} key={tool.id} />)}</div></details>}
          {context && <details className="context-preview" open ref={previewRef}><summary>Histórico local preparado para contexto · prévia</summary><pre>{context}</pre></details>}
        </div>

        <div className="chat-compose">
          {error && <p className="form-error" role="alert">{error}</p>}
          {selectedProvider?.error && !providerReady && <p className="form-error" role="alert">{selectedProvider.error}</p>}
          <form onSubmit={(event) => { event.preventDefault(); void sendQuestion(); }}>
            <label className="sr-only" htmlFor="chat-draft">Sua pergunta</label><textarea ref={draftRef} id="chat-draft" aria-keyshortcuts="Control+Enter" value={draft} onChange={(event) => setDraft(event.target.value)} onKeyDown={(event) => { if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) { event.preventDefault(); if (providerReady && !active && !sending) void sendQuestion(); } }} placeholder={desktop ? 'Pergunte sobre sua carteira ou peça uma projeção…' : 'Abra o aplicativo Windows para conversar.'} rows={3} disabled={!desktop || sending} maxLength={30000} />
            <div className="compose-bar">
              <p className={`connector-notice ${active ? 'connector-active' : ''}`} role="status">{notice}</p>
              {desktop && !active && providerReady && <span className="compose-hint" title="Ctrl + Enter para enviar"><kbd>Ctrl</kbd> + <kbd>Enter</kbd><span className="hint-text"> para enviar</span></span>}
              {active ? <button type="button" className="button" disabled={cancelling} onClick={() => void cancel()}>{cancelling && <LoaderCircle size={16} className="spin" />}{cancelling ? 'Interrompendo…' : 'Interromper'}</button> : <button className="button primary" disabled={!desktop || !providerReady || !draft.trim() || sending || loading}>{sending && <LoaderCircle className="spin" size={16} />}{sending ? 'Enviando…' : 'Enviar'}</button>}
            </div>
          </form>
        </div>
      </section>
    </div>
  </div>;
}

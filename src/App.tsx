import { useCallback, useEffect, useRef, useState } from 'react';
import { Building2, ChartNoAxesCombined, Database, FolderOpen, House, Landmark, LoaderCircle, Menu, MessageSquare, PanelLeftClose, PanelLeftOpen, Pencil, RefreshCw, Settings as SettingsIcon, TrendingUp, Wallet, X, type LucideIcon } from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import { call, desktop, emptySnapshot, errorMessage, money } from './lib/bridge';
import type { AppInfo, BrokerPosition, ChatRun, PortfolioWallet, Snapshot, WealthOverview } from './lib/bridge';
import { Def, Loading, Modal, Field, handleForm, PageActionsSlot, Submit } from './components';
import Overview from './views/Overview';
import Portfolio from './views/Portfolio';
import Projections from './views/Projections';
import Assistant from './views/Assistant';
import Settings from './views/Settings';
import CashPockets from './views/CashPockets';
import Market from './views/Market';
import { startUpdateActivityMonitor } from './lib/updateActivity';
import NotificationCenter from './components/NotificationCenter';
import { useNotifications } from './lib/notifications';
import type { NotificationRoute } from './lib/notifications';
import { PublicDataProvider, usePublicDataController } from './lib/publicData';
import Onboarding from './components/Onboarding';
import type { OnboardingDestination } from './components/Onboarding';
import { onboardingHelp } from './lib/onboarding';
import type { OnboardingStatus } from './lib/onboarding';

type Page = 'overview' | 'portfolio' | 'cash' | 'market' | 'projections' | 'assistant' | 'settings';
const navigation: { id: Page; label: string; icon: LucideIcon }[] = [
  { id: 'overview', label: 'Visão geral', icon: House },
  { id: 'portfolio', label: 'FIIs e posições', icon: Building2 },
  { id: 'cash', label: 'Caixinhas CDI', icon: Landmark },
  { id: 'market', label: 'Mercado', icon: ChartNoAxesCombined },
  { id: 'projections', label: 'Projeções', icon: TrendingUp },
  { id: 'assistant', label: 'Assistente', icon: MessageSquare },
];
const titles: Record<Page, string> = { overview: 'Visão geral', portfolio: 'FIIs e posições', cash: 'Caixinhas CDI', market: 'Mercado', projections: 'Projeções', assistant: 'Assistente', settings: 'Configurações' };
const selectionKey = 'ivestfii.selectedWalletId';
function savedWalletId() { try { return window.localStorage.getItem(selectionKey) || ''; } catch { return ''; } }

export default function App() {
  useEffect(startUpdateActivityMonitor, []);
  const [page, setPage] = useState<Page>('overview');
  const [portfolioInitialTab, setPortfolioInitialTab] = useState<'position' | 'history' | undefined>(undefined);
  const [snapshot, setSnapshot] = useState<Snapshot>(emptySnapshot);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [wallets, setWallets] = useState<PortfolioWallet[]>([]);
  const [walletId, setWalletId] = useState('');
  const [wealth, setWealth] = useState<WealthOverview | null>(null);
  const selectedId = useRef(savedWalletId());
  const requestId = useRef(0);
  const [walletOpen, setWalletOpen] = useState(false);
  const [editingWallet, setEditingWallet] = useState<PortfolioWallet | null>(null);
  const [loading, setLoading] = useState(desktop);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [importOpen, setImportOpen] = useState(false);
  const [importPath, setImportPath] = useState('');
  const [importing, setImporting] = useState(false);
  const [importError, setImportError] = useState('');
  const [positionImportOpen, setPositionImportOpen] = useState(false);
  const [positionImportPath, setPositionImportPath] = useState('');
  const [positionImporting, setPositionImporting] = useState(false);
  const [positionImportError, setPositionImportError] = useState('');
  const [mobileNav, setMobileNav] = useState(false);
  const [actionsSlot, setActionsSlot] = useState<HTMLDivElement | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(true);
  const [smallScreen, setSmallScreen] = useState(() => window.matchMedia('(max-width: 760px)').matches);
  const [assistantStarted, setAssistantStarted] = useState(false);
  const [assistantOpen, setAssistantOpen] = useState(false);
  const [assistantBusy, setAssistantBusy] = useState(false);
  const [requestedConversation, setRequestedConversation] = useState<{ id: string; token: number }>();
  const [requestedDraft, setRequestedDraft] = useState<{ text: string; token: number }>();
  const [onboarding, setOnboarding] = useState<{ walletId: string; step: number } | null>(null);
  const onboardingChecked = useRef(false);
  const onboardingStarting = useRef(false);
  const assistantHost = useRef<HTMLDivElement>(null);
  const assistantTrigger = useRef<HTMLElement | null>(null);
  const assistantFab = useRef<HTMLButtonElement>(null);
  const notifications = useNotifications();
  const { notify: addNotification } = notifications;
  const assistantView = page === 'assistant' ? 'page' : assistantOpen ? 'drawer' : 'hidden';
  const drawerOpen = assistantView === 'drawer';
  const modalDrawer = drawerOpen && smallScreen;
  const publicData = usePublicDataController(walletId, snapshot, !loading);

  useEffect(() => {
    const media = window.matchMedia('(max-width: 760px)');
    const update = () => setSmallScreen(media.matches);
    media.addEventListener('change', update);
    return () => media.removeEventListener('change', update);
  }, []);
  useEffect(() => {
    if (!modalDrawer) return;
    const overflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => { document.body.style.overflow = overflow; };
  }, [modalDrawer]);

  const closeAssistant = useCallback(() => {
    setAssistantOpen(false);
    requestAnimationFrame(() => {
      const target = assistantTrigger.current;
      if (target?.isConnected && target.getClientRects().length && !target.closest('[inert]')) target.focus({ preventScroll: true });
      else assistantFab.current?.focus({ preventScroll: true });
    });
  }, []);
  useEffect(() => {
    if (!drawerOpen) return;
    const frame = requestAnimationFrame(() => assistantHost.current?.querySelector<HTMLElement>('[data-assistant-focus], button:not(:disabled), textarea:not(:disabled)')?.focus({ preventScroll: true }));
    const onKey = (event: KeyboardEvent) => {
      if (document.querySelector('dialog[open]') || event.defaultPrevented) return;
      if (event.key === 'Escape' && (modalDrawer || assistantHost.current?.contains(document.activeElement))) {
        event.preventDefault(); closeAssistant();
      }
      if (event.key !== 'Tab' || !modalDrawer) return;
      const focusable = Array.from(assistantHost.current?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], summary, [tabindex="0"]') || []).filter((element) => element.getClientRects().length && !element.closest('[hidden], [inert]'));
      const first = focusable[0], last = focusable.at(-1);
      if (!first || !last) return;
      if (event.shiftKey && (document.activeElement === first || !assistantHost.current?.contains(document.activeElement))) { event.preventDefault(); last.focus(); }
      else if (!event.shiftKey && (document.activeElement === last || !assistantHost.current?.contains(document.activeElement))) { event.preventDefault(); first.focus(); }
    };
    document.addEventListener('keydown', onKey);
    return () => { cancelAnimationFrame(frame); document.removeEventListener('keydown', onKey); };
  }, [drawerOpen, modalDrawer, closeAssistant]);

  const notify = useCallback((message: string) => {
    setNotice(message);
    const code = /exportad/i.test(message) ? 'export-completed' : /importa|posição completa salva/i.test(message) ? 'import-completed' : page === 'cash' ? 'cash-saved' : page === 'settings' ? 'settings-saved' : 'wallet-saved';
    const title = code === 'export-completed' ? 'Backup exportado' : code === 'import-completed' ? 'Importação concluída' : code === 'cash-saved' ? 'Caixinha atualizada' : code === 'settings-saved' ? 'Configurações salvas' : 'Carteira atualizada';
    addNotification({ code, title, message, tone: 'success', action: { label: 'Abrir página', route: page, walletId: walletId || undefined } });
  }, [addNotification, page, walletId]);
  const onRunNotice = useCallback((run: ChatRun) => {
    if (run.status !== 'completed' && run.status !== 'failed') return;
    addNotification({ id: `chat:${run.id}:${run.status}`, code: run.status === 'completed' ? 'chat-completed' : 'chat-failed', title: run.status === 'completed' ? 'Resposta do assistente pronta' : 'O assistente precisa de atenção', message: run.status === 'completed' ? 'A resposta foi salva. Abra a conversa para continuar.' : 'Não foi possível concluir a resposta. Abra a conversa para conferir os detalhes.', tone: run.status === 'completed' ? 'success' : 'warning', action: { label: 'Abrir conversa', route: 'assistant', conversationId: run.conversationId } });
  }, [addNotification]);

  const refresh = useCallback(async (options?: { propagateError: boolean }) => {
    if (!desktop) return;
    const request = ++requestId.current;
    setLoading(true); setError('');
    try {
      const available = await call<PortfolioWallet[]>('get_wallets');
      if (request !== requestId.current) return;
      const nextId = available.find((wallet) => wallet.id === selectedId.current)?.id || available.find((wallet) => wallet.isDefault)?.id || available[0]?.id;
      if (!nextId) throw new Error('Nenhuma carteira disponível. Reabra o aplicativo para verificar o armazenamento local.');
      selectedId.current = nextId; setWalletId(nextId); setWallets(available);
      try { window.localStorage.setItem(selectionKey, nextId); } catch { /* Selection remains available without browser storage. */ }
      const [portfolioResult, infoResult, wealthResult] = await Promise.allSettled([call<Snapshot>('get_portfolio', { walletId: nextId }), call<AppInfo>('get_app_info', { walletId: nextId }), call<WealthOverview>('get_wealth_overview')]);
      if (request !== requestId.current || selectedId.current !== nextId) return;
      if (portfolioResult.status === 'fulfilled') setSnapshot(portfolioResult.value);
      else setError(errorMessage(portfolioResult.reason));
      if (infoResult.status === 'fulfilled') {
        setInfo(infoResult.value);
        if (available.find((wallet) => wallet.id === nextId)?.isDefault) {
          setImportPath((path) => path || infoResult.value.legacyPath || '');
          setPositionImportPath((path) => path || infoResult.value.positionPath || '');
        }
      } else setError((current) => current || errorMessage(infoResult.reason));
      if (wealthResult.status === 'fulfilled') setWealth(wealthResult.value);
      else { setWealth(null); setError((current) => current || errorMessage(wealthResult.reason)); }
      if (options?.propagateError) {
        for (const result of [portfolioResult, infoResult, wealthResult]) {
          if (result.status === 'rejected') throw result.reason;
        }
      }
    } catch (err) { if (request === requestId.current) setError(errorMessage(err)); if (options?.propagateError) throw err; }
    finally { if (request === requestId.current) setLoading(false); }
  }, []);
  const refreshAfterPurchase = useCallback(async (_savedWalletId: string) => {
    // Refresh the currently open wallet and the combined overview. Confirming a
    // historical scenario in another wallet must not silently change selection.
    await refresh({ propagateError: true });
  }, [refresh]);
  useEffect(() => { void refresh(); return () => { requestId.current += 1; }; }, [refresh]);
  useEffect(() => {
    if (!desktop || loading || !info || error || onboardingChecked.current) return;
    onboardingChecked.current = true;
    void call<OnboardingStatus>('get_onboarding_status').then(async status => {
      if (!status.autoShow || info.migration?.error || info.positionImport?.error) return;
      await call('save_onboarding_progress', { progress: { version: 1, walletId: status.walletId, step: status.step, status: 'started' } });
      setOnboarding({ walletId: status.walletId, step: status.step });
    }).catch(() => { /* Setup remains available in Settings; portfolio access is independent. */ });
  }, [loading, info, error]);
  useEffect(() => {
    if (!desktop) return;
    const unlisten = listen<string>('navigate', (event) => { if (event.payload === 'settings') { setPage('settings'); setMobileNav(false); } });
    return () => { void unlisten.then((cleanup) => cleanup()); };
  }, []);
  useEffect(() => { document.title = `${titles[page]} · IvestFII`; }, [page]);
  useEffect(() => { if (!notice) return; const timer = window.setTimeout(() => setNotice(''), 6500); return () => window.clearTimeout(timer); }, [notice]);
  useEffect(() => {
    const position = snapshot.brokerPosition;
    if (!desktop || !walletId || !position?.warnings.length) return;
    addNotification({ id: `position:${walletId}:${position.sourceHash}`, code: 'import-warning', title: 'Confira a posição importada', message: 'Há observações sobre os valores importados da corretora. Você pode conferi-las em FIIs e posições.', tone: 'warning', action: { label: 'Conferir posição', route: 'portfolio', walletId } });
  }, [walletId, snapshot.brokerPosition, addNotification]);
  useEffect(() => {
    if (!error) return;
    addNotification({ code: 'operation-failed', title: 'Não foi possível concluir a operação', message: 'Confira os detalhes na página e tente novamente.', tone: 'error', action: { label: 'Abrir página', route: page, walletId: walletId || undefined } });
  }, [error, addNotification, page, walletId]);

  const navigate = (next: Page, initialTab?: 'position' | 'history') => {
    if (next === 'assistant') { setAssistantStarted(true); setAssistantOpen(false); }
    setPortfolioInitialTab(initialTab); setPage(next); setMobileNav(false);
  };
  const openAssistant = () => {
    if (page === 'assistant') { document.getElementById('chat-draft')?.focus(); return; }
    assistantTrigger.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setAssistantStarted(true); setMobileNav(false); setAssistantOpen(true);
  };
  const openOnboarding = async () => {
    if (onboardingStarting.current || !walletId) return;
    onboardingStarting.current = true;
    try {
      await call('save_onboarding_progress', { progress: { version: 1, walletId, step: 0, status: 'started' } });
      setOnboarding({ walletId, step: 0 });
    } catch (error) { setError(errorMessage(error)); }
    finally { onboardingStarting.current = false; }
  };
  const exitOnboarding = (destination: OnboardingDestination, notice?: string) => {
    if (notice) setNotice(notice);
    if (onboarding && onboarding.walletId !== walletId) selectWallet(onboarding.walletId);
    setOnboarding(null);
    if (destination === 'assistant') {
      setRequestedDraft(current => ({ text: onboardingHelp, token: (current?.token || 0) + 1 }));
      navigate('assistant');
    } else navigate(destination, destination === 'portfolio' ? 'history' : undefined);
  };
  const openNotification = (route: NotificationRoute, conversationId?: string, targetWalletId?: string) => {
    if (targetWalletId && wallets.some((wallet) => wallet.id === targetWalletId)) selectWallet(targetWalletId);
    if (conversationId) setRequestedConversation((current) => ({ id: conversationId, token: (current?.token || 0) + 1 }));
    navigate(route);
    requestAnimationFrame(() => document.getElementById('main-content')?.focus({ preventScroll: true }));
  };
  const selectWallet = (id: string) => {
    if (!id || id === selectedId.current) return;
    selectedId.current = id; setWalletId(id); setSnapshot(emptySnapshot); setInfo(null);
    setImportPath(''); setPositionImportPath('');
    setImportOpen(false); setPositionImportOpen(false); setImportError(''); setPositionImportError('');
    void refresh();
  };
  const importPortfolio = async () => {
    setImporting(true); setImportError('');
    try {
      const receipt = await call<Record<string, unknown>>('import_legacy', { walletId, path: importPath.trim() });
      await refresh(); setImportOpen(false); notify(receipt.alreadyImported ? 'Este arquivo já foi importado. Nenhum registro foi duplicado.' : `Importação concluída: ${receipt.fundCount ?? '—'} fundos e ${receipt.recordCount ?? '—'} lançamentos. Original preservado.`);
    } catch (err) { setImportError(errorMessage(err)); } finally { setImporting(false); }
  };
  const exportPortfolio = async () => {
    try {
      const path = await call<string | null>('export_portfolio_file', { walletId });
      if (path) notify('Cópia da carteira exportada com segurança.');
    } catch (err) { setError(errorMessage(err)); }
  };
  const chooseImport = async () => {
    try { const path = await call<string | null>('choose_import_file'); if (path) setImportPath(path); }
    catch (err) { setImportError(errorMessage(err)); }
  };
  const openPositionImport = () => { setPositionImportError(''); setPositionImportOpen(true); };
  const importPosition = async () => {
    setPositionImporting(true); setPositionImportError('');
    try {
      const imported = await call<BrokerPosition>('import_broker_position', { walletId, path: positionImportPath.trim() });
      await refresh(); setPositionImportOpen(false);
      notify(imported.fixedIncome.latestObservation ? 'Pós-fixado salvo com os detalhes dos aportes. Datas da posição e histórico preservados.' : 'Posição completa salva: FIIs, pós-fixado e saldo em conta. Histórico de compras preservado.');
    } catch (err) { setPositionImportError(errorMessage(err)); }
    finally { setPositionImporting(false); }
  };
  const choosePosition = async () => {
    try { const path = await call<string | null>('choose_import_file'); if (path) setPositionImportPath(path); }
    catch (err) { setPositionImportError(errorMessage(err)); }
  };
  const selectedWallet = wallets.find((wallet) => wallet.id === walletId);
  const openNewWallet = () => { setEditingWallet(null); setWalletOpen(true); };

  return <PublicDataProvider value={publicData}><PageActionsSlot.Provider value={actionsSlot}><div className={`app-shell ${sidebarCollapsed ? 'sidebar-collapsed' : ''} ${drawerOpen ? 'assistant-docked' : ''} ${page === 'assistant' ? 'assistant-current-page' : ''}`}>
    <a className="skip-link" href="#main-content" onClick={(event) => { event.preventDefault(); document.getElementById('main-content')?.focus(); }}>Pular para o conteúdo</a>
    <aside id="app-sidebar" className={`sidebar ${mobileNav ? 'sidebar-open' : ''}`} aria-label="Carteira e navegação" inert={modalDrawer || undefined}>
      <div className="sidebar-header"><a className="brand" href="#" aria-label="IvestFII · Visão geral" onClick={(event) => { event.preventDefault(); navigate('overview'); }}><span className="brand-name">IvestFII</span><span className="brand-short" aria-hidden="true">IF</span></a><button type="button" className="icon-button sidebar-toggle" aria-label={sidebarCollapsed ? 'Expandir barra lateral' : 'Recolher barra lateral'} aria-controls="app-sidebar" aria-expanded={!sidebarCollapsed} title={sidebarCollapsed ? 'Expandir barra lateral' : 'Recolher barra lateral'} onClick={() => setSidebarCollapsed((current) => !current)}>{sidebarCollapsed ? <PanelLeftOpen size={18} /> : <PanelLeftClose size={18} />}</button></div>
      <button type="button" className="icon-button compact-wallet" aria-label={`Carteira: ${selectedWallet?.name || 'carregando'}. Expandir para trocar carteira`} title={selectedWallet?.name || 'Carteira'} onClick={() => { setSidebarCollapsed(false); requestAnimationFrame(() => document.getElementById('wallet-selector')?.focus()); }}><Wallet size={18} /></button>
      <div className="wallet-switcher"><label htmlFor="wallet-selector">Carteira</label><select id="wallet-selector" value={walletId} disabled={!desktop || !wallets.length || importing || positionImporting} onChange={(event) => selectWallet(event.target.value)}>{!wallets.length && <option value="">{desktop ? 'Carregando…' : 'Aplicativo Windows'}</option>}{wallets.map((wallet) => <option key={wallet.id} value={wallet.id}>{wallet.name}{wallet.institution ? ` · ${wallet.institution}` : ''}</option>)}</select><div className="wallet-switcher-actions"><button className="text-link" onClick={openNewWallet} disabled={!desktop || loading}>Nova carteira</button><button className="icon-button" onClick={() => { setEditingWallet(selectedWallet || null); setWalletOpen(true); }} disabled={!desktop || loading || !selectedWallet} title="Editar nome e instituição" aria-label="Editar carteira selecionada"><Pencil size={14} /></button></div></div>
      <nav aria-label="Navegação principal">{navigation.map(({ id, label, icon: Icon }) => <button type="button" key={id} className={`nav-item ${page === id ? 'active' : ''}`} onClick={() => navigate(id)} aria-label={label} title={sidebarCollapsed ? label : undefined} aria-current={page === id ? 'page' : undefined}><Icon size={17} aria-hidden="true" /><span className="nav-label">{label}</span></button>)}</nav>
      <div className="sidebar-bottom"><button type="button" className={`nav-item ${page === 'settings' ? 'active' : ''}`} onClick={() => navigate('settings')} aria-label="Configurações" title={sidebarCollapsed ? 'Configurações' : undefined} aria-current={page === 'settings' ? 'page' : undefined}><SettingsIcon size={17} aria-hidden="true" /><span className="nav-label">Configurações</span></button><div className="sidebar-status"><Def text={desktop ? 'Carteiras e conversas estão salvas neste computador. A sincronização web ainda não está disponível.' : 'Abra o aplicativo Windows para acessar sua carteira. Esta prévia não contém dados financeiros.'}><Database className="sidebar-data-icon" size={16} aria-hidden="true" /><span className="sidebar-status-copy">{desktop ? 'Dados neste computador' : 'Prévia web, sem dados'}</span><span className="sr-only sidebar-status-compact">{desktop ? 'Dados neste computador' : 'Prévia web, sem dados'}</span></Def></div></div>
    </aside>
    {mobileNav && <button className="nav-backdrop" onClick={() => setMobileNav(false)} aria-label="Fechar menu" />}
    <div className="main-shell">
      <header className="topbar" inert={modalDrawer || undefined}><button className="icon-button mobile-menu" onClick={() => setMobileNav(!mobileNav)} aria-label={mobileNav ? 'Fechar menu' : 'Abrir menu'} aria-expanded={mobileNav} aria-controls="app-sidebar"><Menu size={18} /></button><h1>{titles[page]}</h1>{selectedWallet && <span className="topbar-context">{selectedWallet.name}{selectedWallet.institution ? ` · ${selectedWallet.institution}` : ''}</span>}<div className="topbar-actions" ref={setActionsSlot} /><NotificationCenter items={notifications.items} unreadCount={notifications.unreadCount} onMarkRead={notifications.markRead} onMarkAllRead={notifications.markAllRead} onDismiss={notifications.dismiss} onNavigate={openNotification} /></header>
      <main id="main-content" className="main-content" tabIndex={-1}>
        <div className="page-content" inert={modalDrawer || undefined}>
        {!desktop && <div className="preview-banner"><div><strong>Prévia da interface</strong><span>Os dados ficam no aplicativo Windows; a sincronização web ainda não está configurada.</span></div></div>}
        {error && <div className="alert error" role="alert"><span>{error}</span><button className="text-link" onClick={() => void refresh()}><RefreshCw size={14} />Tentar novamente</button></div>}
        {loading && !snapshot.funds.length ? <Loading /> : <>
          {page === 'overview' && <><Overview key={walletId} walletId={walletId} snapshot={snapshot} onNavigate={navigate} onImport={() => setImportOpen(true)} onImportPosition={openPositionImport} />{wealth && wealth.wallets.length > 1 && <details className="extra-details wallets-overview"><summary>Ver outras carteiras</summary><section className="group"><div className="group-head"><h2><Def text="Custo dos FIIs e saldo das caixinhas de cada carteira. A posição importada não entra nestes totais.">Carteiras</Def></h2><div className="group-actions"><button className="text-link" onClick={openNewWallet} disabled={!desktop}>Nova carteira</button></div></div><div className="block"><div className="table-wrap"><table><caption className="sr-only">Custo dos FIIs e saldo das caixinhas por carteira</caption><thead><tr><th scope="col">Carteira</th><th scope="col">Instituição</th><th scope="col" className="num">FIIs · custo</th><th scope="col" className="num">Caixinhas · saldo</th><th scope="col"><span className="sr-only">Abrir</span></th></tr></thead><tbody>{wealth.wallets.map((item) => <tr key={item.wallet.id}><td>{item.wallet.name}{item.wallet.id === walletId && <span className="muted"> · aberta</span>}</td><td className="muted">{item.wallet.institution || '—'}</td><td className="num">{money(item.fiiInvested)}</td><td className="num">{money(item.cashBalance)}</td><td className="num"><button className="text-link" onClick={() => { selectWallet(item.wallet.id); navigate('portfolio'); }} aria-label={`Abrir carteira ${item.wallet.name}`}>Abrir</button></td></tr>)}</tbody><tfoot><tr><td>Total</td><td /><td className="num">{money(wealth.totals.fiiInvested)}</td><td className="num">{money(wealth.totals.cashBalance)}</td><td /></tr></tfoot></table></div></div></section></details>}</>}
          {page === 'portfolio' && <Portfolio key={walletId} walletId={walletId} snapshot={snapshot} refresh={refresh} onImport={() => setImportOpen(true)} onExport={() => void exportPortfolio()} onImportPosition={openPositionImport} notify={notify} initialTab={portfolioInitialTab} />}
          {page === 'cash' && <CashPockets key={walletId} walletId={walletId} walletName={selectedWallet?.name || 'Sua carteira'} onSaved={refresh} notify={notify} />}
          {page === 'market' && <Market walletId={walletId} snapshot={snapshot} />}
          {page === 'projections' && <Projections key={walletId} walletId={walletId} snapshot={snapshot} />}
          {page === 'settings' && <Settings walletId={walletId} walletName={selectedWallet?.name || 'Carteira selecionada'} info={info} onImport={() => setImportOpen(true)} onExport={() => void exportPortfolio()} onImportPosition={openPositionImport} notify={notify} onOnboarding={() => void openOnboarding()} />}
        </>}
        </div>
        <div id="assistant-panel" ref={assistantHost} className={`assistant-host assistant-host-${assistantView}`} hidden={assistantView === 'hidden'} role={drawerOpen ? 'dialog' : undefined} aria-label={drawerOpen ? 'Assistente lateral' : undefined} aria-modal={drawerOpen ? modalDrawer : undefined}>
          {assistantStarted && <Assistant walletId={walletId} walletNames={Object.fromEntries(wallets.map((wallet) => [wallet.id, wallet.name]))} view={assistantView} onOpenPage={() => { navigate('assistant'); requestAnimationFrame(() => document.getElementById('chat-draft')?.focus({ preventScroll: true })); }} onClose={closeAssistant} requestedConversation={requestedConversation} requestedDraft={requestedDraft} onRunNotice={onRunNotice} onBusyChange={setAssistantBusy} onPurchasesSaved={refreshAfterPurchase} />}
        </div>
      </main>
    </div>
    {!drawerOpen && <button type="button" ref={assistantFab} className="assistant-fab" aria-label="Abrir assistente" aria-controls="assistant-panel" aria-expanded={assistantView === 'page'} title={assistantBusy ? 'Assistente respondendo. Abrir conversa' : 'Conversar com o assistente'} onClick={openAssistant}><MessageSquare size={22} aria-hidden="true" />{assistantBusy && <span className="assistant-fab-busy" aria-label="Respondendo"><LoaderCircle size={12} className="spin" /></span>}</button>}
    {onboarding && wallets.some(wallet => wallet.id === onboarding.walletId) && <Onboarding key={onboarding.walletId} wallet={wallets.find(wallet => wallet.id === onboarding.walletId)!} initialStep={onboarding.step} onSaved={refresh} onExit={exitOnboarding} />}
    {notice && <div className="toast" role="status">{notice}<button className="icon-button" onClick={() => setNotice('')} aria-label="Fechar aviso"><X size={14} /></button></div>}
    {walletOpen && <WalletForm wallet={editingWallet || undefined} onClose={() => setWalletOpen(false)} onSaved={async (wallet) => { setWalletOpen(false); selectedId.current = wallet.id; setWalletId(wallet.id); if (!editingWallet) { setSnapshot(emptySnapshot); setImportPath(''); setPositionImportPath(''); } await refresh(); if (editingWallet) notify('Nome e instituição da carteira atualizados.'); else { navigate('portfolio'); notify(`Carteira ${wallet.name} criada. Cadastre os fundos ou importe seu histórico.`); } }} />}
    {importOpen && <Modal busy={importing} title="Importar histórico ou backup" description={`Destino: ${selectedWallet?.name || 'carteira selecionada'}. Traz os lançamentos e a posição incluída no backup.`} onClose={() => { if (!importing) setImportOpen(false); }}><form onSubmit={handleForm(importPortfolio)}><p className="import-info">O arquivo JSON original não é alterado. Importações repetidas são reconhecidas e não duplicam registros.</p><Field label="Caminho do arquivo JSON" hint="Caminho completo do arquivo neste computador."><input autoFocus value={importPath} onChange={(event) => setImportPath(event.target.value)} placeholder="C:\…\carteira.json" required disabled={importing} /></Field><button type="button" className="button secondary small" onClick={() => void chooseImport()} disabled={!desktop || importing}><FolderOpen size={14} />Procurar arquivo</button>{importError && <p className="form-error" role="alert">{importError}</p>}<div className="form-actions"><button type="button" className="button secondary" onClick={() => setImportOpen(false)} disabled={importing}>Cancelar</button><button className="button primary" disabled={importing || !desktop || !walletId || !importPath.trim()}>{importing && <LoaderCircle className="spin" size={16} />}Importar</button></div></form></Modal>}
    {positionImportOpen && <Modal busy={positionImporting} title="Importar posição da corretora" description={`Destino: ${selectedWallet?.name || 'carteira selecionada'}. Importa uma posição completa ou um complemento do pós-fixado.`} onClose={() => { if (!positionImporting) setPositionImportOpen(false); }}><form onSubmit={handleForm(importPosition)}><p className="import-info">A posição fica salva neste computador e entra na exportação da carteira. O histórico de compras e proventos não é alterado.</p><Field label="Arquivo JSON da posição" hint="Posição, complemento do pós-fixado ou backup completo do IvestFII."><input autoFocus value={positionImportPath} onChange={(event) => setPositionImportPath(event.target.value)} placeholder="C:\…\posicao_corretora.json" required disabled={positionImporting} /></Field><button type="button" className="button secondary small" onClick={() => void choosePosition()} disabled={!desktop || positionImporting}><FolderOpen size={14} />Procurar arquivo</button>{positionImportError && <p className="form-error" role="alert">{positionImportError}</p>}<div className="form-actions"><button type="button" className="button secondary" onClick={() => setPositionImportOpen(false)} disabled={positionImporting}>Cancelar</button><button className="button primary" disabled={positionImporting || !desktop || !walletId || !positionImportPath.trim()}>{positionImporting && <LoaderCircle className="spin" size={16} />}Salvar posição</button></div></form></Modal>}
  </div></PageActionsSlot.Provider></PublicDataProvider>;
}

function WalletForm({ wallet, onClose, onSaved }: { wallet?: PortfolioWallet; onClose: () => void; onSaved: (wallet: PortfolioWallet) => Promise<void> }) {
  const [name, setName] = useState(wallet?.name || '');
  const [institution, setInstitution] = useState(wallet?.institution || '');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const save = async () => {
    setBusy(true); setError('');
    try { const saved = await call<PortfolioWallet>(wallet ? 'update_wallet' : 'create_wallet', { input: { id: wallet?.id, name: name.trim(), institution: institution.trim() } }); await onSaved(saved); }
    catch (err) { setError(errorMessage(err)); } finally { setBusy(false); }
  };
  return <Modal title={wallet ? 'Editar carteira' : 'Nova carteira'} description={wallet ? 'Altere o nome e a instituição usados para identificar esta carteira.' : 'Separe investimentos por banco, corretora ou objetivo. A nova carteira começa vazia.'} onClose={() => { if (!busy) onClose(); }}><form onSubmit={handleForm(save)}><Field label="Nome da carteira"><input autoFocus value={name} onChange={(event) => setName(event.target.value)} placeholder="Ex.: FIIs no Nubank" required maxLength={100} disabled={busy} /></Field><Field label="Banco ou corretora" hint="Opcional. Você também pode organizar uma carteira por objetivo."><input value={institution} onChange={(event) => setInstitution(event.target.value)} placeholder="Ex.: Nubank" maxLength={100} disabled={busy} /></Field>{error && <p className="form-error" role="alert">{error}</p>}<div className="form-actions"><button className="button secondary" type="button" onClick={onClose} disabled={busy}>Cancelar</button><Submit busy={busy}>{wallet ? 'Salvar alterações' : 'Criar carteira'}</Submit></div></form></Modal>;
}

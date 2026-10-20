import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { call, desktop, errorMessage } from './bridge';
import type { CvmPanel, EconomicPanel, MarketDataSettings, NewsSnapshot, PublicDataRefresh, PublicDataSourceState, QuotePanel, Snapshot } from './bridge';

export const emptyQuotes: QuotePanel = { items: [], fetchedAt: null, credentialConfigured: false, errors: [] };
export const emptyCvm: CvmPanel = { items: [], fetchedAt: null, errors: [] };
export const emptyEconomics: EconomicPanel = { indicators: [], errors: [] };
type Source = 'economics' | 'news' | 'quotes' | 'cvm';
interface CacheData { economics: EconomicPanel | null; news: NewsSnapshot | null; quotes: QuotePanel; cvm: CvmPanel }
interface State extends CacheData {
  walletId: string; loading: boolean; refreshing: boolean; revision: number;
  error: string; sources: PublicDataSourceState[]; checkedAt: string | null;
}
interface PublicData extends State {
  refreshEconomics: () => Promise<EconomicPanel>;
  refreshNews: () => Promise<NewsSnapshot>;
  refreshQuotes: () => Promise<QuotePanel>;
  refreshCvm: () => Promise<CvmPanel>;
  reloadCaches: () => Promise<void>;
  configurationChanged: (settings?: MarketDataSettings) => void;
}

const initial = (walletId = ''): State => ({ walletId, economics: null, news: null, quotes: emptyQuotes, cvm: emptyCvm, loading: desktop, refreshing: false, revision: 0, error: '', sources: [], checkedAt: null });
// Only in-flight reads are shared. Finished reads are never reused over newer native caches.
const reads = new Map<string, Promise<unknown>>();
export function readPublicCache<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const key = JSON.stringify([command, args]);
  const existing = reads.get(key);
  if (existing) return existing as Promise<T>;
  const promise = call<T>(command, args);
  reads.set(key, promise);
  void promise.finally(() => { if (reads.get(key) === promise) reads.delete(key); }).catch(() => {});
  return promise;
}
const attempts = new Set<string>();
const automatic = new Map<string, Promise<PublicDataRefresh>>();
const automaticReports = new Map<string, Pick<PublicDataRefresh, 'sources' | 'checkedAt'>>();
const manual = new Map<string, Promise<unknown>>();
function automaticRefresh(key: string, walletId: string): Promise<PublicDataRefresh> | null {
  const active = automatic.get(key);
  if (active) return active;
  if (attempts.has(key)) return null;
  attempts.add(key);
  const promise = call<PublicDataRefresh>('refresh_public_data_if_due', { walletId });
  automatic.set(key, promise);
  void promise.then((result) => automaticReports.set(key, { sources: result.sources, checkedAt: result.checkedAt })).catch(() => {});
  void promise.finally(() => { if (automatic.get(key) === promise) automatic.delete(key); }).catch(() => {});
  return promise;
}

async function caches(walletId: string) {
  const results = await Promise.allSettled([
    readPublicCache<EconomicPanel | null>('get_economic_cache'),
    readPublicCache<NewsSnapshot | null>('get_news_cache'),
    readPublicCache<QuotePanel>('get_quote_cache', { walletId }),
    readPublicCache<CvmPanel>('get_cvm_cache', { walletId }),
  ]);
  const data: Partial<CacheData> = {};
  if (results[0].status === 'fulfilled') data.economics = results[0].value;
  if (results[1].status === 'fulfilled') data.news = results[1].value;
  if (results[2].status === 'fulfilled') data.quotes = results[2].value;
  if (results[3].status === 'fulfilled') data.cvm = results[3].value;
  return { data, failed: results.some((result) => result.status === 'rejected') };
}

const Context = createContext<PublicData | null>(null);
export const PublicDataProvider = Context.Provider;
export function usePublicData() {
  const context = useContext(Context);
  if (!context) throw new Error('Dados públicos fora do contexto do aplicativo.');
  return context;
}

/** One bounded native refresh per wallet/input set in this app session; opening a view never queries the network. */
export function usePublicDataController(walletId: string, snapshot: Snapshot, ready: boolean): PublicData {
  const [state, setState] = useState<State>(() => initial(walletId));
  const [configurationRevision, setConfigurationRevision] = useState(0);
  const [wakeRevision, setWakeRevision] = useState(0);
  const configuration = useRef<MarketDataSettings | null>(null);
  const generation = useRef(0);
  const lastWake = useRef(Date.now());
  const manualVersions = useRef<Record<Source, number>>({ economics: 0, news: 0, quotes: 0, cvm: 0 });
  const pending = useRef(new Map<string, { automatic: number; manual: number }>());
  const input = JSON.stringify([
    walletId,
    snapshot.funds.map((fund) => [fund.ticker, fund.entries]).sort((left, right) => String(left[0]).localeCompare(String(right[0]))),
    snapshot.brokerPosition?.listedFunds.holdings.map((holding) => holding.ticker).sort() ?? [],
    configurationRevision,
    wakeRevision,
  ]);
  const activeInput = useRef(input);
  activeInput.current = input;
  const displayed = state.walletId === walletId ? state : initial(walletId);
  const isBusy = useCallback((scope: string) => {
    const counts = pending.current.get(scope);
    return Boolean(counts && counts.automatic + counts.manual > 0);
  }, []);
  const changePending = useCallback((scope: string, kind: 'automatic' | 'manual', change: 1 | -1) => {
    const counts = pending.current.get(scope) ?? { automatic: 0, manual: 0 };
    counts[kind] = Math.max(0, counts[kind] + change);
    if (counts.automatic + counts.manual) pending.current.set(scope, counts); else pending.current.delete(scope);
    if (activeInput.current === scope) setState((previous) => ({ ...previous, refreshing: counts.automatic + counts.manual > 0 }));
  }, []);

  useEffect(() => {
    const wake = () => {
      if (document.visibilityState === 'hidden' || Date.now() - lastWake.current < 30 * 60 * 1000) return;
      lastWake.current = Date.now(); setWakeRevision((revision) => revision + 1);
    };
    window.addEventListener('focus', wake);
    document.addEventListener('visibilitychange', wake);
    return () => { window.removeEventListener('focus', wake); document.removeEventListener('visibilitychange', wake); };
  }, []);

  useEffect(() => {
    if (!desktop || !walletId || !ready) {
      setState((previous) => previous.refreshing === isBusy(input) ? previous : { ...previous, refreshing: isBusy(input) });
      return;
    }
    let cancelled = false;
    const current = ++generation.current;
    const valid = () => !cancelled && current === generation.current && activeInput.current === input;
    setState((previous) => ({ ...(previous.walletId === walletId ? previous : initial(walletId)), loading: true, refreshing: isBusy(input), error: '' }));
    void (async () => {
      const [cached, settings] = await Promise.all([
        caches(walletId),
        configuration.current ? Promise.resolve(configuration.current) : readPublicCache<MarketDataSettings>('get_market_data_settings').catch(() => null),
      ]);
      if (!valid()) return;
      configuration.current = settings;
      setState((previous) => ({ ...previous, ...cached.data, walletId, loading: false, refreshing: isBusy(input), error: cached.failed ? 'Alguns dados salvos não puderam ser lidos. A consulta automática tentará atualizá-los.' : '' }));
      const key = JSON.stringify([input, settings]);
      const job = automaticRefresh(key, walletId);
      if (!job) {
        const report = automaticReports.get(key);
        if (report) setState((previous) => ({ ...previous, ...report }));
        return;
      }
      changePending(input, 'automatic', 1);
      try {
        const result = await job;
        if (!valid()) return;
        const versions = { ...manualVersions.current };
        const latest = await caches(walletId);
        if (!valid()) return;
        setState((previous) => {
          const next = { economics: result.economics ?? previous.economics, news: result.news ?? previous.news, quotes: result.quotes, cvm: result.cvm, ...latest.data };
          for (const source of ['economics', 'news', 'quotes', 'cvm'] as const) if (versions[source] !== manualVersions.current[source]) delete (next as Partial<CacheData>)[source];
          return { ...previous, ...next, sources: result.sources, checkedAt: result.checkedAt, walletId, loading: false, error: '', revision: previous.revision + 1 };
        });
      } catch {
        if (valid()) setState((previous) => ({ ...previous, error: 'Não foi possível consultar as fontes agora. Os dados salvos continuam disponíveis; confira as datas.' }));
      } finally { changePending(input, 'automatic', -1); }
    })();
    return () => { cancelled = true; generation.current += 1; };
  }, [input, walletId, ready, changePending, isBusy]);

  const configurationChanged = useCallback((settings?: MarketDataSettings) => {
    configuration.current = settings ?? null;
    setConfigurationRevision((revision) => revision + 1);
  }, []);
  const reloadCaches = useCallback(async () => {
    if (!desktop || !walletId) return;
    const scope = activeInput.current;
    const versions = { ...manualVersions.current };
    const result = await caches(walletId);
    if (activeInput.current !== scope) return;
    // The caller already has its computed performance result; this only updates shared source caches.
    setState((previous) => {
      const next = { ...result.data };
      for (const source of ['economics', 'news', 'quotes', 'cvm'] as const) if (versions[source] !== manualVersions.current[source]) delete next[source];
      return { ...previous, ...next };
    });
  }, [walletId]);
  const refresh = useCallback(async <T,>(source: Source, command: string): Promise<T> => {
    if (!desktop || !walletId) throw new Error('Consulta disponível no aplicativo Windows.');
    const scope = activeInput.current;
    manualVersions.current[source] += 1;
    const key = JSON.stringify([command, source === 'quotes' || source === 'cvm' ? walletId : 'global']);
    let promise = manual.get(key) as Promise<T> | undefined;
    if (!promise) {
      promise = call<T>(command, source === 'quotes' || source === 'cvm' ? { walletId } : undefined);
      manual.set(key, promise);
      void promise.finally(() => { if (manual.get(key) === promise) manual.delete(key); }).catch(() => {});
    }
    changePending(scope, 'manual', 1);
    try {
      const result = await promise;
      if (activeInput.current === scope) {
        manualVersions.current[source] += 1;
        setState((previous) => ({ ...previous, [source]: result, revision: previous.revision + (source === 'quotes' ? 1 : 0), error: '' }));
      }
      return result;
    } catch (err) { throw new Error(errorMessage(err)); }
    finally { changePending(scope, 'manual', -1); }
  }, [walletId, changePending]);

  return useMemo(() => ({
    ...displayed,
    refreshEconomics: () => refresh<EconomicPanel>('economics', 'refresh_economic_indicators'),
    refreshNews: () => refresh<NewsSnapshot>('news', 'refresh_news'),
    refreshQuotes: () => refresh<QuotePanel>('quotes', 'refresh_quotes'),
    refreshCvm: () => refresh<CvmPanel>('cvm', 'refresh_cvm_data'),
    reloadCaches, configurationChanged,
  }), [displayed, refresh, reloadCaches, configurationChanged]);
}

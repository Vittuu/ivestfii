import { useCallback, useEffect, useSyncExternalStore } from 'react';
import { call, desktop, errorMessage } from './bridge';
import type { InvestmentProfile, InvestmentProfileInput } from './bridge';

interface State { profile: InvestmentProfile | null; loaded: boolean; loading: boolean; error: string }
interface Slot { state: State; listeners: Set<() => void>; request: Promise<void> | null; generation: number }
const wallets = new Map<string, Slot>();
function slot(walletId: string): Slot {
  let value = wallets.get(walletId);
  if (!value) {
    value = { state: { profile: null, loaded: !desktop, loading: false, error: '' }, listeners: new Set(), request: null, generation: 0 };
    wallets.set(walletId, value);
  }
  return value;
}
function publish(target: Slot, state: State) {
  target.state = state;
  for (const notify of target.listeners) notify();
}
function load(walletId: string, force = false): Promise<void> {
  if (!desktop || !walletId) return Promise.resolve();
  const target = slot(walletId);
  if (target.request) return target.request;
  if (target.state.loaded && !force) return Promise.resolve();
  const generation = ++target.generation;
  publish(target, { ...target.state, loading: true, error: '' });
  const request = call<InvestmentProfile | null>('get_investment_profile', { walletId }).then((profile) => {
    if (target.generation === generation) publish(target, { profile, loaded: true, loading: false, error: '' });
  }).catch((error) => {
    if (target.generation === generation) publish(target, { ...target.state, loaded: true, loading: false, error: errorMessage(error) });
  }).finally(() => { if (target.request === request) target.request = null; });
  target.request = request;
  return request;
}

/** Kept in memory only; the native command persists the profile in this wallet's database. */
export function useInvestmentProfile(walletId?: string) {
  const id = walletId || '';
  const subscribe = useCallback((notify: () => void) => {
    const target = slot(id); target.listeners.add(notify);
    return () => { target.listeners.delete(notify); };
  }, [id]);
  const snapshot = useCallback(() => slot(id).state, [id]);
  const state = useSyncExternalStore(subscribe, snapshot);
  useEffect(() => { void load(id); }, [id]);
  const reload = useCallback(() => load(id, true), [id]);
  return { ...state, reload };
}

export async function saveInvestmentProfile(walletId: string, input: InvestmentProfileInput) {
  const target = slot(walletId);
  const generation = ++target.generation;
  const profile = await call<InvestmentProfile>('save_investment_profile', { walletId, input });
  if (target.generation === generation) publish(target, { profile, loaded: true, loading: false, error: '' });
  return profile;
}

export const riskLabels: Record<InvestmentProfileInput['riskTolerance'], string> = {
  unknown: 'risco não informado', conservative: 'conservador', moderate: 'moderado', aggressive: 'arrojado',
};

/** The shared provider policy obtains the current profile and public data through MCP. */
export function investmentAnalysisPrompt() {
  return 'Analise a carteira selecionada usando meu perfil e os dados mais recentes disponíveis. Mostre melhorias para meus objetivos de curto e longo prazo, com motivos, riscos e o que ainda preciso informar.';
}

/** The native analysis context provides purchaseAnalysis through the same MCP flow. */
export function contributionAnalysisPrompt() {
  return 'Planeje meus próximos aportes na carteira selecionada usando meu perfil e os dados atualizados. Compare as cotações com meu preço médio, sem tratar um preço menor como motivo suficiente para comprar. Avalie diversificação, objetivos de curto e longo prazo e riscos. Se faltar meu orçamento disponível ou alguma informação do perfil, pergunte antes de sugerir valores e quantidades.';
}

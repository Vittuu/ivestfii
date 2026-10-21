import { useEffect, useState } from 'react';
import { dateBR, desktop, monthShort } from '../lib/bridge';
import { riskLabels, useInvestmentProfile } from '../lib/investmentProfile';
import { usePublicData } from '../lib/publicData';
import { Def, Modal } from '../components';
import InvestmentProfileForm from './InvestmentProfileForm';
import type { AssistantView } from '../views/Assistant';

const quoteDayFormat = new Intl.DateTimeFormat('en-CA', { timeZone: 'America/Sao_Paulo', year: 'numeric', month: '2-digit', day: '2-digit' });
function quoteDay(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  const parts = quoteDayFormat.formatToParts(date);
  const part = (type: 'year' | 'month' | 'day') => parts.find((item) => item.type === type)?.value;
  return `${part('year')}-${part('month')}-${part('day')}`;
}

export default function AssistantAnalysisBar({ walletId, walletName, view, disabled, onAnalyze, onPlanContributions }: { walletId?: string; walletName?: string; view: AssistantView; disabled: boolean; onAnalyze: () => void; onPlanContributions: () => void }) {
  const profile = useInvestmentProfile(walletId);
  const data = usePublicData();
  const [editing, setEditing] = useState(false);
  const [saving, setSaving] = useState(false);
  useEffect(() => { setEditing(false); setSaving(false); }, [walletId]);
  useEffect(() => { if (view === 'hidden') { setEditing(false); setSaving(false); } }, [view]);
  const quoteDates = [...new Set(data.quotes.items.flatMap((item) => { const day = item.observation?.quotedAt ? quoteDay(item.observation.quotedAt) : null; return day ? [day] : []; }))];
  const cvmMonths = [...new Set(data.cvm.items.flatMap((item) => item.report ? [item.report.referenceDate.slice(0, 7)] : []))];
  const cdi = data.economics?.indicators.find((item) => item.id === 'cdi');
  const dates = [quoteDates.length === 1 ? `Cotações de ${dateBR(quoteDates[0])}` : quoteDates.length > 1 ? 'Cotações de datas diferentes' : 'Sem cotação salva', cdi ? `CDI de ${dateBR(cdi.referenceDate)}` : 'Sem CDI salvo', cvmMonths.length === 1 ? `CVM ${monthShort(cvmMonths[0])}` : cvmMonths.length > 1 ? 'CVM de meses diferentes' : 'Sem informe CVM'].join(' · ');
  const pending = profile.loading || !profile.loaded;
  const analysisDisabled = !desktop || !walletId || disabled || pending || Boolean(profile.error) || data.loading || data.refreshing;
  return <div className="chat-analysis-bar">
    <div className="chat-analysis-top"><span>Carteira: <strong>{walletName || (walletId ? 'Carteira selecionada' : 'Nenhuma selecionada')}</strong></span><button className="button small" type="button" onClick={onAnalyze} disabled={analysisDisabled}>Analisar carteira</button><button className="button small" type="button" onClick={onPlanContributions} disabled={analysisDisabled}>Planejar aportes</button></div>
    <div className="chat-profile-row">
      <span>{pending ? 'Lendo perfil…' : profile.error ? 'Perfil indisponível.' : profile.profile ? <>Perfil salvo · {riskLabels[profile.profile.riskTolerance]}</> : 'Perfil não preenchido.'}</span>
      {profile.error ? <button className="text-link" type="button" onClick={() => void profile.reload()} disabled={pending}>Tentar ler perfil</button> : <button className="text-link" type="button" onClick={() => setEditing(true)} disabled={!desktop || !walletId || pending}>{profile.profile ? 'Editar perfil' : 'Completar perfil'}</button>}
    </div>
    <p className="chat-analysis-data"><Def text="Referências salvas disponíveis agora. Ao pedir a análise, o assistente consulta as ferramentas MCP, confere as datas e busca atualizações quando permitido pelas fontes.">Dados para a análise</Def>: {dates}{data.refreshing && <span> · conferindo fontes…</span>}</p>
    {editing && walletId && !profile.error && <Modal title="Perfil de investimento" description={walletName || 'Carteira selecionada'} busy={saving} onClose={() => { if (!saving) setEditing(false); }}><InvestmentProfileForm key={walletId} walletId={walletId} profile={profile.profile} onBusyChange={setSaving} onCancel={() => { setEditing(false); setSaving(false); }} onSaved={() => { setEditing(false); setSaving(false); }} /></Modal>}
  </div>;
}

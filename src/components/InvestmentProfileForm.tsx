import { useEffect, useRef, useState } from 'react';
import { LoaderCircle } from 'lucide-react';
import { dateBR, decimal, desktop, errorMessage } from '../lib/bridge';
import type { InvestmentProfile, InvestmentProfileInput } from '../lib/bridge';
import { saveInvestmentProfile, useInvestmentProfile } from '../lib/investmentProfile';
import { Field, handleForm, Loading } from '../components';
import '../styles/investment-profile.css';

function fields(profile: InvestmentProfile | null) {
  return {
    riskTolerance: profile?.riskTolerance ?? 'unknown', shortTermObjective: profile?.shortTermObjective ?? '',
    shortTermMonths: profile?.shortTermMonths == null ? '' : String(profile.shortTermMonths),
    longTermObjective: profile?.longTermObjective ?? '', longTermMonths: profile?.longTermMonths == null ? '' : String(profile.longTermMonths),
    liquidityNeeds: profile?.liquidityNeeds ?? '', emergencyReserveStatus: profile?.emergencyReserveStatus ?? 'unknown',
    monthlyContribution: profile?.monthlyContribution ?? '', knowledgeLevel: profile?.knowledgeLevel ?? '',
    constraints: profile?.constraints ?? '', notes: profile?.notes ?? '',
  };
}

export function InvestmentProfileSection({ walletId, walletName }: { walletId: string; walletName: string }) {
  const state = useInvestmentProfile(walletId);
  return <section className="group investment-profile-section" aria-labelledby="settings-investment-profile">
    <div className="group-head"><h2 id="settings-investment-profile">Perfil de investimento</h2><p>{walletName}</p></div>
    <div className="block">
      {state.loading || !state.loaded ? <Loading /> : state.error ? <div className="block-body"><p className="form-error" role="alert">Não foi possível ler o perfil: {state.error}</p><button className="button small" type="button" onClick={() => void state.reload()}>Tentar novamente</button></div> : <InvestmentProfileForm key={walletId} walletId={walletId} profile={state.profile} />}
    </div>
  </section>;
}

export default function InvestmentProfileForm({ walletId, profile, onSaved, onCancel, onBusyChange }: { walletId: string; profile: InvestmentProfile | null; onSaved?: () => void; onCancel?: () => void; onBusyChange?: (busy: boolean) => void }) {
  const [values, setValues] = useState(() => fields(profile));
  const [initial, setInitial] = useState(() => fields(profile));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [externalChange, setExternalChange] = useState(false);
  const pending = useRef(false);
  const seenProfile = useRef(profile);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const changed = JSON.stringify(values) !== JSON.stringify(initial);
  useEffect(() => {
    if (seenProfile.current === profile) return;
    seenProfile.current = profile;
    if (pending.current) return;
    const saved = fields(profile);
    if (!changed || JSON.stringify(values) === JSON.stringify(saved)) {
      setValues(saved); setInitial(saved); setExternalChange(false);
    } else setExternalChange(true);
  }, [profile, changed, values]);
  const update = (name: keyof typeof values, value: string) => { setValues((current) => ({ ...current, [name]: value })); setNotice(''); };
  const save = async () => {
    if (!desktop || !walletId || pending.current || !changed) return;
    const input: InvestmentProfileInput = {
      riskTolerance: values.riskTolerance as InvestmentProfileInput['riskTolerance'],
      shortTermObjective: values.shortTermObjective.trim(), shortTermMonths: values.shortTermMonths.trim() ? Number(values.shortTermMonths) : null,
      longTermObjective: values.longTermObjective.trim(), longTermMonths: values.longTermMonths.trim() ? Number(values.longTermMonths) : null,
      liquidityNeeds: values.liquidityNeeds.trim(), emergencyReserveStatus: values.emergencyReserveStatus as InvestmentProfileInput['emergencyReserveStatus'],
      monthlyContribution: values.monthlyContribution.trim() ? decimal(values.monthlyContribution) : null,
      knowledgeLevel: values.knowledgeLevel ? values.knowledgeLevel as InvestmentProfileInput['knowledgeLevel'] : null,
      constraints: values.constraints.trim(), notes: values.notes.trim(),
    };
    pending.current = true; setBusy(true); onBusyChange?.(true); setError(''); setNotice('');
    try { const saved = await saveInvestmentProfile(walletId, input); if (mounted.current) { const updated = fields(saved); setValues(updated); setInitial(updated); setExternalChange(false); setNotice('Perfil salvo nesta carteira.'); onSaved?.(); } }
    catch (error) { if (mounted.current) setError(errorMessage(error)); }
    finally { pending.current = false; if (mounted.current) { setBusy(false); onBusyChange?.(false); } }
  };
  const disabled = busy || !desktop || !walletId;
  return <form className="investment-profile-form" onSubmit={handleForm(save)} aria-label="Perfil de investimento" aria-busy={busy || undefined}>
    <p className="profile-intro">Conte o que você quer fazer com o dinheiro. Pode salvar só o que já sabe; o assistente perguntará o que faltar.</p>
    {externalChange && <div className="profile-external-change" role="status"><p>O perfil foi alterado em outra tela. Seu rascunho foi mantido; “Salvar meu rascunho” substituirá o perfil salvo.</p><button className="text-link" type="button" disabled={disabled} onClick={() => { const saved = fields(profile); setValues(saved); setInitial(saved); setExternalChange(false); setNotice(''); }}>Carregar perfil salvo</button></div>}
    <div className="form-grid">
      <Field label="Perfil de risco" hint="Quanto você aceita que os investimentos oscilem ou tenham perdas. Se ainda não sabe, deixe Não informado."><select value={values.riskTolerance} disabled={disabled} onChange={(event) => update('riskTolerance', event.target.value)}><option value="unknown">Não informado</option><option value="conservative">Conservador</option><option value="moderate">Moderado</option><option value="aggressive">Arrojado</option></select></Field>
      <Field label="Reserva de emergência" hint="Dinheiro separado para imprevistos. Esse estado é informado por você; não é calculado pelo saldo da carteira."><select value={values.emergencyReserveStatus} disabled={disabled} onChange={(event) => update('emergencyReserveStatus', event.target.value)}><option value="unknown">Não informado</option><option value="not_started">Ainda não comecei</option><option value="partial">Em formação</option><option value="complete">Concluída</option></select></Field>
    </div>
    <Field label="Quando você pode precisar do dinheiro?" hint="Descreva quanto precisa poder retirar e em quanto tempo. Ex.: parte da reserva a qualquer momento."><textarea rows={2} maxLength={1000} value={values.liquidityNeeds} disabled={disabled} onChange={(event) => update('liquidityNeeds', event.target.value)} placeholder="Necessidade de acesso ao dinheiro" /></Field>
    <fieldset className="profile-objectives"><legend>Seus objetivos</legend>
      <div className="profile-objective-row"><Field label="Objetivo de curto prazo"><input maxLength={500} value={values.shortTermObjective} disabled={disabled} onChange={(event) => update('shortTermObjective', event.target.value)} placeholder="Ex.: uma compra planejada" /></Field><Field label="Prazo de curto prazo (meses)" hint="Deixe vazio se ainda não sabe. Entre 1 e 1.200 meses."><input type="number" min="1" max="1200" step="1" value={values.shortTermMonths} disabled={disabled} onChange={(event) => update('shortTermMonths', event.target.value)} placeholder="Não informado" /></Field></div>
      <div className="profile-objective-row"><Field label="Objetivo de longo prazo"><input maxLength={500} value={values.longTermObjective} disabled={disabled} onChange={(event) => update('longTermObjective', event.target.value)} placeholder="Ex.: construir uma renda futura" /></Field><Field label="Prazo de longo prazo (meses)" hint="Ex.: 120 meses para 10 anos. Deixe vazio se ainda não sabe."><input type="number" min="1" max="1200" step="1" value={values.longTermMonths} disabled={disabled} onChange={(event) => update('longTermMonths', event.target.value)} placeholder="Não informado" /></Field></div>
    </fieldset>
    <details className="extra-details profile-preferences"><summary>Outras preferências (opcional)</summary><div>
      <div className="form-grid"><Field label="Quanto pretende investir por mês (R$)" hint="Sem separador de milhar. Informe 0 se não pretende aplicar; deixe vazio se não sabe."><input inputMode="decimal" value={values.monthlyContribution} disabled={disabled} onChange={(event) => update('monthlyContribution', event.target.value)} placeholder="Não informado" /></Field><Field label="Experiência com investimentos"><select value={values.knowledgeLevel} disabled={disabled} onChange={(event) => update('knowledgeLevel', event.target.value)}><option value="">Não informada</option><option value="beginner">Iniciante</option><option value="intermediate">Intermediária</option><option value="advanced">Avançada</option></select></Field></div>
      <Field label="Restrições e preferências"><textarea rows={2} maxLength={1000} value={values.constraints} disabled={disabled} onChange={(event) => update('constraints', event.target.value)} placeholder="Ex.: investimentos que prefere evitar" /></Field>
      <Field label="Outras informações"><textarea rows={3} maxLength={4000} value={values.notes} disabled={disabled} onChange={(event) => update('notes', event.target.value)} /></Field>
    </div></details>
    {error && <p className="form-error" role="alert">{error}</p>}
    {notice && <p className="profile-notice" role="status">{notice}</p>}
    <div className="profile-form-foot"><span className={changed ? 'settings-pending' : undefined} aria-live="polite">{changed ? 'Alterações não salvas.' : profile ? <>Salvo em <time dateTime={profile.updatedAt}>{dateBR(profile.updatedAt)}</time>.</> : 'Nenhum perfil salvo nesta carteira.'}</span><div>{onCancel && <button className="button secondary" type="button" disabled={busy} onClick={onCancel}>Cancelar</button>}<button className="button primary" type="submit" disabled={disabled || !changed}>{busy && <LoaderCircle size={15} className="spin" />}{busy ? 'Salvando…' : externalChange ? 'Salvar meu rascunho' : 'Salvar perfil'}</button></div></div>
  </form>;
}

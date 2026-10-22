import { useEffect, useRef, useState } from 'react';
import { Field, Loading, Modal } from '../components';
import { call, errorMessage } from '../lib/bridge';
import type { InvestmentProfile, InvestmentProfileInput, PortfolioWallet } from '../lib/bridge';
import { initialProfile } from '../lib/onboarding';
import { saveInvestmentProfile, useInvestmentProfile } from '../lib/investmentProfile';
import '../styles/onboarding.css';

export type OnboardingDestination = 'overview' | 'portfolio' | 'assistant';
export default function Onboarding({ wallet, initialStep, onSaved, onExit }: { wallet: PortfolioWallet; initialStep: number; onSaved: () => Promise<void>; onExit: (destination: OnboardingDestination, notice?: string) => void }) {
  const [step, setStep] = useState(initialStep);
  const [name, setName] = useState(wallet.name);
  const [institution, setInstitution] = useState(wallet.institution);
  const profile = useInvestmentProfile(wallet.id);
  const [profileReady, setProfileReady] = useState(false);
  const [risk, setRisk] = useState<InvestmentProfileInput['riskTolerance']>('unknown');
  const [reserve, setReserve] = useState<InvestmentProfileInput['emergencyReserveStatus']>('unknown');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const pending = useRef(false);
  const initialized = useRef(false);
  const body = useRef<HTMLDivElement>(null);
  const previousStep = useRef(step);
  useEffect(() => {
    if (previousStep.current === step) return;
    previousStep.current = step;
    const frame = requestAnimationFrame(() => body.current?.querySelector<HTMLElement>('h3')?.focus());
    return () => cancelAnimationFrame(frame);
  }, [step]);
  useEffect(() => {
    let current = true;
    void profile.reload().finally(() => { if (current) setProfileReady(true); });
    return () => { current = false; };
  }, [profile.reload]);
  useEffect(() => {
    if (initialized.current || !profileReady || !profile.loaded || profile.loading || profile.error) return;
    initialized.current = true;
    setRisk(profile.profile?.riskTolerance ?? 'unknown');
    setReserve(profile.profile?.emergencyReserveStatus ?? 'unknown');
  }, [profile, profileReady]);

  const progress = (nextStep: number, status: 'started' | 'skipped' | 'completed') => call('save_onboarding_progress', { progress: { version: 1, walletId: wallet.id, step: nextStep, status } });
  const run = async (operation: () => Promise<void>) => {
    if (pending.current) return;
    pending.current = true; setBusy(true); setError('');
    try { await operation(); }
    catch (error) { setError(errorMessage(error)); }
    finally { pending.current = false; setBusy(false); }
  };
  const next = () => run(async () => {
    if (step === 0) {
      if (!name.trim()) throw new Error('Dê um nome para sua carteira ou mantenha Carteira principal.');
      if (name.trim() !== wallet.name || institution.trim() !== wallet.institution) {
        await call('update_wallet', { input: { id: wallet.id, name: name.trim(), institution: institution.trim() } });
      }
    } else if (step === 1) {
      if (!profileReady || !profile.loaded || profile.loading || profile.error) throw new Error('Aguarde a leitura do perfil ou deixe esta etapa para depois.');
      const latest = await call<InvestmentProfile | null>('get_investment_profile', { walletId: wallet.id });
      if (risk !== (latest?.riskTolerance ?? 'unknown') || reserve !== (latest?.emergencyReserveStatus ?? 'unknown')) {
        await saveInvestmentProfile(wallet.id, initialProfile(latest, risk, reserve));
      }
    }
    const target = Math.min(step + 1, 2);
    await progress(target, 'started');
    setStep(target);
    // A refresh failure must not hide the saved step or cause another write.
    try { await onSaved(); } catch { /* The main page exposes its own retry. */ }
  });
  const leave = (destination: OnboardingDestination, complete = false) => run(async () => {
    let notice: string | undefined;
    try { await progress(step, complete ? 'completed' : 'skipped'); }
    catch { notice = 'Você pode continuar usando o aplicativo. Não foi possível guardar a opção de fechar os primeiros passos; eles podem aparecer na próxima abertura.'; }
    onExit(destination, notice);
  });
  return <Modal title="Primeiros passos" description="Comece com o básico. Você pode deixar o restante para depois." busy={busy} onClose={() => void leave('overview')}>
    <form className="onboarding" onSubmit={event => { event.preventDefault(); void next(); }} aria-busy={busy || undefined}>
      <ol className="onboarding-steps" aria-label="Etapas da configuração">{['Sua carteira', 'Seu jeito', 'Começar'].map((label, index) => <li key={label} aria-current={step === index ? 'step' : undefined}><span aria-hidden="true">{index + 1}</span>{label}</li>)}</ol>
      <div ref={body} className="onboarding-step" aria-live="polite">
        {step === 0 && <>
          <h3 tabIndex={-1}>Um lugar para seus investimentos</h3>
          <p>Carteira é o nome do grupo que você quer acompanhar. Pode separar por banco ou por objetivo.</p>
          <Field label="Nome da carteira"><input value={name} onChange={event => setName(event.target.value)} maxLength={100} required disabled={busy} placeholder="Ex.: Meus investimentos" /></Field>
          <Field label="Banco ou corretora (opcional)"><input value={institution} onChange={event => setInstitution(event.target.value)} maxLength={100} disabled={busy} placeholder="Pode deixar em branco" /></Field>
          <p className="onboarding-note">Comece só com um nome. Você pode mudar isso depois.</p>
        </>}
        {step === 1 && <>
          <h3 tabIndex={-1}>Duas perguntas rápidas</h3><p>Essas respostas ajudam o assistente a conversar com você. Se ainda não souber, deixe “Não sei ainda”.</p>
          {profile.loading || !profile.loaded || !profileReady ? <Loading /> : profile.error ? <div role="alert"><p className="form-error">Não foi possível ler seu perfil. Suas informações foram preservadas.</p><button className="text-link" type="button" disabled={busy} onClick={() => void profile.reload()}>Tentar novamente</button></div> : <>
            <Field label="Como você lida com quedas no valor investido?"><select aria-label="Como você lida com quedas no valor investido?" value={risk} onChange={event => setRisk(event.target.value as typeof risk)} disabled={busy}><option value="unknown">Não sei ainda</option><option value="conservative">Prefiro evitar quedas · conservador</option><option value="moderate">Aceito algumas oscilações · moderado</option><option value="aggressive">Aceito oscilações maiores · arrojado</option></select></Field>
            <Field label="Já separou dinheiro para imprevistos?"><select aria-label="Já separou dinheiro para imprevistos?" value={reserve} onChange={event => setReserve(event.target.value as typeof reserve)} disabled={busy}><option value="unknown">Não sei ainda</option><option value="not_started">Ainda não comecei minha reserva</option><option value="partial">Estou formando minha reserva</option><option value="complete">Minha reserva está pronta</option></select></Field>
          </>}
          <p className="onboarding-note">Objetivos, prazos e quanto investir podem ficar para depois. Antes de sugerir aportes, a IA perguntará o que faltar. Você completa seu perfil em Configurações.</p>
          <p className="onboarding-note">Suas respostas são salvas ao continuar.</p>
        </>}
        {step === 2 && <>
          <h3 tabIndex={-1}>Por onde quer começar?</h3>
          <p>Em <strong>FIIs e posições</strong>, adicione seus fundos ou importe um histórico. Em <strong>Caixinhas CDI</strong>, registre sua reserva e outros saldos.</p>
          <div className="onboarding-help"><strong>Ficou com alguma dúvida?</strong><p>Use o assistente para entender termos, telas e próximos passos. Exemplo: “O que é CDI?” ou “Como cadastro minha primeira compra?”.</p><p>Escolha uma IA no Assistente. Se precisar conectar sua conta, a tela informa. A pergunta só é enviada ao clicar em Enviar.</p></div>
          <div className="onboarding-start"><button className="button primary" type="button" disabled={busy} onClick={() => void leave('portfolio', true)}>Adicionar meus FIIs</button><button className="button" type="button" disabled={busy} onClick={() => void leave('assistant', true)}>Tirar dúvidas com a IA</button></div>
          <p className="onboarding-note">Você pode rever estes passos e completar as informações em Configurações.</p>
        </>}
      </div>
      {error && <p className="form-error" role="alert">{error}</p>}
      <div className="form-actions"><button type="button" className="button secondary" disabled={busy} onClick={() => void leave('overview', step === 2)}>{step === 2 ? 'Explorar o aplicativo' : 'Fazer depois'}</button>{step > 0 && <button type="button" className="button secondary" disabled={busy} onClick={() => { setStep(step - 1); setError(''); }}>Voltar</button>}{step < 2 && <button className="button primary" type="submit" disabled={busy || step === 1 && (!profileReady || !profile.loaded || profile.loading || Boolean(profile.error))}>{busy ? 'Salvando…' : 'Continuar'}</button>}</div>
      {step < 2 && <button className="text-link onboarding-ask" type="button" disabled={busy} onClick={() => void leave('assistant')}>Prefiro tirar uma dúvida com a IA</button>}
    </form>
  </Modal>;
}

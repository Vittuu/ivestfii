import { useCallback, useEffect, useRef, useState } from 'react';
import { LoaderCircle, Pencil, Plus, Trash2 } from 'lucide-react';
import { amount, call, dateBR, decimal, desktop, errorMessage, money, percent } from '../lib/bridge';
import type { CashFlow, CashPerformance, CashPocket, CashSimulation, EconomicIndicator } from '../lib/bridge';
import { Def, Empty, Field, handleForm, Loading, Modal, PageActions, SectionHeading } from '../components';
import PublicSourceLink from '../components/PublicSourceLink';

interface Props { walletId: string; walletName: string; onSaved: () => Promise<void>; notify: (message: string) => void }

export default function CashPockets({ walletId, walletName, onSaved, notify }: Props) {
  const [pockets, setPockets] = useState<CashPocket[]>([]);
  const [performance, setPerformance] = useState<CashPerformance | null>(null);
  const [performanceError, setPerformanceError] = useState('');
  const [loading, setLoading] = useState(desktop);
  const [error, setError] = useState('');
  const [simError, setSimError] = useState('');
  const [editing, setEditing] = useState<CashPocket | 'new' | null>(null);
  const [selected, setSelected] = useState('');
  const [days, setDays] = useState('252');
  const [simulation, setSimulation] = useState<CashSimulation | null>(null);
  const [simulating, setSimulating] = useState(false);
  const request = useRef(0);
  const simulateButton = useRef<HTMLButtonElement>(null);
  /** Ligado quando o formulário foi aberto pelo atalho do aviso de taxa ausente. */
  const focusSimulateAfterSave = useRef(false);
  const load = useCallback(async () => {
    if (!desktop || !walletId) { setLoading(false); return; }
    const current = ++request.current;
    setLoading(true); setError(''); setPerformanceError(''); setPerformance(null);
    try {
      const [pocketResult, performanceResult] = await Promise.allSettled([
        call<CashPocket[]>('get_cash_pockets', { walletId }),
        call<CashPerformance>('get_cash_performance', { walletId }),
      ]);
      if (current !== request.current) return;
      if (pocketResult.status === 'fulfilled') {
        setPockets(pocketResult.value);
        setSelected((id) => pocketResult.value.some((item) => item.id === id) ? id : pocketResult.value[0]?.id || '');
      } else setError(errorMessage(pocketResult.reason));
      if (performanceResult.status === 'fulfilled') setPerformance(performanceResult.value);
      else setPerformanceError(`Não foi possível calcular o rendimento cadastrado: ${errorMessage(performanceResult.reason)}`);
    } catch (err) { if (current === request.current) setError(errorMessage(err)); }
    finally { if (current === request.current) setLoading(false); }
  }, [walletId]);
  useEffect(() => { void load(); return () => { request.current += 1; }; }, [load]);
  const pocket = pockets.find((item) => item.id === selected);
  const configured = Boolean(pocket?.annualCdiRate !== null && pocket?.annualCdiRate !== undefined && pocket?.rateDate);
  // Depois de salvar pelo atalho "Editar caixinha", o atalho some (a caixinha passa a ter taxa):
  // o foco vai para o botão de cálculo em vez de cair no início da página.
  useEffect(() => {
    if (!focusSimulateAfterSave.current || editing || loading) return;
    focusSimulateAfterSave.current = false;
    if (configured) simulateButton.current?.focus();
  }, [configured, editing, loading]);
  const simulate = async () => {
    if (!pocket || !configured) return;
    setSimulating(true); setError(''); setSimError(''); setSimulation(null);
    try { setSimulation(await call<CashSimulation>('simulate_cash_pocket', { walletId, input: { pocketId: pocket.id, businessDays: Number(days) } })); }
    catch (err) { setSimError(errorMessage(err)); } finally { setSimulating(false); }
  };
  const selectPocket = (id: string) => { setSelected(id); setSimulation(null); setError(''); setSimError(''); };
  const total = pockets.reduce((sum, item) => sum + (Number(item.balance) || 0), 0);


  return <div className="page cash-page">
    <PageActions><button className="button primary" disabled={!desktop || !walletId || simulating} onClick={() => setEditing('new')}><Plus size={16} />Nova caixinha</button></PageActions>
    {error && <div className="alert error" role="alert">{error}</div>}
    {loading ? <Loading /> : pockets.length ? <>
      <section className="group" aria-labelledby="cash-balances-title">
        <div className="group-head"><h2 id="cash-balances-title"><Def text="Saldos cadastrados por você. Eles ficam separados da posição importada da corretora e não devem ser somados duas vezes.">Seu dinheiro guardado</Def></h2></div>
        <div className="modular">
          <div className="block span-12 cash-total">
            <dl className="cash-actual-readouts">
              <div className="readout"><dt className="figure-label"><Def text="Soma dos saldos que você cadastrou nesta carteira. Atualize cada saldo e sua data quando ele mudar no banco.">Total nas caixinhas</Def></dt><dd className="figure-value lg">{money(performance?.balance ?? total)}</dd><dd className="figure-note">{pockets.length === 1 ? '1 caixinha' : `${pockets.length} caixinhas`}</dd></div>
              <div className="readout cash-applied"><dt className="figure-label"><Def text="Todas as aplicações feitas nas caixinhas, inclusive dinheiro que depois foi resgatado. Não é o saldo atual.">Total aplicado</Def></dt><dd className="figure-value lg">{performance?.invested == null ? '—' : money(performance.invested)}</dd></div>
              <div className="readout"><dt className="figure-label"><Def text="Saldo informado + total resgatado − total aplicado, com base nos valores e na data de cada caixinha. Não usa a taxa CDI da simulação.">Rendimento acumulado</Def></dt><dd className={`figure-value lg ${cashResultClass(performance?.income)}`}>{cashSignedMoney(performance?.income)}</dd></div>
            </dl>
            {performanceError && <p className="cash-actual-note form-error" role="alert">{performanceError}</p>}
            {performance && performance.missingBasisCount > 0 && <p className="cash-actual-note" role="status">Faltam aplicações, resgates ou data do saldo em {performance.missingBasisCount} {performance.missingBasisCount === 1 ? 'caixinha' : 'caixinhas'}. Edite as caixinhas para completar os totais; campo vazio não significa zero.</p>}
          </div>
          <div className="block span-12 cash-list">
            <SectionHeading level={3} title="Caixinhas" />
            <div className="table-wrap">
              <table className="cash-table">
                <caption className="sr-only">Caixinhas da carteira {walletName}: saldo, data do saldo, total aplicado, rendimento acumulado e porcentagem do CDI contratada</caption>
                <colgroup><col /><col className="cash-col-balance" /><col className="cash-col-actual" /><col className="cash-col-actual" /><col className="cash-col-cdi" /><col className="cash-col-actions" /></colgroup>
                <thead><tr>
                  <th scope="col">Caixinha</th>
                  <th scope="col" className="num"><Def text="Saldo cadastrado e a data em que esse valor estava disponível no banco. Não é atualizado automaticamente.">Saldo / data</Def></th>
                  <th scope="col" className="num"><Def text="Soma de todas as aplicações, sem descontar os resgates.">Total aplicado</Def></th>
                  <th scope="col" className="num"><Def text="Saldo + resgates − aplicações. Para calcular, informe os três dados na edição da caixinha; a taxa CDI só serve para simular.">Rendimento acumulado</Def></th>
                  <th scope="col" className="num"><Def text="Remuneração contratada. Exemplo: 120% do CDI paga 1,2 vez a taxa CDI; não significa 120% ao ano.">% do CDI</Def></th>
                  <th scope="col"><span className="sr-only">Ações</span></th>
                </tr></thead>
                <tbody>{pockets.map((item) => {
                  const active = item.id === selected;
                  const institution = item.institution || 'Instituição não informada';
                  const actual = performance?.items.find((entry) => entry.id === item.id);
                  return <tr key={item.id} className={active ? 'is-selected' : undefined}>
                    <td className="cash-name"><span title={item.name}>{item.name}</span><small title={institution}>{institution}</small></td>
                    <td className="num">{money(item.balance)}<small className="cash-balance-date">{item.balanceDate ? dateBR(item.balanceDate) : 'Data não informada'}</small></td>
                    <td className="num cash-applied">{actual?.invested == null ? '—' : money(actual.invested)}</td>
                    <td className={`num ${cashResultClass(actual?.income)}`}>{cashSignedMoney(actual?.income)}{actual && actual.income === null && <small className="cash-incomplete">Complete na edição</small>}</td>
                    <td className="num">{exactPercent(item.cdiPercent)}</td>
                    <td className="cash-row-actions">
                      <button type="button" className="button ghost small cash-pick" aria-pressed={active} aria-label={`Simular ${item.name}`} onClick={() => { if (!active) selectPocket(item.id); }} disabled={simulating}>Simular</button>
                      <button type="button" className="icon-button" onClick={() => setEditing(item)} disabled={simulating} aria-label={`Editar caixinha ${item.name}`} title="Editar"><Pencil size={16} /></button>
                    </td>
                  </tr>;
                })}</tbody>
              </table>
            </div>
          </div>
          <details className="extra-details span-12 cash-details"><summary>Taxas, observações e fontes</summary>
            <dl>{pockets.map((item) => <div key={item.id}>
              <dt>{item.name}</dt>
              <dd>Saldo em {item.balanceDate ? dateBR(item.balanceDate) : 'data não informada'} · {money(item.balance)}{item.totalWithdrawals != null && <> · Total resgatado: <span className="num">{money(item.totalWithdrawals)}</span></>}</dd>
              <dd><Def text="Taxa anual usada apenas na simulação, com sua data de referência. Não muda o saldo cadastrado.">CDI de referência</Def>: {item.annualCdiRate === null ? 'Não informado' : <><span className="num">{exactPercent(item.annualCdiRate)}</span> a.a. · {item.rateSource === 'bcb-sgs' ? 'Banco Central' : 'Manual'}{item.rateDate && <> · {dateBR(item.rateDate)}</>}</>}</dd>
              {item.notes && <dd className="cash-details-notes">{item.notes}</dd>}
              {item.rateSource === 'bcb-sgs' && <dd className="cash-details-source">CDI do Banco Central · SGS{item.rateFetchedAt && <>, consultado em <Stamp value={item.rateFetchedAt} /></>}{item.rateSourceUrl && <> · <span className="cash-source-link"><PublicSourceLink href={item.rateSourceUrl}>Ver série na fonte</PublicSourceLink></span></>}</dd>}
            </div>)}</dl>
          </details>
        </div>
      </section>
      {pocket && <section className="group cash-simulator" aria-labelledby="cash-sim-title">
        <div className="group-head"><h2 id="cash-sim-title"><Def text="Estima o rendimento com uma taxa CDI constante e dias úteis. O valor é bruto, antes de impostos e taxas; a taxa real pode mudar.">Simular rendimento</Def></h2></div>
        <div className="modular">
          <div className="block span-4 cash-sim-setup">
            <SectionHeading level={3} title="O que simular" />
            <form className="block-body cash-sim-form" onSubmit={handleForm(simulate)}>
              <Field label="Caixinha"><select value={selected} disabled={simulating} onChange={(event) => selectPocket(event.target.value)}>{pockets.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}</select></Field>
              <Field label="Prazo em dias úteis" hint="Informe dias úteis, sem contar fins de semana e feriados. A conta usa 252 dias úteis por ano."><input type="number" min="0" max="2520" step="1" value={days} disabled={simulating} required onChange={(event) => { setDays(event.target.value); setSimulation(null); }} /></Field>
              {!configured && <p className="alert warning cash-rate-missing">Para simular, informe o CDI anual de referência e a data dessa taxa.{' '}<button type="button" className="text-link" onClick={() => { focusSimulateAfterSave.current = true; setEditing(pocket); }} disabled={simulating}>Editar caixinha</button></p>}
              <button ref={simulateButton} className="button full-width" disabled={!desktop || !configured || simulating}>{simulating && <LoaderCircle className="spin" size={16} />}{simulating ? 'Calculando…' : 'Calcular rendimento bruto'}</button>
            </form>
          </div>
          <div className="block span-8 cash-sim-result">
            <SectionHeading level={3} title="Resultado" meta={pocket.name} />
            <div className="cash-sim-output" aria-live="polite">
              {simError && <p className="form-error cash-sim-error" role="alert">{simError}</p>}
              {simulation ? <SimulationResult simulation={simulation} pocket={pocket} /> : <SimulationPending pocket={pocket} configured={configured} />}
            </div>

          </div>
        </div>
      </section>}
    </> : <section className="block cash-empty"><Empty title={`Nenhuma caixinha em ${walletName}`} action={<button className="button" disabled={!desktop || !walletId || simulating} onClick={() => setEditing('new')}>Cadastrar caixinha</button>}>Informe onde o dinheiro está guardado, o saldo atual e a porcentagem do CDI contratada, por exemplo 120% do CDI. As caixinhas de outras carteiras continuam separadas.</Empty></section>}
    {editing && <CashPocketForm key={`${walletId}:${editing === 'new' ? 'new' : editing.id}`} walletId={walletId} pocket={editing === 'new' ? undefined : editing} onClose={() => { focusSimulateAfterSave.current = false; setEditing(null); }} onSaved={async () => { setEditing(null); setSimulation(null); await load(); await onSaved(); notify('Caixinha salva na carteira selecionada.'); }} />}
  </div>;
}

/** Premissa da simulação: % do CDI, CDI anual, origem e data da taxa. */
function Premise({ cdiPercent, annualCdiRate, rateDate, source }: { cdiPercent: string; annualCdiRate: string; rateDate: string | null; source: CashPocket['rateSource'] }) {
  return <>Premissa: <span className="num">{exactPercent(cdiPercent)}</span> de um CDI de <span className="num">{exactPercent(annualCdiRate)}</span> a.a. <span className="cash-nowrap">({source === 'bcb-sgs' ? 'Banco Central' : 'manual'}{rateDate && <>, ref. <span className="num">{dateBR(rateDate)}</span></>})</span></>;
}

/** Estado antes do cálculo: mesma estrutura do resultado, com o que já se sabe da caixinha. */
function SimulationPending({ pocket, configured }: { pocket: CashPocket; configured: boolean }) {
  const pending = <><span aria-hidden="true">—</span><span className="sr-only">ainda não calculado</span></>;
  return <>
    <dl className="cash-sim-readouts">
      <div className="readout cash-sim-lead">
        <dt className="figure-label"><Def text="Saldo inicial mais os juros simulados, antes de impostos, tarifas ou regras promocionais. O saldo cadastrado permanece igual.">Saldo bruto estimado</Def></dt>
        <dd className="figure-value lg muted">{pending}</dd>
        {!configured && <dd className="figure-note">Falta a taxa de referência.</dd>}
      </div>
      <div className="readout"><dt className="figure-label">Saldo inicial</dt><dd className="figure-value"><span className="unit">R$</span>{amount(pocket.balance)}</dd></div>
      <div className="readout"><dt className="figure-label"><Def text="Ganho estimado no período, antes de impostos e taxas. Ele não é adicionado ao seu saldo automaticamente.">Rendimento bruto</Def></dt><dd className="figure-value muted">{pending}</dd></div>
    </dl>
    {configured && pocket.annualCdiRate !== null && <details className="extra-details cash-sim-details"><summary>Ver premissas do cálculo</summary><div><Premise cdiPercent={pocket.cdiPercent} annualCdiRate={pocket.annualCdiRate} rateDate={pocket.rateDate} source={pocket.rateSource} /></div></details>}
  </>;
}

function SimulationResult({ simulation, pocket }: { simulation: CashSimulation; pocket: CashPocket }) {
  const gain = Number(simulation.grossEarnings) || 0;
  const start = Number(simulation.startingBalance) || 0;
  const tone = gain > 0 ? 'value-positive' : gain < 0 ? 'value-negative' : '';
  const sign = gain > 0 ? '+' : gain < 0 ? '-' : '';
  return <>
    <dl className="cash-sim-readouts">
      <div className="readout cash-sim-lead">
        <dt className="figure-label"><Def text="Saldo inicial mais o rendimento simulado. É um valor bruto, antes de impostos e taxas, com a taxa CDI constante.">Saldo bruto estimado</Def> após <span className="num">{simulation.businessDays}</span> dias úteis</dt>
        <dd className="figure-value lg"><span className="unit">R$</span>{amount(simulation.endingBalance)}</dd>

      </div>
      <div className="readout"><dt className="figure-label">Saldo inicial</dt><dd className="figure-value"><span className="unit">R$</span>{amount(simulation.startingBalance)}</dd></div>
      <div className="readout"><dt className="figure-label"><Def text="Ganho estimado no período, antes de impostos e taxas. Ele não é adicionado ao seu saldo automaticamente.">Rendimento bruto</Def></dt><dd className={`figure-value ${tone}`}><span className="unit">R$</span>{sign}{amount(Math.abs(gain))}</dd>{start > 0 && <dd className="figure-note"><span className="num">{sign}{percent(Math.abs(gain) / start * 100)}</span> no período</dd>}</div>
    </dl>
    <details className="extra-details cash-sim-details"><summary>Ver premissas do cálculo</summary><div><Premise cdiPercent={simulation.cdiPercent} annualCdiRate={simulation.annualCdiRate} rateDate={simulation.rateDate} source={pocket.rateSource} />{simulation.assumptions.length > 0 && <ul>{simulation.assumptions.map((assumption, index) => <li key={index}>{assumption}</li>)}</ul>}<p>Sem desconto de impostos, tarifas, inflação ou regras promocionais.</p></div></details>
  </>;
}

function CashPocketForm({ walletId, pocket, onClose, onSaved }: { walletId: string; pocket?: CashPocket; onClose: () => void; onSaved: () => Promise<void> }) {
  const [values, setValues] = useState({ name: pocket?.name || '', institution: pocket?.institution || '', balance: pocket?.balance || '', totalDeposits: pocket?.totalDeposits ?? '', totalWithdrawals: pocket?.totalWithdrawals ?? '', balanceDate: pocket?.balanceDate ?? '', cdiPercent: pocket?.cdiPercent || '100', annualCdiRate: pocket?.annualCdiRate ?? '', rateDate: pocket?.rateDate ?? '', notes: pocket?.notes || '' });
  const hasSavedReconciliation = pocket?.totalDeposits != null || pocket?.totalWithdrawals != null || pocket?.balanceDate != null;
  const [clearReconciliation, setClearReconciliation] = useState(false);
  const [cashFlows, setCashFlows] = useState<CashFlow[]>(pocket?.cashFlows ?? []);
  const [flowsEdited, setFlowsEdited] = useState(false);
  const updateFlow = (index: number, patch: Partial<CashFlow>) => {
    setCashFlows((current) => current.map((flow, row) => row === index ? { ...flow, ...patch } : flow));
    setFlowsEdited(true); setClearReconciliation(false);
  };
  const [source, setSource] = useState<{ rateSource: 'manual' | 'bcb-sgs'; rateSourceUrl: string | null; rateFetchedAt: string | null }>({ rateSource: pocket?.rateSource ?? 'manual', rateSourceUrl: pocket?.rateSourceUrl ?? null, rateFetchedAt: pocket?.rateFetchedAt ?? null });
  const [busy, setBusy] = useState(false);
  const [fetching, setFetching] = useState(false);
  const [error, setError] = useState('');
  const [referenceNotice, setReferenceNotice] = useState('');
  const request = useRef(0);
  const savePending = useRef(false);
  const rateRevision = useRef(0);
  const nameInput = useRef<HTMLInputElement>(null);
  useEffect(() => () => { request.current += 1; }, []);
  // O Modal abre o diálogo (showModal) no efeito dele, que roda antes deste e move o foco
  // para o primeiro controle; devolve o foco ao nome, como pede o autoFocus.
  useEffect(() => { nameInput.current?.focus(); }, []);

  const update = (name: keyof typeof values, value: string) => {
    if (name === 'totalDeposits' || name === 'totalWithdrawals' || name === 'balanceDate') setClearReconciliation(false);
    if (name === 'annualCdiRate' || name === 'rateDate') {
      rateRevision.current += 1;
      setSource({ rateSource: 'manual', rateSourceUrl: null, rateFetchedAt: null });
      setReferenceNotice('');
    }
    setValues((current) => ({ ...current, [name]: value }));
  };

  const fetchReference = async () => {
    const current = ++request.current;
    const revision = rateRevision.current;
    setFetching(true); setError(''); setReferenceNotice('');
    try {
      const reference = await call<EconomicIndicator>('fetch_cdi_reference');
      if (current !== request.current) return;
      if (revision !== rateRevision.current) {
        setReferenceNotice('A referência foi editada durante a consulta. Mantivemos os valores que você digitou.');
        return;
      }
      setValues((previous) => ({ ...previous, annualCdiRate: reference.value, rateDate: reference.referenceDate }));
      setSource({ rateSource: 'bcb-sgs', rateSourceUrl: reference.sourceUrl, rateFetchedAt: reference.fetchedAt });
      setReferenceNotice(`Referência de ${dateBR(reference.referenceDate)} preenchida. Salve a caixinha para guardar a taxa.`);
    } catch (err) { if (current === request.current) setError(`Não foi possível consultar o CDI: ${errorMessage(err)} Você pode informar a taxa manualmente.`); }
    finally { if (current === request.current) setFetching(false); }
  };

  const save = async () => {
    if (fetching || savePending.current) return;
    setError('');
    if (Boolean(values.annualCdiRate.trim()) !== Boolean(values.rateDate)) { setError('Preencha o CDI anual e a data de referência juntos, ou deixe ambos vazios.'); return; }
    const reconciliationFields = [values.totalDeposits.trim(), values.totalWithdrawals.trim(), values.balanceDate];
    const filled = reconciliationFields.filter(Boolean).length;
    if (filled !== 0 && filled !== 3) { setError('Para calcular o rendimento, preencha total aplicado, total resgatado e data do saldo juntos. Se não houve resgate, informe 0.'); return; }
    if (!filled && hasSavedReconciliation && !clearReconciliation) { setError('Para apagar os dados já cadastrados, use “Remover dados de rendimento”, ou preencha novamente os três campos.'); return; }
    const hasFlows = cashFlows.length > 0 || flowsEdited || pocket?.cashFlows != null;
    if (!clearReconciliation && hasFlows && filled !== 3) { setError('Para comparar com o CDI, informe também os totais aplicados e resgatados e a data do saldo.'); return; }
    if (!clearReconciliation && cashFlows.some((flow) => !flow.date || !flow.amount.trim())) { setError('Preencha a data e o valor de cada movimento, ou remova a linha incompleta.'); return; }
    const movements = !clearReconciliation && hasFlows ? { cashFlows: cashFlows.map((flow) => ({ ...flow, amount: decimal(flow.amount) })) } : {};
    const reconciliation = filled === 3 ? { totalDeposits: decimal(values.totalDeposits), totalWithdrawals: decimal(values.totalWithdrawals), balanceDate: values.balanceDate } : clearReconciliation ? { clearReconciliation: true } : {};
    savePending.current = true;
    setBusy(true);
    try {
      await call('save_cash_pocket', { walletId, input: { id: pocket?.id, name: values.name.trim(), institution: values.institution.trim(), balance: decimal(values.balance), ...reconciliation, ...movements, cdiPercent: decimal(values.cdiPercent), annualCdiRate: values.annualCdiRate.trim() ? decimal(values.annualCdiRate) : null, rateDate: values.rateDate || null, ...source, notes: values.notes.trim() } });
      await onSaved();
    } catch (err) { setError(errorMessage(err)); } finally { savePending.current = false; setBusy(false); }
  };

  return <Modal title={pocket ? 'Editar caixinha' : 'Nova caixinha'} description={pocket ? 'Atualize o saldo e as condições da reserva.' : 'Registre o saldo e as condições da reserva.'} busy={busy} onClose={() => { if (!busy) onClose(); }}>
    <form className="cash-form" onSubmit={handleForm(save)}>
      <Field label="Nome da caixinha"><input ref={nameInput} autoFocus value={values.name} onChange={(event) => update('name', event.target.value)} placeholder="Ex.: Reserva de emergência" required maxLength={120} disabled={busy} /></Field>
      <Field label="Instituição"><input value={values.institution} onChange={(event) => update('institution', event.target.value)} placeholder="Ex.: Mercado Pago" maxLength={120} required disabled={busy} /></Field>
      <div className="form-grid"><Field label="Saldo atual (R$)" hint="Use vírgula ou ponto para os centavos, sem separador de milhar."><input inputMode="decimal" value={values.balance} onChange={(event) => update('balance', event.target.value)} placeholder="0,00" required disabled={busy} /></Field><Field label="Porcentagem do CDI" hint="120 significa 120% do CDI, não 120% ao ano."><input inputMode="decimal" value={values.cdiPercent} onChange={(event) => update('cdiPercent', event.target.value)} placeholder="Ex.: 120" required disabled={busy} /></Field></div>
      <fieldset className="cash-rate cash-reconciliation">
        <legend>Para calcular o rendimento <span>opcional</span></legend>
        <p className="cash-form-explanation">Rendimento = saldo + resgates − aplicações. Informe os totais desde o início da caixinha até a data do saldo. Resgates incluem os juros retirados.</p>
        <div className="form-grid"><Field label="Total aplicado (R$)" hint="Some todas as aplicações, inclusive as que depois foram resgatadas. Não inclua juros creditados pelo banco como aplicação."><input inputMode="decimal" value={values.totalDeposits} onChange={(event) => update('totalDeposits', event.target.value)} placeholder="Não informado" disabled={busy} /></Field><Field label="Total resgatado (R$)" hint="Some tudo que saiu da caixinha, incluindo juros retirados. Se nunca resgatou, preencha 0."><input inputMode="decimal" value={values.totalWithdrawals} onChange={(event) => update('totalWithdrawals', event.target.value)} placeholder="0 se nunca resgatou" disabled={busy} /></Field></div>
        <div className="cash-date-entry"><Field label="Data do saldo" hint="Data em que o saldo informado acima foi conferido no banco. Não é a data da taxa CDI."><input type="date" value={values.balanceDate} onChange={(event) => update('balanceDate', event.target.value)} disabled={busy} /></Field><button type="button" className="button small secondary" disabled={busy} onClick={() => { const today = new Date(); update('balanceDate', `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, '0')}-${String(today.getDate()).padStart(2, '0')}`); }}>Usar data de hoje</button></div>
        <p className="cash-form-explanation">Campo vazio significa dado desconhecido. Para calcular, preencha os três campos; zero precisa ser informado.</p>
        {hasSavedReconciliation && <button type="button" className="text-link cash-remove-reconciliation" disabled={busy || clearReconciliation} onClick={() => { setValues((current) => ({ ...current, totalDeposits: '', totalWithdrawals: '', balanceDate: '' })); setClearReconciliation(true); setCashFlows([]); setFlowsEdited(false); setError(''); }}>Remover dados de rendimento</button>}
        {clearReconciliation && <p className="cash-reference-notice" role="status">Ao salvar, aplicações, resgates, datas dos movimentos e data do saldo serão removidos. O saldo e a taxa contratada continuam cadastrados; o rendimento ficará desconhecido.</p>}
      </fieldset>
      <fieldset className="cash-rate cash-flows">
        <legend>Datas para comparar com o CDI <span>opcional</span></legend>
        <p className="cash-form-explanation">Informe cada aplicação e resgate com a data em que aconteceu. Assim, a comparação usa o mesmo dinheiro pelo mesmo tempo. A soma deve bater com os totais acima; juros que ficaram na caixinha não são uma nova aplicação.</p>
        {cashFlows.length > 0 ? <div className="table-wrap cash-flow-table"><table>
          <caption className="sr-only">Aplicações e resgates com data</caption>
          <thead><tr><th scope="col">Data</th><th scope="col">Movimento</th><th scope="col">Valor (R$)</th><th scope="col"><span className="sr-only">Ações</span></th></tr></thead>
          <tbody>{cashFlows.map((flow, index) => <tr key={index}>
            <td data-label="Data"><input type="date" aria-label={`Data do movimento ${index + 1}`} value={flow.date} max={values.balanceDate || undefined} onChange={(event) => updateFlow(index, { date: event.target.value })} disabled={busy} /></td>
            <td data-label="Movimento"><select aria-label={`Tipo do movimento ${index + 1}`} value={flow.kind} onChange={(event) => updateFlow(index, { kind: event.target.value as CashFlow['kind'] })} disabled={busy}><option value="deposit">Aplicação</option><option value="withdrawal">Resgate</option></select></td>
            <td data-label="Valor (R$)"><input type="text" inputMode="decimal" aria-label={`Valor do movimento ${index + 1}`} value={flow.amount} placeholder="0,00" onChange={(event) => updateFlow(index, { amount: event.target.value })} disabled={busy} /></td>
            <td><button type="button" className="icon-button" aria-label={`Remover movimento ${index + 1}`} disabled={busy} onClick={() => { setCashFlows((current) => current.filter((_, row) => row !== index)); setFlowsEdited(true); }}><Trash2 size={15} /></button></td>
          </tr>)}</tbody>
        </table></div> : <p className="cash-form-explanation">Sem essas datas, o rendimento cadastrado continua nos totais. A comparação com o CDI fica aguardando os movimentos.</p>}
        <button type="button" className="button small cash-add-flow" disabled={busy} onClick={() => { setCashFlows((current) => [...current, { date: '', kind: 'deposit', amount: '' }]); setFlowsEdited(true); setClearReconciliation(false); }}><Plus size={14} />Adicionar movimento</button>
      </fieldset>
      <fieldset className="cash-rate">
        <legend>CDI para simular <span>opcional</span></legend>
        <div className="cash-fetch"><button className="button small" type="button" onClick={() => void fetchReference()} disabled={!desktop || busy || fetching}>{fetching && <LoaderCircle className="spin" size={14} />}{fetching ? 'Consultando CDI…' : 'Buscar CDI no Banco Central'}</button><Def text="Preenche a taxa e a data com a última referência do Banco Central. O saldo e o percentual contratado continuam como você informou; só a taxa é salva quando você confirmar.">Sobre esta consulta</Def></div>
        <div className="form-grid"><Field label="CDI anual (% a.a.)" hint="Ex.: 14,90"><input inputMode="decimal" value={values.annualCdiRate} onChange={(event) => update('annualCdiRate', event.target.value)} placeholder="Taxa de referência" disabled={busy} /></Field><Field label="Data dessa taxa"><input type="date" value={values.rateDate} onChange={(event) => update('rateDate', event.target.value)} disabled={busy} /></Field></div>
        {source.rateSource === 'bcb-sgs' && <div className="cash-provenance"><strong>Banco Central · SGS · CDI anualizado, base de 252 dias úteis</strong><span>{source.rateFetchedAt ? <>Consultada em <Stamp value={source.rateFetchedAt} /> · a referência pode ser anterior a hoje.</> : 'A referência pode ser anterior a hoje.'}</span>{source.rateSourceUrl && <span className="cash-source-link"><PublicSourceLink href={source.rateSourceUrl}>Ver série na fonte</PublicSourceLink></span>}</div>}
        {referenceNotice && <p className="cash-reference-notice" role="status">{referenceNotice}</p>}
      </fieldset>
      <Field label="Observações" hint="Registre limites de remuneração, carência ou condições promocionais."><textarea value={values.notes} onChange={(event) => update('notes', event.target.value)} rows={3} maxLength={4000} disabled={busy} /></Field>
      {error && <p className="form-error" role="alert">{error}</p>}
      <div className="form-actions"><button className="button secondary" type="button" onClick={onClose} disabled={busy}>Cancelar</button><button className="button primary" type="submit" disabled={busy || fetching}>{busy && <LoaderCircle className="spin" size={16} />}{busy ? 'Salvando…' : 'Salvar caixinha'}</button></div>
    </form>
  </Modal>;
}

function cashResultClass(value: string | null | undefined) { return value == null || Number(value) === 0 ? '' : Number(value) > 0 ? 'value-positive' : 'value-negative'; }
function cashSignedMoney(value: string | null | undefined) { return value == null ? '—' : `${Number(value) > 0 ? '+' : Number(value) < 0 ? '−' : ''}${money(Math.abs(Number(value)))}`; }

/** Percentual sem zeros decimais desnecessários e sem arredondar o valor informado (115,555% continua 115,555%). */
function exactPercent(value: string | number) { return `${new Intl.NumberFormat('pt-BR', { minimumFractionDigits: 0, maximumFractionDigits: 8 }).format(Number(value) || 0)}%`; }
/** Data e hora de uma consulta, com os números em mono: "16/10/2026 às 09:00". */
function Stamp({ value }: { value: string }) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return <span className="num">{value}</span>;
  return <><span className="num">{date.toLocaleDateString('pt-BR')}</span> às <span className="num">{date.toLocaleTimeString('pt-BR', { hour: '2-digit', minute: '2-digit' })}</span></>;
}

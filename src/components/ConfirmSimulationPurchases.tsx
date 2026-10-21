import { useEffect, useRef, useState } from 'react';
import { LoaderCircle } from 'lucide-react';
import { Field, Modal } from '../components';
import { call, dateBR, desktop, errorMessage } from '../lib/bridge';
import type { FundContributionResult, FundContributionRow } from '../lib/fundContributionSimulation';
import { simulationNumber } from '../lib/fundContributionSimulation';
import type { PurchaseConfirmationReport, PurchaseDraft, SimulationPurchaseInput } from '../lib/purchaseConfirmation';
import { eligibleSimulationFunds, localToday, purchaseDrafts, purchasesFromDrafts, purchaseStatusDescription, purchaseStatusLabel } from '../lib/purchaseConfirmation';

interface Props {
  result: FundContributionResult;
  toolId: string;
  conversationId: string;
  onPurchasesSaved?: (walletId: string) => Promise<void>;
  onBusyChange?: (busy: boolean) => void;
}

const money = (value: string) => `R$ ${simulationNumber(value, 2)}`;

function PurchaseReport({ report, receipt = false }: { report: PurchaseConfirmationReport; receipt?: boolean }) {
  return <div className="table-wrap purchase-confirmation-table" role="region" aria-label={receipt ? 'Compras já registradas desta simulação' : 'Conferência dos lançamentos de compra'} tabIndex={0}>
    <table><caption>{receipt ? 'Compras registradas desta simulação' : 'Confira antes de salvar'}</caption><thead><tr><th scope="col">Fundo e data real</th><th scope="col">Quantidade e preço real</th><th scope="col">Total da compra</th><th scope="col">Situação</th></tr></thead><tbody>{report.rows.map(row => <tr key={row.fundId}>
      <th scope="row"><span className="ticker">{row.ticker}</span><span className="fund-simulation-note"><time dateTime={row.purchaseDate}>{dateBR(row.purchaseDate)}</time></span></th>
      <td><span className="fund-simulation-number">{simulationNumber(row.shares)} cotas</span><span className="fund-simulation-note">{money(row.pricePerShare)} por cota</span></td>
      <td className="fund-simulation-number">{money(row.total)}</td>
      <td><strong className={`purchase-status purchase-status-${row.status}`}>{purchaseStatusLabel(row.status)}</strong><span className="fund-simulation-note">{purchaseStatusDescription(row)}</span></td>
    </tr>)}</tbody></table>
  </div>;
}

function PurchaseForm({ result, toolId, conversationId, funds, receipts, onClose, onReport, onPurchasesSaved, onBusyChange }: Props & { funds: FundContributionRow[]; receipts: PurchaseConfirmationReport; onClose: () => void; onReport: (report: PurchaseConfirmationReport) => void }) {
  const [drafts, setDrafts] = useState(() => purchaseDrafts(funds));
  const [preview, setPreview] = useState<{ report: PurchaseConfirmationReport; input: SimulationPurchaseInput } | null>(null);
  const [completed, setCompleted] = useState<PurchaseConfirmationReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [refreshError, setRefreshError] = useState('');
  const pending = useRef(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const recorded = new Map(receipts.rows.map(row => [row.fundId, row]));
  const update = (fundId: string, change: Partial<PurchaseDraft>) => {
    if (pending.current || completed) return;
    setDrafts(rows => rows.map(row => row.fundId === fundId ? { ...row, ...change } : row));
    setPreview(null); setError('');
  };
  const start = () => { pending.current = true; setBusy(true); setError(''); onBusyChange?.(true); };
  const finish = () => { pending.current = false; if (mounted.current) setBusy(false); onBusyChange?.(false); };
  const check = async () => {
    if (pending.current || completed) return;
    let input: SimulationPurchaseInput;
    try { input = { sourceToolId: toolId, purchases: purchasesFromDrafts(drafts) }; }
    catch (error) { setError(errorMessage(error)); return; }
    start(); setPreview(null);
    try {
      const report = await call<PurchaseConfirmationReport>('preview_simulation_purchases', { walletId: result.wallet.id, conversationId, input });
      if (report.rows.length > 0 && report.rows.every(row => row.status === 'already_confirmed')) {
        // A previous confirmation may have succeeded before its IPC reply was lost.
        // Reconcile the durable receipt without issuing another write.
        onReport(report);
        if (mounted.current) setCompleted(report);
        await refresh();
      } else if (mounted.current) setPreview({ report, input });
    } catch (error) { if (mounted.current) setError(errorMessage(error)); }
    finally { finish(); }
  };
  const refresh = async () => {
    if (!onPurchasesSaved) return;
    try { await onPurchasesSaved(result.wallet.id); if (mounted.current) setRefreshError(''); }
    catch (error) { if (mounted.current) setRefreshError(`As compras foram registradas. A tela da carteira ainda não foi atualizada: ${errorMessage(error)}`); }
  };
  const confirm = async () => {
    if (pending.current || completed || !preview?.report.canConfirm || !preview.report.previewToken) return;
    start();
    try {
      const report = await call<PurchaseConfirmationReport>('confirm_simulation_purchases', { walletId: result.wallet.id, conversationId, input: preview.input, previewToken: preview.report.previewToken });
      if (report.rows.some(row => row.status !== 'already_confirmed')) {
        if (mounted.current) { setPreview({ report, input: preview.input }); setError('A confirmação não foi concluída. Confira a situação das compras abaixo antes de continuar.'); }
        return;
      }
      // Saving succeeded. Refresh failures must never turn this into a failed purchase.
      onReport(report);
      if (mounted.current) { setCompleted(report); setPreview(null); }
      await refresh();
    } catch (error) {
      if (mounted.current) { setError(errorMessage(error)); setPreview(null); }
    } finally { finish(); }
  };
  const retryRefresh = async () => { if (pending.current) return; start(); try { await refresh(); } finally { finish(); } };
  const readonly = busy || completed !== null;
  return <Modal title="Registrar compras realizadas" description={`Carteira da simulação: ${result.wallet.name}`} busy={busy} onClose={onClose}>
    <form className="purchase-confirmation-form" onSubmit={event => { event.preventDefault(); void check(); }} aria-busy={busy || undefined}>
      <p className="purchase-confirmation-intro">Selecione só as compras que você realizou. Informe a data, a quantidade e o preço reais; eles podem ser diferentes da simulação.</p>
      {!completed && funds.map((fund, index) => {
        const draft = drafts[index];
        const saved = recorded.get(fund.fundId!);
        return <fieldset className="purchase-confirmation-fund" key={fund.fundId} disabled={readonly || Boolean(saved)}>
          <legend><label><input type="checkbox" checked={draft.selected} disabled={readonly || Boolean(saved)} onChange={event => update(draft.fundId, { selected: event.target.checked })} /><span className="ticker">{fund.symbol}</span><span>{saved ? purchaseStatusLabel(saved.status) : 'Comprei este fundo'}</span></label></legend>
          {saved ? <p className={`purchase-status purchase-status-${saved.status}`}>{purchaseStatusDescription(saved)}</p> : <>
            <p className="purchase-confirmation-reference">Referência da simulação: {fund.sharesToBuy === null ? 'quantidade pendente' : `${simulationNumber(fund.sharesToBuy)} cotas`}{fund.price !== null && <> a {money(fund.price)} por cota</>}. Confira o preço executado na corretora.</p>
            <div className="purchase-confirmation-fields">
              <Field label={`Data real da compra · ${fund.symbol}`}><input type="date" min="1900-01-01" max={localToday()} required={draft.selected} disabled={!draft.selected || readonly} value={draft.purchaseDate} onChange={event => update(draft.fundId, { purchaseDate: event.target.value })} /></Field>
              <Field label={`Cotas compradas · ${fund.symbol}`}><input inputMode="numeric" pattern="[1-9][0-9]*" required={draft.selected} disabled={!draft.selected || readonly} value={draft.shares} onChange={event => update(draft.fundId, { shares: event.target.value })} /></Field>
              <Field label={`Preço real por cota (R$) · ${fund.symbol}`} hint="A sugestão vem da cotação da simulação. Substitua pelo preço real da compra; até duas casas decimais, sem separador de milhar."><input inputMode="decimal" required={draft.selected} disabled={!draft.selected || readonly} value={draft.pricePerShare} onChange={event => update(draft.fundId, { pricePerShare: event.target.value })} /></Field>
            </div>
          </>}
        </fieldset>;
      })}
      {preview && <div className="purchase-confirmation-preview" aria-live="polite"><PurchaseReport report={preview.report} />{preview.report.rows.some(row => row.status === 'conflict') && <p className="form-error" role="alert">Há compras que precisam ser conferidas. Nenhum lançamento desta seleção será salvo enquanto houver conflito.</p>}{preview.report.canConfirm && <p className="purchase-confirmation-intro">Confirme somente se os dados acima correspondem às operações realizadas. Os fundos não selecionados permanecem como simulação.</p>}</div>}
      {completed && <div className="purchase-confirmation-completed" aria-live="polite"><p className="purchase-confirmation-success">{completed.createdCount === 0 ? 'Compras confirmadas no histórico, sem criar lançamentos duplicados.' : <>Compras registradas: {completed.createdCount} novo{completed.createdCount === 1 ? '' : 's'} lançamento{completed.createdCount === 1 ? '' : 's'}{completed.matchedCount > 0 ? `; ${completed.matchedCount} já estava${completed.matchedCount === 1 ? '' : 'm'} no histórico, sem duplicar` : ''}.</>}</p><PurchaseReport report={completed} receipt /></div>}
      {error && <p className="form-error" role="alert">{error}</p>}
      {refreshError && <div role="status"><p className="form-error">{refreshError}</p><button className="text-link" type="button" disabled={busy} onClick={() => void retryRefresh()}>Atualizar a tela da carteira</button></div>}
      <div className="form-actions"><button className="button secondary" type="button" disabled={busy} onClick={onClose}>{completed ? 'Concluir' : 'Cancelar'}</button>{!completed && <button className="button secondary" type="submit" disabled={busy || funds.every(fund => recorded.has(fund.fundId!))}>{busy && <LoaderCircle size={15} className="spin" />}Conferir compras</button>}{!completed && preview?.report.canConfirm && <button className="button primary" type="button" disabled={busy || !preview.report.previewToken} onClick={() => void confirm()}>{busy && <LoaderCircle size={15} className="spin" />}Confirmar lançamentos</button>}</div>
    </form>
  </Modal>;
}

export default function ConfirmSimulationPurchases(props: Props) {
  const { result, toolId, conversationId } = props;
  const [report, setReport] = useState<PurchaseConfirmationReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [open, setOpen] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const requestVersion = useRef(0);
  const opening = useRef(false);
  const funds = eligibleSimulationFunds(result);
  useEffect(() => {
    const request = ++requestVersion.current;
    opening.current = false;
    setReport(null); setLoading(true); setError(''); setOpen(false);
    if (!desktop || !conversationId || !funds.length) { setLoading(false); return; }
    void call<PurchaseConfirmationReport>('get_simulation_purchase_confirmations', { walletId: result.wallet.id, conversationId, sourceToolId: toolId })
      .then(report => { if (request === requestVersion.current) setReport(report); })
      .catch(error => { if (request === requestVersion.current) setError(errorMessage(error)); })
      .finally(() => { if (request === requestVersion.current) setLoading(false); });
    return () => { requestVersion.current += 1; };
  }, [result.wallet.id, conversationId, toolId, funds.length, attempt]);
  const openPurchases = async () => {
    if (!desktop || !conversationId || loading || !report || opening.current) return;
    const request = ++requestVersion.current;
    opening.current = true; setLoading(true); setError('');
    try {
      // The assistant remains mounted while hidden. Re-read durable receipts so
      // edits made in the portfolio history appear here before any new action.
      const saved = await call<PurchaseConfirmationReport>('get_simulation_purchase_confirmations', { walletId: result.wallet.id, conversationId, sourceToolId: toolId });
      if (request === requestVersion.current) { setReport(saved); setOpen(true); }
    } catch (error) { if (request === requestVersion.current) setError(errorMessage(error)); }
    finally { if (request === requestVersion.current) { opening.current = false; setLoading(false); } }
  };
  if (!funds.length) return null;
  const recordReport = (next: PurchaseConfirmationReport) => setReport(previous => ({
    ...next,
    rows: [...(previous?.rows || []).filter(old => !next.rows.some(row => row.fundId === old.fundId)), ...next.rows],
    canConfirm: false,
  }));
  const remaining = funds.some(fund => !report?.rows.some(row => row.fundId === fund.fundId));
  return <div className="purchase-confirmation-actions">
    {report && report.rows.length > 0 && <PurchaseReport report={report} receipt />}
    <div className="purchase-confirmation-action-row"><span className="fund-simulation-foot">A compra só entra no histórico após você conferir e confirmar os dados reais.</span><button className="button small" type="button" disabled={!desktop || !conversationId || loading || Boolean(error) || !report} onClick={() => void openPurchases()}>{loading ? 'Consultando compras…' : remaining ? 'Registrar compras realizadas' : 'Ver compras registradas'}</button></div>
    {!desktop && <p className="fund-simulation-foot">O registro de compras está disponível no aplicativo Windows.</p>}
    {error && <div className="purchase-confirmation-load-error" role="alert"><p>Não foi possível consultar as compras desta simulação: {error}</p><button className="text-link" type="button" onClick={() => setAttempt(value => value + 1)}>Tentar novamente</button></div>}
    {open && report && <PurchaseForm {...props} funds={funds} receipts={report} onReport={recordReport} onClose={() => setOpen(false)} />}
  </div>;
}

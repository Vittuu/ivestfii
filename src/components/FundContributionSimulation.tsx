import { dateBR } from '../lib/bridge';
import { Def } from '../components';
import type { FundContributionResult, FundContributionRow } from '../lib/fundContributionSimulation';
import { simulationNumber } from '../lib/fundContributionSimulation';
import ConfirmSimulationPurchases from './ConfirmSimulationPurchases';
import '../styles/fund-contribution-simulation.css';

const money = (value: string | null) => value === null ? '—' : `R$ ${simulationNumber(value, 2)}`;
const percent = (value: string | null) => value === null ? '—' : simulationNumber(value, 2) + '%';
const pair = (before: string | null, after: string | null, format: (value: string | null) => string) => <>{format(before)} <span aria-label="para">→</span> {format(after)}</>;

function Quote({ row }: { row: FundContributionRow }) {
  if (row.price === null) return <span className="fund-simulation-note">Sem cotação</span>;
  const reference = row.quote?.observation;
  const description = [reference?.source ? `Fonte: ${reference.source}.` : '', reference?.quotedAt ? `Horário original da cotação: ${reference.quotedAt}.` : '', reference?.fetchedAt ? `Coleta: ${reference.fetchedAt}.` : '', 'Preço de referência da simulação; não é preço de execução.'].filter(Boolean).join(' ');
  return <span className="fund-simulation-note"><Def text={description}>{money(row.price)}</Def>{row.quoteDate && <> · <time dateTime={row.quoteDate}>{dateBR(row.quoteDate)}</time></>}{row.quoteStatus !== 'available' && <span className="fund-simulation-source">Cotação salva</span>}</span>;
}

export default function FundContributionSimulation({ result, toolId, conversationId, onPurchasesSaved, onBusyChange }: { result: FundContributionResult; toolId: string; conversationId: string; onPurchasesSaved?: (walletId: string) => Promise<void>; onBusyChange?: (busy: boolean) => void }) {
  const { report, wallet } = result;
  const allocation = report.portfolioAllocation;
  const incomplete = report.status === 'incomplete' || allocation.status === 'incomplete';
  const issues = [...report.issues, ...(allocation.issues || [])];
  const basis = allocation.valuationBasis === 'mixed-dates' ? `Saldos de datas diferentes${allocation.fromDate && allocation.toDate ? `: ${dateBR(allocation.fromDate)} a ${dateBR(allocation.toDate)}` : ''}.` : allocation.valuationBasis === 'undated' ? 'Há saldos sem data informada.' : allocation.asOf ? `Composição em ${dateBR(allocation.asOf)}.` : '';
  return <section className="fund-contribution-simulation" aria-label={`Simulação de aporte · ${wallet.name}`} data-tool-id={toolId} data-wallet-id={wallet.id}>
    <header className="fund-simulation-head"><h3>Simulação do aporte</h3><span>Carteira: <strong>{wallet.name}</strong></span></header>
    <dl className="fund-simulation-summary">
      <div><dt>Orçamento total</dt><dd>{money(result.budget)}</dd></div>
      <div><dt>Compras de FIIs</dt><dd>{money(report.spentAmount)}</dd></div>
      <div><dt>Saldo livre</dt><dd>{money(allocation.unallocatedCash)}</dd></div>
    </dl>
    {incomplete && <p className="fund-simulation-pending"><Def text={issues.join(' ') || 'Há dados ausentes. Os valores e pesos desconhecidos permanecem indisponíveis.'}>Há dados pendentes na simulação.</Def></p>}
    <div className="table-wrap" role="region" aria-label="Compras simuladas por fundo, role horizontalmente para ver todas as colunas" tabIndex={0}>
      <table className="fund-simulation-funds"><caption>FIIs · reserva de {money(report.budget)} · sobra de {money(report.remainder)}</caption><thead><tr><th scope="col">Fundo e cotação</th><th scope="col">Compra simulada</th><th scope="col">Composição antes → depois</th></tr></thead>
        <tbody>{!report.rows.length && <tr><td colSpan={3}>Nenhum FII nesta simulação.</td></tr>}{report.rows.map(row => <tr key={row.fundId || row.symbol}>
          <th scope="row"><span className="ticker">{row.symbol}</span><Quote row={row} />{!row.classificationKnown && <span className="fund-simulation-note">Segmento não confirmado</span>}</th>
          <td><span className="fund-simulation-number">{row.sharesToBuy === null ? 'Compra pendente' : `${simulationNumber(row.sharesToBuy)} cotas · ${money(row.spentAmount)}`}</span><span className="fund-simulation-note">Cotas: {pair(row.currentShares, row.afterShares, simulationNumber)}</span><span className="fund-simulation-note">Reservado: {money(row.budget)} · sobra: {money(row.remainder)}</span></td>
          <td><span className="fund-simulation-number">Nos FIIs: {pair(row.beforeFiiPercent, row.afterFiiPercent, percent)}</span><span className="fund-simulation-note">Na carteira: {pair(row.beforePortfolioPercent, row.afterPortfolioPercent, percent)}</span><span className="fund-simulation-note">Valor: {pair(row.currentValue, row.afterValue, money)}</span></td>
        </tr>)}</tbody>
      </table>
    </div>
    <div className="table-wrap" role="region" aria-label="Composição da carteira antes e depois do aporte" tabIndex={0}>
      <table className="fund-simulation-classes"><caption>Carteira inteira · antes → depois</caption><thead><tr><th scope="col">Destino</th><th scope="col">Valor</th><th scope="col">Peso na carteira</th></tr></thead><tbody>{allocation.rows.map(row => <tr key={row.kind}>
        <th scope="row">{row.kind === 'unallocatedCash' ? 'Saldo livre' : row.name}</th><td><span className="fund-simulation-number">{pair(row.currentValue, row.afterValue, money)}</span><span className="fund-simulation-note">Aporte: {money(row.contribution)}</span></td><td className="fund-simulation-number">{pair(row.beforePercent, row.afterPercent, percent)}</td>
      </tr>)}</tbody><tfoot><tr><th scope="row">Total</th><td className="fund-simulation-number">{pair(allocation.totalBefore, allocation.totalAfter, money)}</td><td>—</td></tr></tfoot></table>
    </div>
    <p className="fund-simulation-foot">{basis && <span>{basis} </span>}Cotações são referências, sem garantia de preço de execução. Saldo livre permanece disponível e não é aplicado em caixinhas.</p>
    <p className="fund-simulation-foot">Os valores acima preservam a simulação original. Compras confirmadas aparecem abaixo; “—” indica dado indisponível.</p>
    <ConfirmSimulationPurchases result={result} toolId={toolId} conversationId={conversationId} onPurchasesSaved={onPurchasesSaved} onBusyChange={onBusyChange} />
  </section>;
}

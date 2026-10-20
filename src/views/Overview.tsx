import { useId } from 'react';
import { Plus, Upload } from 'lucide-react';
import { desktop, money, monthShort, percent, quantity } from '../lib/bridge';
import type { Snapshot } from '../lib/bridge';
import { ledgerRows } from '../lib/ledger';
import { useMonthlyIncome } from '../lib/useMonthlyIncome';
import { Def, Empty, PageActions, SectionHeading, ShareBar, TextLink } from '../components';
import CompletePortfolio from './CompletePortfolio';
import FiiPerformancePanel from '../components/FiiPerformance';
import MarketSummary from '../components/MarketSummary';
import MonthlyIncomeChart from '../components/MonthlyIncomeChart';

type Target = 'portfolio' | 'assistant' | 'projections' | 'cash' | 'market';
interface Props { walletId: string; snapshot: Snapshot; onNavigate: (page: Target, portfolioTab?: 'position' | 'history') => void; onImport: () => void; onImportPosition?: () => void }

const cents = (value: number) => Math.round(value * 100) / 100;
const monthKey = (date: Date) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}`;
const perCota = (value: string | number) => new Intl.NumberFormat('pt-BR', { minimumFractionDigits: 2, maximumFractionDigits: 4 }).format(Number(value) || 0);
const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;

export default function Overview({ walletId, snapshot, onNavigate, onImport, onImportPosition }: Props) {
  const historyId = useId();
  const { funds, totals } = snapshot;
  const invested = Number(totals.invested);
  const sorted = [...funds].sort((a, b) => Number(b.totalInvested) - Number(a.totalInvested));
  const rows = funds.flatMap((fund) => ledgerRows(fund).map((row) => ({ ...row, ticker: fund.ticker, month: row.entry.month.slice(0, 7) })));

  const today = new Date();
  const currentKey = monthKey(today);
  const untilLabel = today.toLocaleDateString('pt-BR', { day: '2-digit', month: '2-digit' });
  const monthKeys = Array.from({ length: 12 }, (_, index) => monthKey(new Date(today.getFullYear(), today.getMonth() - 11 + index, 1)));
  const allMonthKeys = [monthKey(new Date(today.getFullYear(), today.getMonth() - 12, 1)), ...monthKeys];
  const allPoints = useMonthlyIncome(walletId, snapshot, allMonthKeys);
  const points = allPoints.slice(1);
  const period = `${monthShort(points[0].key)} a ${monthShort(currentKey)}`;
  const total12 = cents(points.reduce((sum, point) => sum + point.value, 0));
  const hasIncome = points.some((point) => point.incomeRecorded > 0);
  // Yield on cost sobre os 12 meses fechados: o mês corrente parcial não derruba o indicador no início de cada mês.
  const closedKeys = Array.from({ length: 12 }, (_, index) => monthKey(new Date(today.getFullYear(), today.getMonth() - 12 + index, 1)));
  const closedPoints = allPoints.slice(0, -1);
  const closed12 = cents(closedPoints.reduce((sum, point) => sum + point.value, 0));
  const yieldOnCost = invested > 0 && closedPoints.some((point) => point.incomeRecorded > 0) ? closed12 / invested * 100 : null;

  const fundRows = sorted.slice(0, 6);
  const others = sorted.slice(6);
  const recent = [...rows].sort((a, b) => b.month.localeCompare(a.month) || Number(b.dividend) - Number(a.dividend) || a.ticker.localeCompare(b.ticker)).slice(0, 4);

  return <div className="page overview-page">
    <PageActions>
      {onImportPosition && <button type="button" className="button" onClick={onImportPosition} disabled={!desktop} aria-label="Importar posição" title="Importar posição da corretora"><Upload size={16} /><span className="label-long">Importar posição</span></button>}
      <button type="button" className="button primary" onClick={() => onNavigate('portfolio', 'history')}><Plus size={16} />Novo lançamento</button>
    </PageActions>

    <CompletePortfolio position={snapshot.brokerPosition} onImportPosition={onImportPosition} showImportAction={false} />
    {desktop && <FiiPerformancePanel key={walletId} walletId={walletId} snapshot={snapshot} onProjection={() => onNavigate('projections')} onCash={() => onNavigate('cash')} />}
    {desktop && <MarketSummary snapshot={snapshot} onMarket={() => onNavigate('market')} />}

    <section className="group ov-history" aria-labelledby={historyId}>
      <div className="group-head"><h2 id={historyId}>Histórico de FIIs</h2>{snapshot.brokerPosition && <p><Def text="Custo de aquisição e proventos registrados nos lançamentos. Os proventos recebidos são o acumulado dos lançamentos, não saldo em conta.">Não somado ao patrimônio</Def></p>}</div>
      {funds.length ? <div className="ov-summary">
        <div className="modular">
        <div className="block span-12 ov-funds">
          <SectionHeading title="Fundos" level={3} meta={<><span className="num">{funds.length}</span> {funds.length === 1 ? 'fundo' : 'fundos'} · <span className="num">{quantity(totals.cotas)}</span> cotas</>}><TextLink onClick={() => onNavigate('portfolio')}>Abrir FIIs e posições</TextLink></SectionHeading>
          <div className="table-wrap">
            <table className="ov-table ov-funds-table">
              <caption className="sr-only">Fundos do histórico, do maior para o menor custo de aquisição</caption>
              <colgroup><col className="ov-col-fund" /><col className="ov-col-cotas" /><col className="ov-col-cost" /><col /><col className="ov-col-income" /><col className="ov-col-proceeds" /></colgroup>
              <thead><tr>
                <th scope="col">Fundo</th>
                <th scope="col" className="num">Cotas</th>
                <th scope="col" className="num ov-purchase"><Def text="Soma do dinheiro usado nas compras registradas deste fundo. Não é o preço atual nem o ganho do investimento.">Valor comprado</Def></th>
                <th scope="col"><Def text="Parte do dinheiro das compras que foi para este fundo. Exemplo: R$ 200 de R$ 1.000 comprados representam 20%. Não é preço atual nem ganho.">Parte do valor comprado</Def></th>
                <th scope="col" className="num"><Def text="Estimativa pelas cotas atuais e pela média dos últimos 6 proventos por cota registrados. Pode variar.">Renda estimada/mês</Def></th>
                <th scope="col" className="num ov-received"><Def text="Soma dos proventos registrados para este fundo. Não é saldo em conta.">Proventos recebidos</Def></th>
              </tr></thead>
              <tbody>{fundRows.map((fund) => <tr key={fund.id}>
                <td className="ticker" title={[fund.name, fund.sector].filter(Boolean).join(' · ') || undefined}>{fund.ticker}{(fund.name || fund.sector) && <span className="ov-fund-name"> {fund.name || fund.sector}</span>}</td>
                <td className="num">{quantity(fund.totalCotas)}</td>
                <td className="num ov-purchase">{money(fund.totalInvested)}</td>
                <td><ShareBar value={Number(fund.totalInvested) || 0} max={invested} label={percent(invested > 0 ? Number(fund.totalInvested) / invested * 100 : 0)} /></td>
                <td className="num">{money(fund.estimatedMonthlyIncome)}</td>
                <td className="num ov-received">{money(fund.dividendsReceived)}</td>
              </tr>)}</tbody>
            </table>
          </div>
          {others.length > 0 && <div className="block-foot">Mais {plural(others.length, 'fundo', 'fundos')}, com <span className="num">{money(others.reduce((sum, fund) => sum + (Number(fund.totalInvested) || 0), 0))}</span> de custo<TextLink onClick={() => onNavigate('portfolio', 'history')}>Ver todos</TextLink></div>}
        </div>

        <div className="block span-12 ov-recent">
          <SectionHeading title="Últimos lançamentos" level={3}><TextLink onClick={() => onNavigate('portfolio', 'history')}>Ver todos</TextLink></SectionHeading>
          {recent.length ? <div className="table-wrap">
            <table className="ov-table ov-recent-table">
              <caption className="sr-only">Lançamentos mais recentes do histórico</caption>
              <colgroup><col className="ov-col-month" /><col className="ov-col-ticker" /><col /><col /></colgroup>
              <thead><tr><th scope="col">Mês</th><th scope="col">Fundo</th><th scope="col" className="num ov-purchase"><Def text="Dinheiro usado para comprar as cotas deste lançamento. Um traço indica que não houve compra registrada nessa linha.">Compras</Def></th><th scope="col" className="num ov-received"><Def text="Total recebido neste lançamento: informado por você ou calculado pelo Histórico. Um traço indica que não houve rendimento positivo registrado nessa linha.">Rendimentos</Def></th></tr></thead>
              <tbody>{recent.map((row) => {
                const bought = row.kind === 'purchase' || row.kind === 'both';
                const received = row.kind === 'income' || row.kind === 'both';
                return <tr key={row.entry.id}>
                  <td className="ov-month" title={row.month === currentKey ? `Mês parcial, até ${untilLabel}` : undefined}><span className="num">{monthShort(row.month)}</span></td>
                  <td className="ticker">{row.ticker}</td>
                  <td className="num ov-purchase">{bought ? <>{money(row.purchaseCost)}<span className="ov-purchase-quantity">{quantity(row.entry.cotasAdded)} {Number(row.entry.cotasAdded) === 1 ? 'cota' : 'cotas'} × {money(row.entry.pricePerCota)}</span></> : <span className="dash">—</span>}</td>
                  <td className="num ov-received">{received ? row.computed ? <Def text={`Calculado: ${quantity(row.held)} cotas no mês × R$ ${perCota(row.entry.dividendPerCota)} por cota.`}>{money(row.dividend)}</Def> : money(row.dividend) : <span className="dash">—</span>}</td>
                </tr>;
              })}</tbody>
            </table>
          </div> : <Empty title="Nenhum lançamento">Registre compras e proventos em FIIs e posições.</Empty>}
        </div>
        </div>
        <details className="extra-details ov-extra">
          <summary>Como ler os valores do histórico</summary>
          <div className="modular">
            <div className="block span-4 ov-readouts-block">
              <p className="block-body ov-method-note">Compras mostram o dinheiro usado nos lançamentos. Rendimentos mostram o que foi recebido. Esses registros não atualizam a posição importada da corretora.</p>
              <div className="readouts ov-readouts"><div className="readout"><span className="figure-label"><Def text={`Soma dos proventos informados e calculados pela B3 de ${monthShort(closedKeys[0])} a ${monthShort(closedKeys[11])}, meses fechados, dividida pelo custo de aquisição. Meses sem dados não confirmam rendimento zero. Não inclui a variação do preço dos fundos.`}>Proventos sobre o custo (12 meses)</Def></span><span className="figure-value">{yieldOnCost === null ? '—' : percent(yieldOnCost)}</span></div></div>
            </div>
            <div className="block span-8 ov-income">
              <SectionHeading title="Proventos por mês" level={3}><span className="ov-head-figure"><Def text={`Soma dos rendimentos informados e preenchidos pela B3 de ${period}. Sem novas compras, as cotas anteriores continuam. Cada cálculo automático usa o provento real e as cotas com direito na data-com; um valor informado por você tem prioridade. ${monthShort(currentKey)} parcial, até ${untilLabel}, aparece hachurado.`}>em 12 meses</Def><span className="num">{hasIncome ? money(total12) : '—'}</span></span></SectionHeading>
              <MonthlyIncomeChart points={points} until={untilLabel} />
            </div>
          </div>
        </details>
      </div> : <div className="block">
        <Empty title="Nenhum fundo no histórico" action={<div className="ov-empty-actions"><button type="button" className="button" onClick={onImport} disabled={!desktop}><Upload size={16} />Importar histórico</button><button type="button" className="button" onClick={() => onNavigate('portfolio', 'history')}>Cadastrar fundo</button></div>}>
          {desktop ? 'Importe os dados do aplicativo anterior ou cadastre o primeiro fundo. Os registros ficam salvos neste computador.' : 'Abra o aplicativo Windows para acessar a carteira. A versão web usará a mesma interface quando a sincronização estiver disponível.'}
        </Empty>
      </div>}
    </section>
  </div>;
}

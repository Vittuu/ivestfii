import { useCallback, useId, useState } from 'react';
import { Plus, Upload } from 'lucide-react';
import { desktop, money, monthShort, percent, quantity } from '../lib/bridge';
import type { FiiPerformance, Snapshot } from '../lib/bridge';
import { recentLedgerRows } from '../lib/ledger';
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
  const incomeScope = JSON.stringify([walletId, funds.map((fund) => [fund.id, fund.ticker, fund.entries])]);
  const [performance, setPerformance] = useState<{ scope: string; data: FiiPerformance | null } | null>(null);
  const receivePerformance = useCallback((data: FiiPerformance | null) => setPerformance({ scope: incomeScope, data }), [incomeScope]);
  const income = performance?.scope === incomeScope ? performance.data?.income : null;
  const invested = Number(totals.invested);
  const sorted = [...funds].sort((a, b) => Number(b.totalInvested) - Number(a.totalInvested));

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
  const recent = recentLedgerRows(funds);

  return <div className="page overview-page">
    <PageActions>
      {onImportPosition && <button type="button" className="button" onClick={onImportPosition} disabled={!desktop} aria-label="Importar posição" title="Importar posição da corretora"><Upload size={16} /><span className="label-long">Importar posição</span></button>}
      <button type="button" className="button primary" onClick={() => onNavigate('portfolio', 'history')}><Plus size={16} />Novo lançamento</button>
    </PageActions>

    <CompletePortfolio position={snapshot.brokerPosition} onImportPosition={onImportPosition} showImportAction={false} onShowFixedIncome={() => onNavigate('portfolio', 'position')} />
    {desktop && <FiiPerformancePanel key={walletId} walletId={walletId} snapshot={snapshot} onProjection={() => onNavigate('projections')} onCash={() => onNavigate('cash')} onFixedIncome={() => onNavigate('portfolio', 'position')} onData={receivePerformance} />}
    {desktop && <MarketSummary snapshot={snapshot} onMarket={() => onNavigate('market')} />}

    <section className="group ov-history" aria-labelledby={historyId}>
      <div className="group-head"><h2 id={historyId}>Histórico de FIIs</h2>{snapshot.brokerPosition && <p><Def text="Compras registradas e proventos acumulados, incluindo os dados da B3 quando disponíveis. Estes valores não são somados novamente ao patrimônio importado da corretora.">Não somado ao patrimônio</Def></p>}</div>
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
                <th scope="col" className="num ov-received"><Def text={income ? `Proventos do histórico mais os pagamentos da B3 que faltavam, até ${income.asOf.split('-').reverse().join('/')}. Um valor informado por você tem prioridade no mesmo fundo e mês. Não é saldo em conta.` : 'Soma dos proventos registrados para este fundo. Não é saldo em conta.'}>{income ? 'Proventos acumulados' : 'Proventos recebidos'}</Def></th>
              </tr></thead>
              <tbody>{fundRows.map((fund) => <tr key={fund.id}>
                <td className="ticker" title={[fund.name, fund.sector].filter(Boolean).join(' · ') || undefined}>{fund.ticker}{(fund.name || fund.sector) && <span className="ov-fund-name"> {fund.name || fund.sector}</span>}</td>
                <td className="num">{quantity(fund.totalCotas)}</td>
                <td className="num ov-purchase">{money(fund.totalInvested)}</td>
                <td><ShareBar value={Number(fund.totalInvested) || 0} max={invested} label={percent(invested > 0 ? Number(fund.totalInvested) / invested * 100 : 0)} /></td>
                <td className="num">{money(fund.estimatedMonthlyIncome)}</td>
                <td className="num ov-received">{money(income?.byFund.find((item) => item.fundId === fund.id)?.totalAmount ?? fund.dividendsReceived)}</td>
              </tr>)}</tbody>
            </table>
          </div>
          {others.length > 0 && <div className="block-foot">Mais {plural(others.length, 'fundo', 'fundos')}, com <span className="num">{money(others.reduce((sum, fund) => sum + (Number(fund.totalInvested) || 0), 0))}</span> de custo<TextLink onClick={() => onNavigate('portfolio', 'history')}>Ver todos</TextLink></div>}
        </div>

        <div className="block span-12 ov-recent">
          <p className="ov-recent-note">Compras e rendimentos ficam separados. Receber um rendimento não significa que houve uma nova compra.</p>
          <div className="ov-recent-columns">
          <section className="ov-recent-purchases" aria-label="Últimas compras de FIIs">
            <SectionHeading title="Últimas compras" level={3}><TextLink onClick={() => onNavigate('portfolio', 'history')}>Ver todas</TextLink></SectionHeading>
            {recent.purchases.length ? <div className="table-wrap">
              <table className="ov-table ov-recent-table">
                <caption className="sr-only">Compras mais recentes, com a quantidade comprada e seu preço registrado</caption>
                <colgroup><col className="ov-col-purchase-date" /><col className="ov-col-ticker" /><col /></colgroup>
                <thead><tr><th scope="col"><Def text="Data informada na compra. Nos registros antigos sem o dia exato, mostramos apenas o mês; nenhum dia é presumido.">Data da compra</Def></th><th scope="col">Fundo</th><th scope="col" className="num ov-purchase"><Def text="Quantidade comprada neste lançamento multiplicada pelo preço por cota registrado. São compras efetivas, mesmo quando mais antigas que os últimos rendimentos.">Valor da compra</Def></th></tr></thead>
                <tbody>{recent.purchases.map((row) => <tr key={`${row.fundId}-${row.entry.id}`}>
                  <td className="ov-month">{row.entry.purchaseDate ? <time className="num" dateTime={row.entry.purchaseDate}>{row.entry.purchaseDate.split('-').reverse().join('/')}</time> : <><span className="num">{monthShort(row.entry.month.slice(0, 7))}</span><span className="ov-purchase-date-note">dia não informado</span></>}</td>
                  <td className="ticker">{row.ticker}</td>
                  <td className="num ov-purchase">{Number(row.entry.pricePerCota) > 0 ? <>{money(row.purchaseCost)}<span className="ov-purchase-quantity">{quantity(row.entry.cotasAdded)} {Number(row.entry.cotasAdded) === 1 ? 'cota' : 'cotas'} × R$ {perCota(row.entry.pricePerCota)}</span></> : <><Def text="Esta compra tem quantidade de cotas, mas o preço foi registrado como zero. Confira o preço no histórico para completar o valor comprado.">Preço a conferir</Def><span className="ov-purchase-quantity">{quantity(row.entry.cotasAdded)} {Number(row.entry.cotasAdded) === 1 ? 'cota' : 'cotas'} · preço registrado R$ 0,00</span></>}</td>
                </tr>)}</tbody>
              </table>
            </div> : <Empty title="Nenhuma compra registrada">As compras aparecem aqui quando você informa cotas adicionadas no histórico.</Empty>}
          </section>
          <section className="ov-recent-income" aria-label="Últimos rendimentos registrados de FIIs">
            <SectionHeading title="Últimos rendimentos" level={3}><TextLink onClick={() => onNavigate('portfolio', 'history')}>Ver todos</TextLink></SectionHeading>
            <p className="ov-recent-income-note">Lançamentos do histórico. Os proventos oficiais, quando disponíveis, também entram no gráfico e nos totais acima.</p>
            {recent.income.length ? <div className="table-wrap">
              <table className="ov-table ov-recent-table">
                <caption className="sr-only">Rendimentos mais recentes registrados no histórico de FIIs</caption>
                <colgroup><col className="ov-col-month" /><col className="ov-col-ticker" /><col /></colgroup>
                <thead><tr><th scope="col">Mês</th><th scope="col">Fundo</th><th scope="col" className="num ov-received"><Def text="Rendimento positivo informado por você ou calculado pelo histórico. Comprar cotas não é necessário para receber: as cotas anteriores continuam no saldo.">Rendimento recebido</Def></th></tr></thead>
                <tbody>{recent.income.map((row) => <tr key={`${row.fundId}-${row.entry.id}`}>
                  <td className="ov-month" title={row.entry.month.slice(0, 7) === currentKey ? `Mês parcial, até ${untilLabel}` : undefined}><span className="num">{monthShort(row.entry.month.slice(0, 7))}</span></td>
                  <td className="ticker">{row.ticker}</td>
                  <td className="num ov-received">{row.computed ? <Def text={`Calculado pelo histórico: ${quantity(row.held)} cotas no mês × R$ ${perCota(row.entry.dividendPerCota)} por cota.`}>{money(row.dividend)}</Def> : money(row.dividend)}</td>
                </tr>)}</tbody>
              </table>
            </div> : <Empty title="Nenhum rendimento registrado">Os rendimentos lançados no histórico aparecem aqui.</Empty>}
          </section>
          </div>
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
              <SectionHeading title="Proventos por mês" level={3}><span className="ov-head-figure"><Def text={`Soma dos rendimentos informados e preenchidos pelas fontes oficiais de ${period}. Sem novas compras, as cotas anteriores continuam. Cada cálculo automático usa o provento real e as cotas com direito na data-com; um valor informado por você tem prioridade. ${monthShort(currentKey)} parcial, até ${untilLabel}, aparece hachurado.`}>em 12 meses</Def><span className="num">{hasIncome ? money(total12) : '—'}</span></span></SectionHeading>
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

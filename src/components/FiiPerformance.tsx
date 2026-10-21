import { useEffect, useId, useRef, useState } from 'react';
import { LoaderCircle, RefreshCw } from 'lucide-react';
import { call, dateBR, desktop, errorMessage, money, percent } from '../lib/bridge';
import type { FiiPerformance as PerformanceData, Snapshot } from '../lib/bridge';
import { Def, Empty, SectionHeading, TextLink } from '../components';
import PublicSourceLink from './PublicSourceLink';
import PortfolioEvolutionChart from './PortfolioEvolutionChart';
import { readPublicCache, usePublicData } from '../lib/publicData';

interface Props { walletId: string; snapshot: Snapshot; onProjection: () => void; onCash: () => void; onFixedIncome?: () => void; onData?: (data: PerformanceData | null) => void }

const number = (value: string | null | undefined) => {
  if (value == null || value.trim() === '') return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
};
const resultClass = (value: string | null | undefined) => {
  const parsed = number(value);
  return parsed === null || parsed === 0 ? '' : parsed > 0 ? 'value-positive' : 'value-negative';
};
const signedMoney = (value: string | null | undefined) => {
  const parsed = number(value);
  return parsed === null ? '—' : `${parsed > 0 ? '+' : parsed < 0 ? '−' : ''}${money(Math.abs(parsed))}`;
};
const signedPercent = (value: string) => {
  const parsed = number(value);
  return parsed === null ? '—' : `${parsed > 0 ? '+' : parsed < 0 ? '−' : ''}${percent(Math.abs(parsed))}`;
};

/** Native commands calculate performance; this view only formats cached results and chart geometry. */
export default function FiiPerformancePanel({ walletId, snapshot, onProjection, onCash, onFixedIncome, onData }: Props) {
  const publicData = usePublicData();
  const { invested, dividendsReceived: dividends } = snapshot.totals;
  const headingId = useId();
  const [data, setData] = useState<PerformanceData | null>(null);
  const [loading, setLoading] = useState(desktop);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const generation = useRef(0);
  const loadedWallet = useRef(walletId);
  const pending = useRef(false);
  useEffect(() => {
    const current = ++generation.current;
    pending.current = false;
    if (loadedWallet.current !== walletId) { setData(null); loadedWallet.current = walletId; }
    setError(''); setNotice(''); setRefreshing(false);
    if (!desktop || !walletId) { setLoading(false); return; }
    setLoading(true);
    void readPublicCache<PerformanceData | null>('get_fii_performance', { walletId })
      .then((cached) => { if (current === generation.current) { setData(cached); onData?.(cached); } })
      .catch((err) => { if (current === generation.current) setError(errorMessage(err)); })
      .finally(() => { if (current === generation.current) setLoading(false); });
    return () => { generation.current += 1; };
  }, [walletId, snapshot, publicData.revision, onData]);
  const refresh = async () => {
    if (pending.current || publicData.loading || publicData.refreshing || !desktop || !walletId) return;
    pending.current = true;
    const current = ++generation.current;
    setRefreshing(true); setError(''); setNotice('');
    try {
      const result = await call<PerformanceData>('refresh_fii_performance', { walletId });
      if (current !== generation.current) return;
      setData(result); onData?.(result); setNotice('Consulta concluída. Confira as datas dos dados.');
      void publicData.reloadCaches();
    } catch (err) {
      if (current === generation.current) setError(`${errorMessage(err)}${data ? ' Os dados já salvos continuam visíveis.' : ''}`);
    } finally {
      if (current === generation.current) { pending.current = false; setRefreshing(false); }
    }
  };
  const benchmark = data?.portfolioBenchmark;
  const totalInvested = data?.invested ?? invested;
  const totalDividends = data?.dividends ?? dividends;
  const income = data?.income;
  const apiIncome = number(income?.apiAmount) ?? 0;
  const portfolio = data?.portfolio;
  const cash = portfolio?.cash;
  const fixedIncome = portfolio?.fixedIncome;
  const hasFixedIncome = Boolean(fixedIncome && fixedIncome.status !== 'empty');
  const issues = portfolio?.issues ?? [];
  const cashBenchmarkPending = Boolean(benchmark?.components.some((component) => component.kind === 'cash' && component.issues.length > 0 && snapshot.cashPockets?.find((pocket) => pocket.id === component.id)?.cashFlows == null));
  const cashPending = Boolean(cash && cash.missingBasisCount > 0);
  return <section className="group fii-performance" aria-labelledby={headingId} aria-busy={loading || refreshing || undefined}>
    <div className="group-head"><h2 id={headingId}>Resultado da carteira</h2><div className="group-actions"><TextLink onClick={onProjection}>Ver projeção</TextLink><button className="button small" type="button" onClick={() => void refresh()} disabled={!desktop || !walletId || loading || refreshing || publicData.loading || publicData.refreshing}>{refreshing ? <LoaderCircle className="spin" size={15} /> : <RefreshCw size={15} />}{refreshing ? 'Atualizando…' : 'Atualizar resultado'}</button></div></div>
    <div className="block">
      <div className="readouts ov-essential performance-readouts">
        <div className="readout ov-purchase"><span className="figure-label"><Def text="Compras de FIIs, aplicações nas caixinhas e valor aplicado no pós-fixado identificado nesta carteira. Nas caixinhas, inclui dinheiro depois resgatado: os resgates não são descontados do total aplicado.">Total aplicado</Def></span><span className="figure-value lg">{portfolio?.invested == null ? '—' : money(portfolio.invested)}</span></div>
        <div className="readout"><span className="figure-label performance-income-label"><Def text={<>Rendimentos dos FIIs mais o ganho das caixinhas (saldo + resgates − aplicações) e do pós-fixado identificado. O ganho do pós-fixado já está no saldo; a rentabilidade dos últimos 12 meses não é somada novamente.{income && <p>FIIs: {money(income.recordedAmount)} do histórico + {money(income.apiAmount)} da B3 = {money(totalDividends)}, até {dateBR(income.asOf)}. A B3 completa os meses sem rendimento informado; um valor seu, inclusive zero, tem prioridade.</p>}</>}>Rendimentos acumulados</Def></span><span className={`figure-value lg ${resultClass(portfolio?.income)}`}>{signedMoney(portfolio?.income)}</span></div>
        <div className="readout"><span className="figure-label"><Def text="Resultado dos FIIs (variação das cotas + rendimentos do histórico e da B3, contados uma vez), mais o ganho das caixinhas e do pós-fixado identificado nesta carteira. Cada saldo mantém sua própria data. Saldo em conta e proventos a receber da corretora não entram neste cálculo.">Resultado da carteira</Def></span><span className={`figure-value lg ${resultClass(portfolio?.totalResult)}`}>{signedMoney(portfolio?.totalResult)}</span>{portfolio?.totalReturnPercent != null && <span className="performance-return"><Def text="Resultado dividido pelo total aplicado, que inclui aplicações depois resgatadas. É um percentual acumulado; não é uma taxa anual nem uma rentabilidade ponderada pelo tempo.">Estimado · {signedPercent(portfolio.totalReturnPercent)} sobre o total aplicado</Def></span>}</div>
      </div>
      <p className="performance-scope">{hasFixedIncome ? 'FIIs do Histórico, caixinhas cadastradas e pós-fixado da corretora' : 'FIIs do Histórico e caixinhas cadastradas'} nesta carteira. Cada saldo mantém a data informada na sua origem.</p>
      {apiIncome > 0 && income && <p className="performance-message performance-api-income">Inclui <strong className="num">{money(income.apiAmount)}</strong> em proventos calculados pela B3 até <time dateTime={income.asOf}>{dateBR(income.asOf)}</time>. Os valores informados por você têm prioridade.{income.status === 'partial' && <span> Há meses sem dados; o resultado usa os rendimentos disponíveis.</span>}</p>}
      {loading && <p className="performance-message" role="status"><LoaderCircle className="spin" size={15} />Lendo dados salvos…</p>}
      {refreshing && <p className="performance-message" role="status">Consultando cotações, proventos e CDI. Os dados salvos continuam visíveis.</p>}
      {error && <p className="performance-message form-error" role="alert">{error}</p>}
      {!desktop && <p className="performance-message">O cálculo e as consultas estão disponíveis no aplicativo Windows.</p>}
      {cash && cash.missingBasisCount > 0 && <p className="performance-message performance-warning performance-cash-pending" role="status">Complete aplicações, resgates e data do saldo em {cash.missingBasisCount} {cash.missingBasisCount === 1 ? 'caixinha' : 'caixinhas'} para fechar os totais. <TextLink onClick={onCash}>Completar caixinhas</TextLink></p>}
      {!cashPending && issues.length > 0 && <p className="performance-message performance-warning" role="status">{issues[0]}</p>}
      {issues.length > 1 && <details className="extra-details performance-issues"><summary>Ver observações do cálculo ({issues.length})</summary><ul>{issues.map((issue, index) => <li key={index}>{issue}</li>)}</ul></details>}
      {portfolio?.status === 'empty' ? <Empty title="Nenhum investimento registrado">Registre compras de FIIs no Histórico, cadastre suas caixinhas ou importe os dados do pós-fixado para acompanhar o resultado.</Empty> : <>
      <div className="performance-portfolio-summary">
        <div className="table-wrap"><table className="performance-portfolio-table"><caption className="sr-only">Composição do resultado da carteira: FIIs, caixinhas e pós-fixado</caption><thead><tr>
          <th scope="col">Investimento</th><th scope="col" className="num ov-purchase"><Def text="Compras dos FIIs, aplicações brutas das caixinhas (sem descontar resgates) e valor aplicado no pós-fixado identificado.">Aplicado</Def></th><th scope="col" className="num"><Def text="Nos FIIs, histórico mais os proventos da B3 que faltavam, sem duplicar o mesmo fundo e mês. Nas caixinhas, saldo + resgates − aplicações. No pós-fixado, ganho acumulado já incluído no saldo informado.">Rendimentos</Def></th><th scope="col" className="num"><Def text="Nos FIIs, soma variação das cotas e rendimentos. Nas caixinhas e no pós-fixado, o próprio ganho acumulado. Cada investimento entra uma vez nos totais.">Resultado</Def></th>
        </tr></thead><tbody>
          <tr><th scope="row">FIIs</th><td className="num ov-purchase">{money(totalInvested)}</td><td className={`num ${resultClass(totalDividends)}`}>{signedMoney(totalDividends)}</td><td className={`num ${resultClass(data?.totalResult)}`}>{signedMoney(data?.totalResult)}</td></tr>
          <tr><th scope="row"><TextLink onClick={onCash}>Caixinhas</TextLink></th><td className="num ov-purchase">{cash?.invested == null ? '—' : money(cash.invested)}</td><td className={`num ${resultClass(cash?.income)}`}>{signedMoney(cash?.income)}</td><td className={`num ${resultClass(cash?.income)}`}>{signedMoney(cash?.income)}</td></tr>
          {hasFixedIncome && fixedIncome && <tr className="performance-fixed-income-row"><th scope="row">{onFixedIncome ? <TextLink onClick={onFixedIncome}>Pós-fixado</TextLink> : 'Pós-fixado'}</th><td className="num ov-purchase">{fixedIncome.invested == null ? '—' : money(fixedIncome.invested)}</td><td className={`num ${resultClass(fixedIncome.income)}`}>{signedMoney(fixedIncome.income)}</td><td className={`num ${resultClass(fixedIncome.result)}`}>{signedMoney(fixedIncome.result)}</td></tr>}
          <tr className="performance-total"><th scope="row">Total</th><td className="num ov-purchase">{portfolio?.invested == null ? '—' : money(portfolio.invested)}</td><td className={`num ${resultClass(portfolio?.income)}`}>{signedMoney(portfolio?.income)}</td><td className={`num ${resultClass(portfolio?.totalResult)}`}>{signedMoney(portfolio?.totalResult)}</td></tr>
        </tbody></table></div>
      </div>
      {hasFixedIncome && fixedIncome && <p className="performance-scope performance-fixed-income-scope">Pós-fixado{fixedIncome.name ? `: ${fixedIncome.name}` : ''}{fixedIncome.referenceDate && <> · saldo de <time dateTime={fixedIncome.referenceDate}>{dateBR(fixedIncome.referenceDate)}</time></>}. {fixedIncome.status === 'available' ? 'O rendimento já está no saldo; o valor dos últimos 12 meses não é somado novamente.' : 'Faltam dados para fechar o rendimento deste investimento.'}</p>}
      <div className="performance-comparison">
        <div className="performance-chart-block">
          <SectionHeading title="Evolução da carteira × 100% do CDI" level={3} />
          <p className="performance-estimate performance-chart-scope">FIIs, pós-fixado e caixinhas. As linhas comparam o ganho acumulado, sem contar os aportes como lucro.</p>
          <PortfolioEvolutionChart data={data?.portfolioEvolution} loading={loading} />
          <p className="performance-estimate"><Def text={<><p>O CDI usa os mesmos aportes e resgates, até a data do registro real de cada mês. Os registros só entram quando todos os investimentos têm valores na mesma data.</p><p>O ponto “Últimos valores” mostra o resultado já conhecido da carteira, com a data de cada investimento. Quando as datas são diferentes, ele fica separado da linha mensal e não informa se a carteira ficou acima ou abaixo do CDI.</p><p>Meses sem saldo da carteira ficam com uma lacuna. O app guarda automaticamente os próximos registros reais por carteira. Valores antigos não são estimados a partir do saldo de hoje.</p>{data?.portfolioEvolution?.assumptions.map((assumption, index) => <p key={index}>{assumption}</p>)}</>}>Como ler a evolução mensal</Def></p>
          {cashBenchmarkPending && <p className="performance-message performance-warning performance-cash-benchmark-pending">A comparação das caixinhas precisa das datas dos depósitos e retiradas. <TextLink onClick={onCash}>Conferir caixinhas</TextLink></p>}
        </div>
        <div className="performance-breakdown">
          <SectionHeading title="Comparação atual por investimento" level={3} />
          <p className="performance-estimate">Últimos valores de cada investimento, com o período e a data do saldo.</p>
          <div className="table-wrap"><table className="performance-table performance-component-table"><caption className="sr-only">Resultado da carteira e CDI por investimento, com as datas de cada saldo</caption><thead><tr><th scope="col">Investimento e período</th><th scope="col" className="num">Resultado</th><th scope="col" className="num">100% do CDI</th></tr></thead><tbody>
            {benchmark?.components.map((component, index) => <tr key={`${component.kind}-${component.id ?? index}`}>
              <th scope="row"><span>{component.name}</span><span className="performance-component-period">{component.startDate ? <><time dateTime={component.startDate}>{dateBR(component.startDate)}</time> → </> : 'Início não informado · '}{component.asOf ? <time dateTime={component.asOf}>{dateBR(component.asOf)}</time> : 'Data do saldo não informada'}</span>{component.issues.length > 0 && <span className="performance-component-note"><Def text={<ul>{component.issues.map((issue, issueIndex) => <li key={issueIndex}>{issue}</li>)}</ul>}>Dados pendentes</Def></span>}</th>
              <td className={`num ${resultClass(component.result)}`}>{signedMoney(component.result)}</td>
              <td className="num">{signedMoney(component.cdi?.result)}</td>
            </tr>)}
            {!benchmark && <tr><td colSpan={3}>Aguardando os dados da comparação da carteira.</td></tr>}
            {benchmark && benchmark.components.length > 0 && <tr className="performance-total"><th scope="row">Carteira inteira</th><td className={`num ${resultClass(benchmark.result)}`}>{signedMoney(benchmark.result)}</td><td className="num">{signedMoney(benchmark.cdi?.result)}</td></tr>}
          </tbody></table></div>
        </div>
      </div>
      {cash && cash.count > 0 && <details className="extra-details performance-cash-dates"><summary>Conferir saldos e datas das caixinhas</summary><p>Cada saldo corresponde à data informada na própria caixinha. <TextLink onClick={onCash}>Editar caixinhas</TextLink></p><div className="table-wrap"><table><caption className="sr-only">Saldos informados por caixinha e suas respectivas datas</caption><thead><tr><th scope="col">Caixinha</th><th scope="col" className="num">Saldo informado</th><th scope="col" className="num">Data do saldo</th></tr></thead><tbody>{cash.items.map((item) => <tr key={item.id}><th scope="row">{item.name}</th><td className="num">{money(item.balance)}</td><td className="num">{item.balanceDate ? <time dateTime={item.balanceDate}>{dateBR(item.balanceDate)}</time> : 'Não informada'}</td></tr>)}</tbody></table></div></details>}
      </>}
      <div className="performance-source">
        {data?.quoteSource && <span>{data.quoteSource}{data.quoteFetchedAt && <> · coletado em <time dateTime={data.quoteFetchedAt}>{dateBR(data.quoteFetchedAt)}</time></>}</span>}
        {data?.cdiSource && <span>{data.cdiSourceUrl ? <PublicSourceLink href={data.cdiSourceUrl}>{data.cdiSource}</PublicSourceLink> : data.cdiSource}{data.cdiFetchedAt && <> · coletado em <time dateTime={data.cdiFetchedAt}>{dateBR(data.cdiFetchedAt)}</time></>}</span>}
        {data?.dividendSource && <span><Def text={<><p>Proventos publicados pela B3 e calculados pelas cotas com direito na data-com. Os cálculos completam os rendimentos disponíveis e conservam os lançamentos originais.</p>{data.dividendSources?.map((source) => <p key={source.symbol}><PublicSourceLink href={source.sourceUrl}>{source.symbol}</PublicSourceLink> · consultado em {dateBR(source.fetchedAt)}</p>)}</>}>Proventos B3</Def>{data.dividendFetchedAt && <> · coletados em <time dateTime={data.dividendFetchedAt}>{dateBR(data.dividendFetchedAt)}</time></>}</span>}
        {data && <span><Def text="Compara os mesmos aportes e resgates dos FIIs, pós-fixado e caixinhas com uma hipótese de rendimento a 100% do CDI, até a data de cada investimento. A comparação é bruta, sem impostos; os proventos recebidos dos FIIs não são reinvestidos automaticamente.">CDI bruto · proventos dos FIIs não reinvestidos</Def></span>}
      </div>
      {notice && !refreshing && <p className="performance-message" role="status">{notice}</p>}
    </div>
  </section>;
}

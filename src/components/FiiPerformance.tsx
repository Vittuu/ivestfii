import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { LoaderCircle, RefreshCw } from 'lucide-react';
import { call, dateBR, desktop, errorMessage, money, monthShort, percent } from '../lib/bridge';
import type { FiiPerformance as PerformanceData, Snapshot } from '../lib/bridge';
import { Def, Empty, SectionHeading, TextLink } from '../components';
import PublicSourceLink from './PublicSourceLink';
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
  const hasResult = number(data?.totalResult) !== null;
  const comparable = hasResult && number(data?.cdi?.result) !== null;
  const totalInvested = data?.invested ?? invested;
  const totalDividends = data?.dividends ?? dividends;
  const income = data?.income;
  const apiIncome = number(income?.apiAmount) ?? 0;
  const hasDatedPurchases = snapshot.funds.some((fund) => fund.entries.some((entry) => entry.purchaseDate));
  const recordedDividends = income?.recordedAmount ?? totalDividends;
  const portfolio = data?.portfolio;
  const cash = portfolio?.cash;
  const fixedIncome = portfolio?.fixedIncome;
  const hasFixedIncome = Boolean(fixedIncome && fixedIncome.status !== 'empty');
  const issues = portfolio?.issues ?? [];
  const fiiIssues = data?.issues ?? [];
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
          <SectionHeading title="FIIs × 100% do CDI" level={3} />
          {fiiIssues.length > 0 && <p className="performance-message performance-warning" role="status">{fiiIssues[0]}</p>}
          {fiiIssues.length > 1 && <details className="extra-details performance-issues"><summary>Ver observações dos FIIs ({fiiIssues.length})</summary><ul>{fiiIssues.map((issue, index) => <li key={index}>{issue}</li>)}</ul></details>}
          <p className="performance-estimate performance-chart-scope">Apenas FIIs. Caixinhas e pós-fixado entram nos totais acima, sem participar deste gráfico.</p>
          <p className="performance-period">{data?.startMonth && data.asOf ? <>De {monthShort(data.startMonth)} até <time dateTime={data.asOf}>{dateBR(data.asOf)}</time></> : 'Período da comparação ainda indisponível.'}</p>
          <p className="performance-estimate">{hasDatedPurchases ? 'Comparação estimada · compras nas datas informadas; sem dia, usa o fim do mês ou a data da cotação.' : 'Comparação estimada · compras no fim do mês; no mês da cotação, até a cotação.'}</p>
          {comparable && data?.totalResult != null && data.cdi ? <ResultBars fii={data.totalResult} cdi={data.cdi.result} fiiPercent={data.totalReturnPercent} cdiPercent={data.cdi.returnPercent} /> : <p className="performance-unavailable">{data?.status === 'empty' ? 'Sem compras de FIIs registradas para comparar com o CDI.' : hasResult ? 'O resultado dos FIIs está disponível. Falta o CDI do mesmo período para comparar.' : 'A comparação precisa do valor completo dos FIIs e do CDI no mesmo período. Atualize os dados para consultar.'}</p>}
          {data?.difference != null && comparable && <p className="performance-difference">Diferença para o CDI: <strong className={`num ${resultClass(data.difference)}`}>{signedMoney(data.difference)}</strong></p>}
        </div>
        <div className="performance-breakdown">
          <SectionHeading title="Como o resultado dos FIIs é formado" level={3} />
          <div className="table-wrap"><table className="performance-table"><caption className="sr-only">Composição do resultado total dos FIIs</caption><tbody>
            <tr><th scope="row"><Def text="Valor das cotas atuais pelas cotações usadas no cálculo. Não altera a posição importada da corretora.">Valor de mercado dos FIIs</Def></th><td className="num">{data?.marketValue == null ? '—' : money(data.marketValue)}</td></tr>
            <tr><th scope="row" className="ov-purchase">− Total investido</th><td className="num ov-purchase">{money(totalInvested)}</td></tr>
            <tr><th scope="row"><Def text="Valor de mercado das cotas menos o valor das compras registradas. Pode ser ganho ou perda.">= Variação das cotas</Def></th><td className={`num ${resultClass(data?.priceResult)}`}>{signedMoney(data?.priceResult)}</td></tr>
            <tr><th scope="row" className="ov-received"><Def text="Totais informados e valores calculados pelas cotas × rendimento por cota, conforme o Histórico; valores calculados são estimativas. Estes valores têm prioridade sobre a B3.">+ Rendimentos registrados</Def></th><td className="num ov-received">{money(recordedDividends)}</td></tr>
            {apiIncome > 0 && <tr className="performance-api-row"><th scope="row" className="ov-received"><Def text="Proventos por cota publicados na B3 multiplicados pelas cotas com direito ao pagamento. Só preenche fundos e meses sem rendimento informado. Os pagamentos são limitados à mesma data das cotações usadas na comparação.">+ Proventos B3</Def></th><td className="num ov-received">{money(income!.apiAmount)}</td></tr>}
            <tr className="performance-total"><th scope="row">= Resultado total</th><td className={`num ${resultClass(data?.totalResult)}`}>{signedMoney(data?.totalResult)}</td></tr>
          </tbody></table></div>
        </div>
      </div>
      {cash && cash.count > 0 && <details className="extra-details performance-cash-dates"><summary>Conferir saldos e datas das caixinhas</summary><p>Cada saldo corresponde à data informada na própria caixinha. <TextLink onClick={onCash}>Editar caixinhas</TextLink></p><div className="table-wrap"><table><caption className="sr-only">Saldos informados por caixinha e suas respectivas datas</caption><thead><tr><th scope="col">Caixinha</th><th scope="col" className="num">Saldo informado</th><th scope="col" className="num">Data do saldo</th></tr></thead><tbody>{cash.items.map((item) => <tr key={item.id}><th scope="row">{item.name}</th><td className="num">{money(item.balance)}</td><td className="num">{item.balanceDate ? <time dateTime={item.balanceDate}>{dateBR(item.balanceDate)}</time> : 'Não informada'}</td></tr>)}</tbody></table></div></details>}
      </>}
      <div className="performance-source">
        {data?.quoteSource && <span>{data.quoteSource}{data.quoteFetchedAt && <> · coletado em <time dateTime={data.quoteFetchedAt}>{dateBR(data.quoteFetchedAt)}</time></>}</span>}
        {data?.cdiSource && <span>{data.cdiSourceUrl ? <PublicSourceLink href={data.cdiSourceUrl}>{data.cdiSource}</PublicSourceLink> : data.cdiSource}{data.cdiFetchedAt && <> · coletado em <time dateTime={data.cdiFetchedAt}>{dateBR(data.cdiFetchedAt)}</time></>}</span>}
        {data?.dividendSource && <span><Def text={<><p>Proventos publicados pela B3 e calculados pelas cotas com direito na data-com. Os cálculos completam os rendimentos disponíveis e conservam os lançamentos originais.</p>{data.dividendSources?.map((source) => <p key={source.symbol}><PublicSourceLink href={source.sourceUrl}>{source.symbol}</PublicSourceLink> · consultado em {dateBR(source.fetchedAt)}</p>)}</>}>Proventos B3</Def>{data.dividendFetchedAt && <> · coletados em <time dateTime={data.dividendFetchedAt}>{dateBR(data.dividendFetchedAt)}</time></>}</span>}
        {data && <span><Def text="Compara as mesmas compras com uma hipótese de rendimento a 100% do CDI. A comparação é bruta, sem impostos, e os rendimentos recebidos dos FIIs não são reinvestidos automaticamente.">CDI bruto · rendimentos não reinvestidos</Def></span>}
      </div>
      {notice && !refreshing && <p className="performance-message" role="status">{notice}</p>}
    </div>
  </section>;
}

function ResultBars({ fii, cdi, fiiPercent, cdiPercent }: { fii: string; cdi: string; fiiPercent: string | null; cdiPercent: string }) {
  const chartId = useId();
  const [active, setActive] = useState(0);
  const values = [number(fii)!, number(cdi)!];
  const minimum = Math.min(0, ...values);
  const maximum = Math.max(0, ...values);
  const range = maximum - minimum || 1;
  const y = (value: number) => 40 + (maximum - value) / range * 166;
  const zero = y(0);
  const names = ['FIIs', '100% do CDI'];
  const percentages = [fiiPercent, cdiPercent];
  const select = (event: KeyboardEvent<SVGSVGElement>) => {
    const index = event.key === 'ArrowRight' || event.key === 'End' ? 1 : event.key === 'ArrowLeft' || event.key === 'Home' ? 0 : null;
    if (index === null) return;
    event.preventDefault(); setActive(index);
  };
  return <div className="performance-bars">
    <svg width="100%" height="258" role="img" tabIndex={0} onKeyDown={select} aria-labelledby={`${chartId}-title ${chartId}-desc`}>
      <title id={`${chartId}-title`}>Ganhos acumulados em reais: FIIs e 100% do CDI</title>
      <desc id={`${chartId}-desc`}>FIIs: {signedMoney(fii)}. CDI: {signedMoney(cdi)}. A linha horizontal marca zero. Use as setas para consultar cada barra.</desc>
      <line className="performance-zero" x1="6%" x2="94%" y1={zero} y2={zero} />
      {values.map((value, index) => {
        const x = index ? '59%' : '19%';
        const center = index ? '70%' : '30%';
        const top = Math.min(zero, y(value));
        const height = Math.abs(y(value) - zero);
        return <g key={names[index]} onPointerEnter={() => setActive(index)} className={active === index ? 'is-active' : undefined}>
          <rect className="performance-hit" x={index ? '55%' : '15%'} y="28" width="30%" height="214" />
          <rect className={index ? 'performance-bar cdi' : `performance-bar fii ${value < 0 ? 'negative' : ''}`} x={x} y={top} width="22%" height={height} />
          <text className="performance-bar-value" x={center} y={value < 0 ? y(value) + 20 : y(value) - 10} textAnchor="middle">{signedMoney(index ? cdi : fii)}</text>
          <text className="performance-bar-label" x={center} y="248" textAnchor="middle">{names[index]}</text>
        </g>;
      })}
    </svg>
    <p className="performance-point" role="status">{names[active]}: <strong>{signedMoney(active ? cdi : fii)}</strong>{percentages[active] != null && <> · {signedPercent(percentages[active]!)} sobre o valor comprado</>}</p>
    <details className="extra-details performance-chart-table"><summary>Ver valores da comparação</summary><div className="table-wrap"><table><caption className="sr-only">Resultados acumulados comparados com as mesmas compras</caption><thead><tr><th scope="col">Investimento</th><th scope="col" className="num">Resultado</th><th scope="col" className="num">Sobre compras</th></tr></thead><tbody>{names.map((name, index) => <tr key={name}><th scope="row">{name}</th><td className="num">{signedMoney(index ? cdi : fii)}</td><td className="num">{percentages[index] == null ? '—' : signedPercent(percentages[index]!)}</td></tr>)}</tbody></table></div></details>
  </div>;
}

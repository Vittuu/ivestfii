import { useId } from 'react';
import { amount, dateBR, money, monthShort } from '../lib/bridge';
import type { EconomicIndicator, PublicDataSourceId, Snapshot } from '../lib/bridge';
import { usePublicData } from '../lib/publicData';
import { Def, SectionHeading, TextLink } from '../components';
import PublicSourceLink from './PublicSourceLink';

const sourceNames: Record<PublicDataSourceId, string> = { economics: 'Indicadores', news: 'Notícias', quotes: 'Cotações', cvm: 'CVM', cdiHistory: 'CDI da comparação', dividends: 'Proventos B3', historicalValuations: 'Histórico da carteira' };
const statusNames = { updated: 'Consulta concluída', cached: 'Dados salvos', unavailable: 'Indisponível agora', not_configured: 'Falta configuração', deferred: 'Consulta adiada', unsupported: 'Sem cobertura para este fundo' };
const indicators = [
  { id: 'cdi', label: 'CDI anual', unit: '% a.a.', help: 'CDI anualizado na data de referência, em uma base de 252 dias úteis. Não é uma promessa de rendimento da sua carteira.' },
  { id: 'selic', label: 'Meta Selic', unit: '% a.a.', help: 'Meta da taxa básica de juros definida pelo Copom, na data informada.' },
  { id: 'ipca', label: 'IPCA no mês', unit: '% no mês', help: 'Variação mensal de preços do período publicado. Não é uma taxa anual.' },
] as const;
const valid = (value: string | null | undefined) => value != null && value.trim() !== '' && Number.isFinite(Number(value));
function quotePrice(value: string, currency: string) {
  if (!valid(value)) return '—';
  try { return new Intl.NumberFormat('pt-BR', { style: 'currency', currency }).format(Number(value)); }
  catch { return `${amount(value)} ${currency}`; }
}
function marketDate(value: string | null) {
  if (!value) return 'Não informada';
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString('pt-BR', { dateStyle: 'short', timeStyle: 'short' });
}
function IndicatorOrigin({ indicator }: { indicator: EconomicIndicator }) {
  return <><small>Ref. <time dateTime={indicator.referenceDate}>{indicator.id === 'ipca' ? monthShort(indicator.referenceDate) : dateBR(indicator.referenceDate)}</time> · <PublicSourceLink href={indicator.sourceUrl}>Banco Central</PublicSourceLink></small><small>Coletado em <time dateTime={indicator.fetchedAt}>{dateBR(indicator.fetchedAt)}</time></small></>;
}

/** Short, dated source data; no values here alter imported balances or recorded purchases. */
export default function MarketSummary({ snapshot, onMarket }: { snapshot: Snapshot; onMarket: () => void }) {
  const heading = useId();
  const data = usePublicData();
  const symbols = [...new Set([...snapshot.funds.map((fund) => fund.ticker), ...(snapshot.brokerPosition?.listedFunds.holdings.map((holding) => holding.ticker) ?? [])])].sort();
  const rows = symbols.slice(0, 4);
  const news = data.news?.items.slice(0, 3) ?? [];
  const unconfigured = data.sources.some((source) => source.id === 'quotes' && source.status === 'not_configured');
  const unavailable = data.sources.some((source) => source.status === 'unavailable' || source.status === 'cached' && Boolean(source.message));
  const deferred = data.sources.some((source) => source.status === 'deferred');
  return <section className="group public-data-summary" aria-labelledby={heading}>
    <div className="group-head"><h2 id={heading}>Mercado em resumo</h2><div className="group-actions"><TextLink onClick={onMarket}>Abrir Mercado</TextLink></div></div>
    <div className="block">
      <div className="public-data-state" role="status" aria-busy={data.loading || data.refreshing || undefined}>
        {data.loading ? 'Lendo dados salvos…' : data.refreshing ? 'Consultando fontes em segundo plano. Os dados salvos continuam visíveis.' : data.checkedAt ? <>Verificação automática em <time dateTime={data.checkedAt}>{marketDate(data.checkedAt)}</time>. Confira a referência de cada dado.</> : 'Dados salvos neste computador. Cada fonte tem sua data de referência.'}
      </div>
      {data.error && <p className="public-data-guidance" role="status">{data.error}</p>}
      {unconfigured && <p className="public-data-guidance">Cotações sem chave configurada. Cadastre a chave da brapi em Configurações → Dados de mercado.</p>}
      {unavailable && <p className="public-data-guidance">Algumas fontes não puderam ser atualizadas agora. Os dados salvos continuam disponíveis com suas datas.</p>}
      {deferred && !data.refreshing && <p className="public-data-guidance">Algumas consultas foram adiadas para respeitar os limites das fontes.</p>}
      <dl className="public-indicators">{indicators.map((definition) => {
        const item = data.economics?.indicators.find((indicator) => indicator.id === definition.id);
        const issue = data.economics?.errors.find((error) => error.id === definition.id);
        return <div key={definition.id} data-indicator={definition.id}>
          <dt><Def text={definition.help}>{definition.label}</Def></dt>
          <dd className="public-indicator-value num">{item && valid(item.value) ? <>{amount(item.value)} <span>{definition.unit}</span></> : '—'}</dd>
          <dd className="public-indicator-origin">{item ? <IndicatorOrigin indicator={item} /> : <small>Sem referência salva</small>}{issue && <small className="public-data-warning">Referência indisponível agora.</small>}</dd>
        </div>;
      })}</dl>
      <div className="public-summary-grid">
        <div className="public-funds">
          <SectionHeading level={3} title="Fundos da carteira" meta="Dados externos" />
          {rows.length ? <div className="table-wrap" role="region" aria-label="Cotações e informes resumidos, role horizontalmente para ver todas as colunas" tabIndex={0}><table className="public-funds-table">
            <caption className="sr-only">Cotações e valor patrimonial dos fundos</caption>
            <thead><tr><th scope="col">Fundo</th><th scope="col" className="num"><Def text="Preço externo informado pela brapi, com possível atraso. Não atualiza a posição importada da corretora.">Cotação</Def></th><th scope="col">Data da cotação</th><th scope="col" className="num"><Def text="Parte do patrimônio do fundo correspondente a uma cota, no mês informado pela CVM. Pode ser diferente do preço na bolsa.">VP / cota</Def></th><th scope="col">Mês do informe</th></tr></thead>
            <tbody>{rows.map((symbol) => {
              const quote = data.quotes.items.find((item) => item.symbol === symbol);
              const cvm = data.cvm.items.find((item) => item.symbol === symbol);
              const observation = quote?.observation;
              const report = cvm?.report;
              return <tr key={symbol}><th scope="row" className="ticker">{symbol}</th>
                <td className="num">{observation ? quotePrice(observation.price, observation.currency) : '—'}</td>
                <td>{observation?.quotedAt ? <time dateTime={observation.quotedAt}>{marketDate(observation.quotedAt)}</time> : 'Não informada'}</td>
                <td className="num">{report && valid(report.equityPerShare) ? money(report.equityPerShare!) : '—'}</td>
                <td>{report ? <time dateTime={report.referenceDate}>{monthShort(report.referenceDate)}</time> : '—'}</td>
              </tr>;
            })}</tbody>
          </table></div> : <p className="public-summary-empty">Sem dados dos fundos nesta carteira. Cadastre fundos ou confira as fontes no Mercado.</p>}
          <p className="public-summary-note">{symbols.length > rows.length && <>{rows.length} de {symbols.length} fundos. </>}Cotações e informes não são dados ao vivo. Fontes e avisos por fundo ficam no Mercado.</p>
        </div>
        <div className="public-news"><SectionHeading level={3} title="Notícias de economia" />
          {news.length ? <ul>{news.map((item) => <li key={item.id}><PublicSourceLink href={item.url}>{item.title}</PublicSourceLink><span>{item.source} · {item.publishedAt ? <time dateTime={item.publishedAt}>{dateBR(item.publishedAt)}</time> : 'Sem data de publicação'}</span></li>)}</ul> : <p className="public-summary-empty">Nenhuma notícia salva. A consulta automática tentará buscar as manchetes.</p>}
          {data.news && <p className="public-summary-note">Coletadas em <time dateTime={data.news.fetchedAt}>{dateBR(data.news.fetchedAt)}</time> · <PublicSourceLink href={data.news.sourceUrl}>{data.news.source}</PublicSourceLink></p>}
        </div>
      </div>
      {data.sources.length > 0 && <details className="extra-details public-source-checks"><summary>Situação das fontes</summary><ul>{data.sources.map((source) => <li key={source.id} data-source={source.id} data-source-status={source.status}><strong>{sourceNames[source.id]}</strong><span>{statusNames[source.status]}{source.message && <> · {source.message}</>}</span></li>)}</ul></details>}
    </div>
  </section>;
}

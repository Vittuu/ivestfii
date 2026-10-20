import { useEffect, useRef, useState } from 'react';
import { LoaderCircle, RefreshCw } from 'lucide-react';
import { amount, call, dateBR, desktop, errorMessage, money, monthShort, quantity } from '../lib/bridge';
import type { CvmFundItem, EconomicIndicator, MarketDataSettings, MarketDataStatus, QuoteItem } from '../lib/bridge';
import { Def, Empty, Field, Loading, PageActions } from '../components';
import PublicSourceLink from '../components/PublicSourceLink';
import { emptyEconomics as emptyPanel, usePublicData } from '../lib/publicData';

const definitions = [
  { id: 'cdi', label: 'CDI', unit: '% a.a.', note: 'Base de 252 dias úteis · usado nas caixinhas', description: 'Referência para simular suas caixinhas. A taxa anualizada corresponde à data informada.' },
  { id: 'selic', label: 'Meta Selic', unit: '% a.a.', note: 'Definida pelo Copom', description: 'Meta para a taxa básica de juros definida pelo Copom.' },
  { id: 'ipca', label: 'IPCA', unit: '% no mês', note: 'Variação mensal de preços', description: 'Variação mensal do índice de preços. A data identifica o período publicado na série.' },
] as const;
/** Espaço inseparável antes de cada "·", para a linha nunca quebrar começando pelo separador. */
const SEP = '\u00a0· ';
/** "consultado em" preso ao horário que vem depois. */
const FETCHED = 'consultado\u00a0em\u00a0';

export default function Market({ walletId }: { walletId: string }) {
  const publicData = usePublicData();
  const panel = publicData.economics ?? emptyPanel;
  const loading = publicData.loading;
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const request = useRef(0);
  const news = useNews();

  useEffect(() => () => { request.current += 1; }, []);

  const refresh = async () => {
    const current = ++request.current;
    setRefreshing(true); setError(''); setNotice('');
    try {
      const result = await publicData.refreshEconomics();
      if (current !== request.current) return;
      setNotice(result.errors.length ? 'Atualização parcial. Confira os avisos e a data de referência de cada indicador.' : 'Indicadores atualizados. Confira a data de referência de cada um.');
    } catch (err) {
      if (current === request.current) setError(`${errorMessage(err)} As referências já salvas continuam disponíveis abaixo.`);
    } finally { if (current === request.current) setRefreshing(false); }
  };

  const emptyIndicators = <Empty title="Nenhum indicador salvo">{desktop ? 'Atualize os indicadores para buscar CDI, meta Selic e IPCA no Banco Central. Nas caixinhas, o CDI também pode ser informado manualmente.' : 'A consulta e o cache de indicadores estão disponíveis no aplicativo Windows.'}</Empty>;

  return <div className="page market-page">
    <PageActions>
      <button className="button primary" disabled={!desktop || loading || refreshing || publicData.refreshing} onClick={() => void refresh()}>{refreshing ? <LoaderCircle size={16} className="spin" /> : <RefreshCw size={16} />}{refreshing ? 'Atualizando indicadores…' : 'Atualizar indicadores'}</button>
    </PageActions>

    {publicData.refreshing && <p className="market-status" role="status">Consultando fontes em segundo plano. Os dados salvos continuam disponíveis.</p>}
    {publicData.error && <p className="market-status" role="status">{publicData.error}</p>}
    <WalletMarket key={walletId} walletId={walletId} />

    <section className="group" aria-labelledby="market-indicators-title">
      <div className="group-head">
        <h2 id="market-indicators-title"><Def text="Referências do Banco Central. Cada série segue seu calendário. O aplicativo consulta automaticamente quando necessário; você também pode pedir uma atualização.">Indicadores</Def></h2>
        <p>Banco Central</p>
        {notice && <p className="market-status" role="status">{notice}</p>}
      </div>
      {error && <div className="alert error" role="alert">{error}</div>}
      {loading ? <div className="block"><Loading /></div> : !panel.indicators.length && !panel.errors.length ? <div className="block">{emptyIndicators}</div> : <div className="modular market-panel">
        {definitions.map((definition, index) => {
          const indicator = panel.indicators.find((item) => item.id === definition.id);
          const issue = panel.errors.find((item) => item.id === definition.id);
          const labelId = `market-${definition.id}-label`;
          const descriptionId = `market-${definition.id}-desc`;
          const lead = index === 0;
          return <div className={`block market-indicator${lead ? ' is-lead' : ''}`} role="group" aria-labelledby={labelId} aria-describedby={descriptionId} key={definition.id}>
            <div className="block-body">
              <span className="figure-label" id={labelId}><Def text={`${definition.description} ${definition.note}.`}>{definition.label}</Def></span>
              <span className="sr-only" id={descriptionId}>{definition.description}</span>
              <strong className={`${lead ? 'figure-hero' : 'figure-value lg'}${indicator ? '' : ' is-empty'}`}>{indicator ? formatRate(indicator.value) : '—'}{indicator && rateNumber(indicator.value) !== null && <span className="unit">{definition.unit}</span>}</strong>
              {issue && <p className="market-issue" role="alert">{issue.message}{indicator ? ' Exibindo a referência salva.' : ''}</p>}
              <p className="market-src">{indicator ? <>Ref. <time dateTime={indicator.referenceDate}>{referenceLabel(indicator)}</time></> : 'Sem referência salva'}</p>
            </div>
          </div>;
        })}
        {!panel.indicators.length && <div className="block market-panel-empty">{emptyIndicators}</div>}
      </div>}
      {panel.indicators.length > 0 && <details className="extra-details market-data-details"><summary>Detalhes dos dados</summary><div className="market-source-list">{panel.indicators.map((indicator) => <div key={indicator.id}><strong>{definitions.find((item) => item.id === indicator.id)?.label || indicator.name}</strong><IndicatorSource indicator={indicator} showFetch /></div>)}</div></details>}
    </section>

    <News state={news} />
  </div>;
}

const statusLabels: Record<MarketDataStatus, string> = {
  mapping_required: 'Vincular CNPJ', invalid_mapping: 'CNPJ inválido', not_found: 'Informe não encontrado', unsupported: 'Tipo não atendido', cached: 'Salvo neste computador', available: 'Consulta concluída', unavailable: 'Indisponível',
  not_configured: 'Chave não configurada', unauthorized: 'Chave não autorizada', forbidden: 'Acesso não incluído', rate_limited: 'Limite de consultas atingido',
};

/** Views share native caches; only explicit actions call the manual source commands. */
function useWalletPanel<K extends 'quotes' | 'cvm'>(walletId: string, source: K) {
  const publicData = usePublicData();
  const panel = publicData[source];
  const loading = publicData.loading;
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const request = useRef(0);
  const pending = useRef(false);
  useEffect(() => () => { request.current += 1; }, [walletId]);
  const refresh = async () => {
    if (pending.current || publicData.refreshing) return;
    const current = ++request.current;
    pending.current = true; setRefreshing(true); setError(''); setNotice('');
    try {
      const result = await (source === 'quotes' ? publicData.refreshQuotes() : publicData.refreshCvm());
      if (current !== request.current) return;
      setNotice(result.errors.length ? 'Consulta parcial. Confira os avisos de cada fundo.' : 'Consulta concluída. Confira as datas de cada fundo.');
    } catch (err) { if (current === request.current) setError(`${errorMessage(err)} Os dados já salvos continuam disponíveis.`); }
    finally { if (current === request.current) { pending.current = false; setRefreshing(false); } }
  };
  return { panel, loading, refreshing: refreshing || publicData.refreshing, error, notice, refresh };
}

function WalletMarket({ walletId }: { walletId: string }) {
  const publicData = usePublicData();
  const quotes = useWalletPanel(walletId, 'quotes');
  const cvm = useWalletPanel(walletId, 'cvm');
  const [mapping, setMapping] = useState<{ symbol: string; cnpj: string } | null>(null);
  const [saving, setSaving] = useState(false);
  const [mappingError, setMappingError] = useState('');
  const [mappingNotice, setMappingNotice] = useState('');
  const [overrides, setOverrides] = useState<Record<string, string | null>>({});
  const request = useRef(0);
  const savePending = useRef(false);
  const mappingButtons = useRef(new Map<string, HTMLButtonElement>());
  useEffect(() => () => { request.current += 1; }, []);
  const closeMapping = (symbol: string) => {
    setMapping(null);
    requestAnimationFrame(() => mappingButtons.current.get(symbol)?.focus({ preventScroll: true }));
  };
  const saveMapping = async (remove = false) => {
    if (!mapping || savePending.current || cvm.refreshing) return;
    if (!remove && mapping.cnpj.replace(/\D/g, '').length !== 14) { setMappingError('Informe um CNPJ de fundo com 14 dígitos.'); return; }
    const current = ++request.current;
    const { symbol, cnpj } = mapping;
    savePending.current = true; setSaving(true); setMappingError(''); setMappingNotice('');
    try {
      const result = await call<MarketDataSettings>('save_cvm_mapping', { symbol, cnpj: remove ? null : cnpj });
      if (current !== request.current) return;
      const effectiveCnpj = result.cvmMappings[symbol] || null;
      setOverrides((old) => ({ ...old, [symbol]: effectiveCnpj }));
      publicData.configurationChanged(result);
      closeMapping(symbol);
      setMappingNotice(remove ? effectiveCnpj ? `CNPJ em uso para ${symbol}: ${cnpjLabel(effectiveCnpj)}. A consulta automática verificará o informe desse fundo.` : `Vínculo de ${symbol} removido. Os dados do relatório anterior não serão exibidos.` : `CNPJ de ${symbol} salvo. A consulta automática verificará o informe desse fundo.`);
    } catch (err) { if (current === request.current) setMappingError(errorMessage(err)); }
    finally { if (current === request.current) { savePending.current = false; setSaving(false); } }
  };
  const mappedItem = (item: CvmFundItem): CvmFundItem => {
    if (!Object.prototype.hasOwnProperty.call(overrides, item.symbol)) return item;
    const cnpj = overrides[item.symbol];
    if ((cnpj || '').replace(/\D/g, '') === (item.cnpj || '').replace(/\D/g, '')) return item;
    return { ...item, cnpj, report: null, status: cnpj ? 'unavailable' : 'mapping_required', error: cnpj ? 'Vínculo alterado. Atualize a CVM para consultar esse CNPJ.' : null };
  };
  return <>
    <section className="group" aria-labelledby="market-quotes-title">
      <div className="group-head">
        <h2 id="market-quotes-title">Cotações dos fundos</h2>
        <div className="group-actions"><RefreshButton name="Atualizar cotações" busy={quotes.refreshing} disabled={quotes.loading || !quotes.panel.credentialConfigured} onClick={quotes.refresh} /></div>
      </div>
      <div className="block market-data-block">
        {quotes.loading ? <Loading /> : <>
          {!quotes.panel.credentialConfigured && <p className="market-data-guidance">Cadastre uma chave gratuita da brapi em Configurações → Dados de mercado. Os valores salvos, quando disponíveis, continuam visíveis.</p>}
          <PanelMessages error={quotes.error} notice={quotes.notice} errors={quotes.panel.errors} />
          {quotes.panel.items.length ? <div className="table-wrap" role="region" aria-label="Tabela de cotações, role horizontalmente para ver todas as colunas" tabIndex={0}><table className="market-quotes-table">
            <caption className="sr-only">Cotações externas dos fundos da carteira selecionada</caption>
            <thead><tr><th scope="col">Ativo</th><th scope="col" className="num"><Def text="Preço externo da brapi, separado da posição importada. Não altera o patrimônio da carteira e pode ter atraso conforme o plano.">Cotação</Def></th><th scope="col"><Def text="Data e horário de mercado informados para esta cotação. A data da coleta fica nos detalhes e não substitui este horário.">Data de mercado</Def></th><th scope="col">Situação</th></tr></thead>
            <tbody>{quotes.panel.items.map((item) => <QuoteRow item={item} key={item.symbol} />)}</tbody>
          </table></div> : <Empty title="Nenhum fundo para consultar">Os fundos registrados na carteira selecionada aparecerão aqui.</Empty>}
          {quotes.panel.items.length > 0 && <details className="extra-details market-data-details"><summary>Detalhes dos dados</summary>
            <div className="table-wrap" role="region" aria-label="Fontes e coleta das cotações" tabIndex={0}><table className="market-quotes-details-table"><caption className="sr-only">Fontes, coleta e cadência das cotações externas</caption><thead><tr><th scope="col">Ativo</th><th scope="col">Fonte e coleta</th><th scope="col">Cadência informada</th></tr></thead><tbody>{quotes.panel.items.map((item) => <tr key={item.symbol}><th scope="row" className="ticker">{item.symbol}</th><td>{item.observation ? <SourceCollected source={item.observation.source} url={item.observation.sourceUrl} fetchedAt={item.observation.fetchedAt} /> : '—'}</td><td>{item.observation?.delayMinutes == null ? 'Não informada' : <>Cadência do gratuito: {item.observation.delayMinutes} min</>}</td></tr>)}</tbody></table></div>
            <p className="market-detail-note">Cotações externas não alteram a posição importada, os lançamentos nem o patrimônio registrado. O plano gratuito pode ter atraso e limites por ativo.</p>
          </details>}
        </>}
      </div>
    </section>

    <section className="group" aria-labelledby="market-cvm-title">
      <div className="group-head">
        <h2 id="market-cvm-title">Informes mensais da CVM</h2>
        <div className="group-actions"><RefreshButton name="Atualizar CVM" busy={cvm.refreshing} disabled={cvm.loading || saving} onClick={cvm.refresh} /></div>
      </div>
      <div className="block market-data-block">
        {cvm.loading ? <Loading /> : <>
          <PanelMessages error={cvm.error} notice={mappingNotice || cvm.notice} errors={cvm.panel.errors} />
          {cvm.panel.items.length ? <div className="table-wrap" role="region" aria-label="Tabela de informes CVM, role horizontalmente para ver todas as colunas" tabIndex={0}><table className="market-cvm-table">
            <caption className="sr-only">Informes mensais dos fundos da carteira selecionada</caption>
            <thead><tr><th scope="col">Fundo</th><th scope="col">Tipo</th><th scope="col" className="num"><Def text="Valor patrimonial por cota declarado à CVM. É mensal, contábil e não é preço de negociação. Dados ausentes ou ambíguos permanecem sem valor.">VP / cota</Def></th><th scope="col"><Def text="Mês de referência do informe. A entrega à CVM e a coleta pelo aplicativo têm datas próprias nos detalhes.">Competência</Def></th></tr></thead>
            <tbody>{cvm.panel.items.map((stored) => {
              const item = mappedItem(stored); const report = item.report;
              return <tr key={item.symbol}>
                <th scope="row"><span className="ticker">{item.symbol}</span>{item.status !== 'available' && <span className="market-cell-note">{statusLabels[item.status] || item.status}</span>}{item.error && <span className="market-cell-error">{item.error}</span>}</th>
                <td>{report ? report.fundType ?? 'FII' : '—'}</td>
                <td className="num right">{report ? nullableMoney(report.equityPerShare) : '—'}</td>
                <td>{report ? <time className="num" dateTime={report.referenceDate}>{monthShort(report.referenceDate)}</time> : '—'}</td>
              </tr>;
            })}</tbody>
          </table></div> : <Empty title="Nenhum fundo para consultar">Cadastre fundos ou importe uma posição na carteira selecionada.</Empty>}
          {cvm.panel.items.length > 0 && <details className="extra-details market-data-details"><summary>Detalhes dos dados</summary>
            <div className="table-wrap" role="region" aria-label="Dados complementares e vínculos CNPJ dos informes CVM" tabIndex={0}><table className="market-cvm-details-table"><caption className="sr-only">Dados complementares e vínculos CNPJ dos informes mensais da CVM</caption><thead><tr><th scope="col">Fundo / CNPJ</th><th scope="col" className="num">Patrimônio líquido</th><th scope="col" className="num">Cotistas</th><th scope="col" className="num">Cotas emitidas</th><th scope="col">Entrega</th><th scope="col">Fonte e coleta</th><th scope="col">Vínculo</th></tr></thead><tbody>{cvm.panel.items.map((stored) => {
              const item = mappedItem(stored); const report = item.report;
              return <tr key={item.symbol}>
                <th scope="row"><span className="ticker">{item.symbol}</span>{report?.fundName && <span className="market-cell-note">{report.fundName}</span>}<span className="market-cell-note num">{item.cnpj ? cnpjLabel(item.cnpj) : 'CNPJ não vinculado'}</span></th>
                <td className="num right">{report ? nullableMoney(report.netAssets) : '—'}</td><td className="num right">{report?.shareholders == null ? '—' : quantity(report.shareholders)}</td><td className="num right">{report?.issuedShares == null ? '—' : quantity(report.issuedShares)}</td>
                <td>{report?.filedAt ? <time dateTime={report.filedAt}>{dateBR(report.filedAt)}</time> : '—'}</td>
                <td>{report ? <SourceCollected source={report.source} url={report.sourceUrl} fetchedAt={report.fetchedAt} /> : '—'}</td>
                <td><button ref={(element) => { if (element) mappingButtons.current.set(item.symbol, element); else mappingButtons.current.delete(item.symbol); }} className="text-link" type="button" disabled={!desktop || saving || cvm.refreshing} aria-label={`Vincular CNPJ de ${item.symbol}`} onClick={() => { setMapping({ symbol: item.symbol, cnpj: item.cnpj || '' }); setMappingError(''); setMappingNotice(''); }}>{item.cnpj ? 'Editar CNPJ' : 'Vincular CNPJ'}</button></td>
              </tr>;
            })}</tbody></table></div>
          {mapping && <form className="market-mapping" onSubmit={(event) => { event.preventDefault(); void saveMapping(); }} aria-label={`CNPJ de ${mapping.symbol}`}>
            <Field label={`CNPJ do fundo ${mapping.symbol}`} hint="Use o CNPJ do fundo, não o da administradora. O vínculo é compartilhado entre carteiras; não muda os dados financeiros."><input type="text" inputMode="numeric" autoComplete="off" value={mapping.cnpj} onChange={(event) => setMapping((old) => old && ({ ...old, cnpj: event.target.value }))} disabled={saving || cvm.refreshing} maxLength={18} autoFocus /></Field>
            {mappingError && <p className="form-error" role="alert">{mappingError}</p>}
            <div className="market-mapping-actions"><button className="button primary" type="submit" disabled={saving || cvm.refreshing || !mapping.cnpj.trim()}>{saving ? 'Salvando…' : 'Salvar vínculo'}</button><button className="button" type="button" disabled={saving} onClick={() => closeMapping(mapping.symbol)}>Cancelar</button><button className="text-link" type="button" disabled={saving || cvm.refreshing} onClick={() => void saveMapping(true)}>Remover vínculo</button></div>
          </form>}
          <p className="market-detail-note">Quando necessário, vincule o ticker ao CNPJ do fundo para consultar a CVM. O valor patrimonial é mensal e não representa a cotação de negociação.</p>
          </details>}
        </>}
      </div>
    </section>
  </>;
}

function RefreshButton({ name, busy, disabled, onClick }: { name: string; busy: boolean; disabled: boolean; onClick: () => Promise<void> }) {
  return <button className="button small" type="button" aria-label={name} disabled={!desktop || disabled || busy} onClick={() => void onClick()}>{busy ? <LoaderCircle className="spin" size={16} /> : <RefreshCw size={16} />}{busy ? 'Consultando…' : name}</button>;
}

function PanelMessages({ error, notice, errors }: { error: string; notice: string; errors: string[] }) {
  return <>{error && <p className="market-data-message form-error" role="alert">{error}</p>}{notice && <p className="market-data-message" role="status">{notice}</p>}{errors.length > 0 && <ul className="market-data-message market-panel-errors" aria-label="Avisos da consulta">{Array.from(new Set(errors)).map((issue) => <li key={issue}>{issue}</li>)}</ul>}</>;
}

function QuoteRow({ item }: { item: QuoteItem }) {
  const value = item.observation;
  const stale = value && item.status !== 'available';
  return <tr>
    <th scope="row" className="ticker">{item.symbol}</th>
    <td className="num right">{value ? quoteMoney(value.price, value.currency) : '—'}</td>
    <td>{value?.quotedAt ? <Stamp value={value.quotedAt} /> : <span className="market-cell-note">Não informada</span>}</td>
    <td><span className="market-cell-status">{statusLabels[item.status] || item.status}</span>{stale && item.status !== 'cached' && <span className="market-cell-note">Exibindo cotação salva</span>}{item.error && <span className="market-cell-error">{item.error}</span>}</td>
  </tr>;
}

function SourceCollected({ source, url, fetchedAt }: { source: string; url: string; fetchedAt: string }) {
  return <><PublicSourceLink className="market-source-link" href={url}>{source}</PublicSourceLink><span className="market-cell-note">Coleta <Stamp value={fetchedAt} /></span></>;
}

function nullableMoney(value: string | null) { return value == null || value.trim() === '' || !Number.isFinite(Number(value)) ? '—' : money(value); }
function quoteMoney(value: string, currency: string) {
  if (!value.trim() || !Number.isFinite(Number(value))) return '—';
  try { return new Intl.NumberFormat('pt-BR', { style: 'currency', currency }).format(Number(value)); }
  catch { return `${amount(value)} ${currency}`; }
}
function cnpjLabel(value: string) { return value.replace(/\D/g, '').replace(/^(\d{2})(\d{3})(\d{3})(\d{4})(\d{2})$/, '$1.$2.$3/$4-$5'); }

function useNews() {
  const publicData = usePublicData();
  const snapshot = publicData.news;
  const loading = publicData.loading;
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const request = useRef(0);
  useEffect(() => () => { request.current += 1; }, []);
  const refresh = async () => {
    const current = ++request.current;
    setRefreshing(true); setError(''); setNotice('');
    try {
      await publicData.refreshNews();
      if (current !== request.current) return;
      setNotice('Notícias atualizadas.');
    } catch (err) { if (current === request.current) setError(`${errorMessage(err)}${snapshot ? ' As manchetes salvas continuam disponíveis.' : ''}`); }
    finally { if (current === request.current) setRefreshing(false); }
  };
  return { snapshot, loading, refreshing: refreshing || publicData.refreshing, error, notice, refresh };
}

function News({ state }: { state: ReturnType<typeof useNews> }) {
  const { snapshot, loading, refreshing, error, notice, refresh } = state;
  const items = snapshot?.items.slice(0, 20) ?? [];
  // A coluna Fonte só aparece quando alguma manchete vem de outra fonte além da do feed.
  const showSource = items.some((item) => item.source !== snapshot?.source);
  const datesWithYear = items.some((item) => item.publishedAt && !isCurrentYear(item.publishedAt));
  return <section className="group" aria-labelledby="market-news-title">
    <div className="group-head market-news-head">
      <h2 id="market-news-title"><Def text="Manchetes de economia geral da Agência Brasil. São contexto macroeconômico, sem pesquisa ou filtro específico pelos ativos da carteira.">Notícias</Def></h2>
      <p>Agência Brasil</p>
      {notice && <p className="market-status" role="status">{notice}</p>}
      <div className="group-actions">
        <button className="button small" aria-label={refreshing ? 'Atualizando notícias' : 'Atualizar notícias'} disabled={!desktop || loading || refreshing} onClick={() => void refresh()}>{refreshing ? <LoaderCircle size={16} className="spin" /> : <RefreshCw size={16} />}{refreshing ? 'Atualizando…' : 'Atualizar'}</button>
      </div>
    </div>
    {error && <div className="alert error" role="alert">{error}</div>}
    <div className="block market-news">
      {loading ? <Loading /> : snapshot && items.length ? <>
        <div className="table-wrap">
          <table className="market-news-table">
            <caption className="sr-only">Manchetes de economia da {snapshot.source}</caption>
            <colgroup><col className={`market-col-date${datesWithYear ? ' with-year' : ''}`} /><col />{showSource && <col className="market-col-source" />}</colgroup>
            <thead><tr><th scope="col">Data</th><th scope="col">Manchete</th>{showSource && <th scope="col" className="market-news-source">Fonte</th>}</tr></thead>
            <tbody>{items.map((item) => <tr key={item.id}>
              <td className="market-news-date">{item.publishedAt ? <Stamp value={item.publishedAt} /> : <span className="market-news-nodate" title="Data de publicação não informada">Sem data</span>}</td>
              <td className="market-news-title"><PublicSourceLink className="market-headline" href={item.url} after={showSource ? <span className="market-news-source-inline">{SEP}{item.source}</span> : undefined}>{item.title}</PublicSourceLink></td>
              {showSource && <td className="market-news-source" title={item.source}>{item.source}</td>}
            </tr>)}</tbody>
          </table>
        </div>
        <details className="extra-details market-data-details"><summary>Detalhes dos dados</summary>
          <p className="market-src"><PublicSourceLink className="market-source-link" href={snapshot.sourceUrl} after={<>{SEP}{FETCHED}<Stamp value={snapshot.fetchedAt} />{SEP}salvo neste computador</>}>{snapshot.source}</PublicSourceLink></p>
        </details>
      </> : <Empty title="Nenhuma manchete salva">{desktop ? 'Atualize as notícias para carregar as manchetes mais recentes. Elas ficam salvas para leitura sem conexão.' : 'A consulta e o cache das notícias estão disponíveis no aplicativo Windows.'}</Empty>}
    </div>
  </section>;
}

/** Linha de origem no pé do indicador: "Banco Central · SGS 4389 · ref. 15/10/2026". */
function IndicatorSource({ indicator, showFetch }: { indicator: EconomicIndicator; showFetch: boolean }) {
  return <p className="market-src"><PublicSourceLink className="market-source-link" href={indicator.sourceUrl} after={<>{SEP}<span className="market-keep">ref. <time dateTime={indicator.referenceDate}>{referenceLabel(indicator)}</time></span>{showFetch && <>{SEP}{FETCHED}<Stamp value={indicator.fetchedAt} /></>}</>}>{indicator.source}</PublicSourceLink></p>;
}

/** Carimbo curto ("15/10 18:10"), sem quebra interna, com o valor completo no title. A hora pode descer de linha na tabela estreita. */
function Stamp({ value }: { value: string }) {
  const date = parseStamp(value);
  if (!date) return <time dateTime={value}>{value}</time>;
  return <time className="market-keep" dateTime={value} title={fullStamp(value)}>{shortDay(date)} <span className="market-stamp-time">{date.toLocaleTimeString('pt-BR', { hour: '2-digit', minute: '2-digit' })}</span></time>;
}

/** Duas casas, como o Banco Central publica (14,90). Um valor não numérico aparece como veio, nunca como 0,00. */
function formatRate(value: string) { const number = rateNumber(value); return number === null ? value : amount(number, 2); }
function rateNumber(value: string) { const number = value.trim() ? Number(value) : Number.NaN; return Number.isFinite(number) ? number : null; }
/** O IPCA é mensal: a data da série identifica o mês publicado. */
function referenceLabel(indicator: EconomicIndicator) { return indicator.id === 'ipca' ? monthShort(indicator.referenceDate) : dateBR(indicator.referenceDate); }
function parseStamp(value: string) { const date = new Date(value); return Number.isNaN(date.getTime()) ? null : date; }
function isCurrentYear(value: string) { const date = parseStamp(value); return !date || date.getFullYear() === new Date().getFullYear(); }
/** "15/10"; o ano só aparece quando não é o atual ("15/10/2025"). */
function shortDay(date: Date) { return date.toLocaleDateString('pt-BR', date.getFullYear() === new Date().getFullYear() ? { day: '2-digit', month: '2-digit' } : { day: '2-digit', month: '2-digit', year: 'numeric' }); }
function fullStamp(value: string) { const date = parseStamp(value); return date ? date.toLocaleString('pt-BR', { dateStyle: 'short', timeStyle: 'short' }).replace(', ', ' ') : value; }

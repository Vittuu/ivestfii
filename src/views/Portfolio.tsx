import { useEffect, useRef, useState } from 'react';
import { ChevronDown, Download, Pencil, Plus, Search, Upload } from 'lucide-react';
import { call, decimal, desktop, errorMessage, money, monthLabel, monthShort, percent, quantity } from '../lib/bridge';
import type { Fund, RecordEntry, Snapshot } from '../lib/bridge';
import { ledgerRows } from '../lib/ledger';
import type { LedgerRow } from '../lib/ledger';
import { Def, Empty, Field, handleForm, Modal, PageActions, SectionHeading, ShareBar, Submit, Tabs } from '../components';
import CompletePortfolio from './CompletePortfolio';

interface Props { walletId: string; snapshot: Snapshot; refresh: () => Promise<void>; onImport: () => void; onExport: () => void; notify: (text: string) => void; onImportPosition?: () => void; initialTab?: Tab }

type Tab = 'position' | 'history';
const tabs: { id: Tab; label: string }[] = [{ id: 'position', label: 'Posição' }, { id: 'history', label: 'Histórico' }];
const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;

export default function Portfolio({ walletId, snapshot, refresh, onImport, onExport, notify, onImportPosition, initialTab }: Props) {
  // Sem posição importada, a aba Posição só teria o convite para importar: abre no histórico.
  const [tab, setTab] = useState<Tab>(() => initialTab ?? (snapshot.brokerPosition ? 'position' : 'history'));
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState('');
  const [fundModal, setFundModal] = useState<Fund | 'new' | null>(null);
  const [recordModal, setRecordModal] = useState<{ fund: Fund; entry?: RecordEntry } | null>(null);
  const [detailModal, setDetailModal] = useState<{ fund: Fund; row: LedgerRow } | null>(null);
  const fund = snapshot.funds.find((item) => item.id === selected) || snapshot.funds[0];
  const visible = snapshot.funds.filter((item) => `${item.ticker} ${item.name} ${item.sector}`.toLowerCase().includes(query.toLowerCase()));
  const totalCost = Number(snapshot.totals.invested) || 0;
  const rows = fund ? ledgerRows(fund) : [];
  const purchases = rows.filter((row) => row.kind === 'purchase' || row.kind === 'both');
  const income = rows.filter((row) => row.kind === 'income' || row.kind === 'both');
  const otherRecords = rows.filter((row) => row.kind === 'information');

  return <div className="page portfolio-page">
    <PageActions>
      <button className="icon-button" onClick={onExport} disabled={!desktop} title="Exportar carteira completa" aria-label="Exportar carteira completa"><Download size={16} /></button>
      {/* Sem fundos, a ação principal passa a ser cadastrar o primeiro fundo. */}
      <button className={fund ? 'button' : 'button primary'} onClick={() => setFundModal('new')} disabled={!desktop}><Plus size={16} />Novo fundo</button>
      <button className={fund ? 'button primary' : 'button'} onClick={() => fund && setRecordModal({ fund })} disabled={!fund}><Plus size={16} />Novo lançamento</button>
    </PageActions>

    <Tabs id="portfolio" label="Seções de FIIs e posições" tabs={tabs} value={tab} onChange={setTab} />

    <div className="tab-panel" role="tabpanel" id="portfolio-panel-position" aria-labelledby="portfolio-tab-position" hidden={tab !== 'position'}>
      <CompletePortfolio position={snapshot.brokerPosition} detail onImportPosition={onImportPosition} />
    </div>

    <section className="group tab-panel" role="tabpanel" id="portfolio-panel-history" aria-labelledby="portfolio-tab-history" hidden={tab !== 'history'}>
      <div className="group-head">
        <h2><Def text="Compras e rendimentos registrados por mês. Um mesmo lançamento pode conter ambos. Este histórico é separado da posição importada; recebimentos acumulados não são saldo disponível em conta.">Histórico de FIIs</Def></h2>
        {snapshot.brokerPosition && <p><Def text="A posição mostra os saldos de uma importação. O histórico registra compras e recebimentos por mês e não é somado novamente à posição.">Não somado ao patrimônio</Def></p>}
        <div className="group-actions"><button type="button" className="button small" onClick={onImport} disabled={!desktop}><Upload size={16} />Importar histórico</button></div>
      </div>
      <p className="ledger-intro">Veja quanto você comprou e quanto recebeu de cada fundo. Escolha um fundo para ver os meses.</p>
      <div className="modular">
        <div className="block span-12 funds-block">
          <SectionHeading level={3} title="Fundos" meta={snapshot.funds.length ? `${plural(snapshot.funds.length, 'fundo', 'fundos')} · ${plural(snapshot.recordCount, 'lançamento', 'lançamentos')}` : undefined}>
            <label className="search-field"><Search size={16} aria-hidden="true" /><input aria-label="Buscar fundo no histórico" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Ticker, nome ou segmento" /></label>
          </SectionHeading>
          {visible.length ? <><div className="table-wrap"><table className="funds-table">
            <caption className="sr-only">Fundos do histórico com cotas, renda mensal estimada e proventos recebidos</caption>
            <thead><tr>
              <th scope="col">Fundo</th>
              <th scope="col" className="num">Cotas</th>
              <th scope="col" className="num"><Def text="Cotas atuais multiplicadas pela média dos últimos 6 proventos por cota registrados. O valor pode variar.">Renda estimada/mês</Def></th>
              <th scope="col" className="num"><Def text="Soma dos proventos registrados desde o primeiro lançamento. Não é saldo disponível em conta.">Proventos recebidos</Def></th>
              <th scope="col" className="col-action"><span className="sr-only">Ações</span></th>
            </tr></thead>
            <tbody>{visible.map((item) => {
              const current = item.id === fund?.id;
              return <tr key={item.id} className={current ? 'is-selected' : undefined}>
                <td className="fund-cell"><button className="fund-button" onClick={() => setSelected(item.id)} aria-label={`Ver lançamentos de ${item.ticker}`} aria-current={current ? 'true' : undefined} title={item.name || undefined}><span className="ticker">{item.ticker}</span><span className="fund-name">{item.name || 'Fundo imobiliário'}</span></button></td>
                <td className="num">{quantity(item.totalCotas)}</td>
                <td className="num">{money(item.estimatedMonthlyIncome)}</td>
                <td className="num">{money(item.dividendsReceived)}</td>
                <td className="col-action"><button className="icon-button" aria-label={`Editar ${item.ticker}`} title="Editar fundo" onClick={() => setFundModal(item)}><Pencil size={16} /></button></td>
              </tr>;
            })}</tbody>
            {!query && <tfoot><tr>
              <th scope="row">Total do histórico</th>
              <td className="num">{quantity(snapshot.totals.cotas)}</td>
              <td className="num">{money(snapshot.totals.estimatedMonthlyIncome)}</td>
              <td className="num">{money(snapshot.totals.dividendsReceived)}</td>
              <td />
            </tr></tfoot>}
          </table></div>
          <section className="portfolio-cost-summary" aria-label="Valores das compras por fundo">
            <div className="ledger-section-head"><h4>Compras por fundo</h4><p>Valores aplicados nas compras registradas.</p></div>
            <div className="table-wrap"><table className="fund-costs-table">
              <caption className="sr-only">Segmento, preço médio e custo das compras de cada fundo no histórico</caption>
              <thead><tr><th scope="col">Fundo</th><th scope="col">Segmento</th><th scope="col" className="num"><Def text="Média do preço das compras registradas. Não é a cotação atual do fundo.">Preço médio</Def></th><th scope="col" className="num"><Def text="Soma do valor das compras registradas no histórico. Não é o patrimônio atual.">Custo das compras</Def></th><th scope="col" className="share-cell"><Def text="Quanto este fundo representa do valor de todas as compras registradas. Exemplo: R$ 200 comprados neste fundo de R$ 1.000 comprados no total = 20%. Não indica valor atual nem ganho.">Parte do valor comprado</Def></th></tr></thead>
              <tbody>{visible.map((item) => <tr key={item.id}><td className="ticker">{item.ticker}</td><td className={item.sector ? 'fund-sector' : 'fund-sector muted'}>{item.sector || 'Não informado'}</td><td className="num">{money(item.averagePrice)}</td><td className="num">{money(item.totalInvested)}</td><td className="share-cell"><ShareBar value={Number(item.totalInvested) || 0} max={totalCost} label={percent(totalCost > 0 ? (Number(item.totalInvested) || 0) / totalCost * 100 : 0)} /></td></tr>)}</tbody>
              {!query && <tfoot><tr><th scope="row" colSpan={3}>Total das compras</th><td className="num">{money(snapshot.totals.invested)}</td><td /></tr></tfoot>}
            </table></div>
          </section></> : <Empty title={query ? 'Nenhum fundo encontrado' : 'Nenhum fundo no histórico'}>{query ? 'Tente outro ticker, nome ou segmento.' : 'Importe o JSON da versão anterior ou cadastre o primeiro fundo.'}</Empty>}
        </div>

        {fund && <div className="block span-12 entries-block">
          <div className="block-head">
            <h3>Lançamentos · <span className="ticker">{fund.ticker}</span></h3>
            <span className="block-meta">{fund.name || 'Fundo imobiliário'} · {plural(fund.entries.length, 'lançamento', 'lançamentos')}</span>
            <div className="block-actions">
              <label className="fund-select"><span className="sr-only">Selecionar fundo</span><select value={fund.id} onChange={(event) => setSelected(event.target.value)}>{snapshot.funds.map((item) => <option key={item.id} value={item.id}>{item.ticker}</option>)}</select><ChevronDown size={14} aria-hidden="true" /></label>
              <button className="button small" onClick={() => setRecordModal({ fund })}><Plus size={16} />Novo lançamento</button>
            </div>
          </div>
          <section className="ledger-section ledger-purchases" aria-label={`Compras de ${fund.ticker}`}>
            <div className="ledger-section-head"><h4>Compras · <span className="ticker">{fund.ticker}</span></h4></div>
            {purchases.length ? <div className="table-wrap" role="region" aria-label={`Tabela de compras de ${fund.ticker}`} tabIndex={0}><table className="purchases-table">
              <caption className="sr-only">Compras de {fund.ticker}, do mais recente ao mais antigo</caption>
              <thead><tr><th scope="col">Mês</th><th scope="col" className="num">Cotas compradas</th><th scope="col" className="num">Preço por cota</th><th scope="col" className="num">Total pago</th><th scope="col" className="col-action"><span className="sr-only">Detalhes e edição</span></th></tr></thead>
              <tbody>{purchases.map((row) => { const { entry, purchaseCost } = row; return <tr key={entry.id}>
                <td className="month-cell"><time className="num" dateTime={entry.month.slice(0, 7)} title={monthLabel(entry.month)}>{monthShort(entry.month)}</time></td>
                <td className="num">{quantity(entry.cotasAdded)}</td><td className="num">{money(entry.pricePerCota)}</td><td className="num ledger-paid">{money(purchaseCost)}</td>
                <td className="col-action"><span className="record-actions"><button className="text-link" type="button" aria-label={`Ver detalhes da compra de ${monthLabel(entry.month)}`} onClick={() => setDetailModal({ fund, row })}>Detalhes</button><button className="icon-button" aria-label={`Editar compra de ${monthLabel(entry.month)}`} title="Editar compra" onClick={() => setRecordModal({ fund, entry })}><Pencil size={16} /></button></span></td>
              </tr>; })}</tbody>
            </table></div> : <p className="ledger-empty">Nenhuma compra registrada neste fundo.</p>}
          </section>
          <section className="ledger-section ledger-receipts" aria-label={`Rendimentos de ${fund.ticker}`}>
            <div className="ledger-section-head"><h4>Rendimentos · <span className="ticker">{fund.ticker}</span></h4></div>
            {income.length ? <div className="table-wrap" role="region" aria-label={`Tabela de rendimentos de ${fund.ticker}`} tabIndex={0}><table className="income-table">
              <caption className="sr-only">Rendimentos recebidos de {fund.ticker}, do mais recente ao mais antigo</caption>
              <thead><tr><th scope="col">Mês</th><th scope="col" className="num"><Def text="Valor recebido informado no registro. Se o total está vazio, usa as cotas acumuladas até o fim do mês multiplicadas pelo rendimento por cota. Um total 0 informado não é recalculado.">Valor recebido</Def></th><th scope="col" className="col-action"><span className="sr-only">Detalhes e edição</span></th></tr></thead>
              <tbody>{income.map((row) => { const { entry, dividend } = row; return <tr key={entry.id}>
                <td className="month-cell"><time className="num" dateTime={entry.month.slice(0, 7)} title={monthLabel(entry.month)}>{monthShort(entry.month)}</time></td><td className="num ledger-received">{money(dividend)}</td>
                <td className="col-action"><span className="record-actions"><button className="text-link" type="button" aria-label={`Ver detalhes do rendimento de ${monthLabel(entry.month)}`} onClick={() => setDetailModal({ fund, row })}>Detalhes</button><button className="icon-button" aria-label={`Editar rendimento de ${monthLabel(entry.month)}`} title="Editar rendimento" onClick={() => setRecordModal({ fund, entry })}><Pencil size={16} /></button></span></td>
              </tr>; })}</tbody>
            </table></div> : <p className="ledger-empty">Nenhum rendimento recebido neste fundo.</p>}
          </section>
          {otherRecords.length > 0 && <details className="extra-details portfolio-entry-details">
            <summary>Outros registros</summary>
            <div className="table-wrap" role="region" aria-label={`Outros registros de ${fund.ticker}`} tabIndex={0}><table className="other-records-table">
              <caption className="sr-only">Registros sem compra ou recebimento positivo de {fund.ticker}</caption>
              <thead><tr><th scope="col">Mês</th><th scope="col">Registro</th><th scope="col" className="col-action"><span className="sr-only">Detalhes e edição</span></th></tr></thead>
              <tbody>{otherRecords.map((row) => { const { entry } = row; return <tr key={entry.id}>
                <td className="month-cell"><time className="num" dateTime={entry.month.slice(0, 7)} title={monthLabel(entry.month)}>{monthShort(entry.month)}</time></td><td>Sem compra ou recebimento</td>
                <td className="col-action"><span className="record-actions"><button className="text-link" type="button" aria-label={`Ver detalhes do registro de ${monthLabel(entry.month)}`} onClick={() => setDetailModal({ fund, row })}>Detalhes</button><button className="icon-button" aria-label={`Editar registro de ${monthLabel(entry.month)}`} title="Editar registro" onClick={() => setRecordModal({ fund, entry })}><Pencil size={16} /></button></span></td>
              </tr>; })}</tbody>
            </table></div>
          </details>}
        </div>}
      </div>
    </section>

    {fundModal && <FundForm walletId={walletId} fund={fundModal === 'new' ? undefined : fundModal} onClose={() => setFundModal(null)} onSaved={async () => { setFundModal(null); await refresh(); notify('Fundo salvo na carteira selecionada.'); }} />}
    {recordModal && <RecordForm walletId={walletId} {...recordModal} onClose={() => setRecordModal(null)} onSaved={async () => { setRecordModal(null); await refresh(); notify('Lançamento salvo. Indicadores atualizados.'); }} />}
    {detailModal && <RecordDetails {...detailModal} onClose={() => setDetailModal(null)} />}
  </div>;
}

function RecordDetails({ fund, row, onClose }: { fund: Fund; row: LedgerRow; onClose: () => void }) {
  const { entry } = row;
  return <Modal title={`Detalhes do lançamento · ${fund.ticker}`} onClose={onClose}>
    <dl className="record-facts">
      <div><dt>Fundo</dt><dd className="ticker">{fund.ticker}</dd></div>
      <div><dt>Mês</dt><dd><time dateTime={entry.month.slice(0, 7)}>{monthLabel(entry.month)}</time></dd></div>
      <div><dt>Cotas compradas</dt><dd className="num">{entry.cotasAdded.replace('.', ',')}</dd></div>
      <div><dt>Preço por cota</dt><dd className="num">{recordedMoney(entry.pricePerCota)}</dd></div>
      <div><dt>Total pago</dt><dd className="num ledger-paid">{money(row.purchaseCost)}</dd></div>
      <div><dt>Valor recebido</dt><dd className="num ledger-received">{money(row.dividend)}</dd></div>
      <div><dt>Rendimento por cota</dt><dd className="num">{recordedMoney(entry.dividendPerCota)}</dd></div>
    </dl>
    {entry.notes && <details className="extra-details record-source"><summary>Texto do arquivo de origem</summary><p>Guardado para conferir com o extrato.</p><pre>{entry.notes}</pre></details>}
    <div className="form-actions"><button className="button" type="button" onClick={onClose}>Voltar ao histórico</button></div>
  </Modal>;
}

/** Mantém todas as casas registradas, inclusive nas referências por cota. */
function recordedMoney(value: string) {
  const [whole, fraction = ''] = value.split('.');
  return `R$ ${whole.replace(/\B(?=(\d{3})+(?!\d))/g, '.')},${fraction.padEnd(2, '0')}`;
}

function FundForm({ walletId, fund, onClose, onSaved }: { walletId: string; fund?: Fund; onClose: () => void; onSaved: () => Promise<void> }) {
  const [ticker, setTicker] = useState(fund?.ticker || '');
  const [name, setName] = useState(fund?.name || '');
  const [sector, setSector] = useState(fund?.sector || '');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const save = async () => { setBusy(true); setError(''); try { await call('upsert_fund', { walletId, input: { id: fund?.id, ticker: ticker.trim().toUpperCase(), name: name.trim(), sector: sector.trim() } }); await onSaved(); } catch (err) { setError(errorMessage(err)); } finally { setBusy(false); } };
  return <Modal title={fund ? `Editar ${fund.ticker}` : 'Novo fundo'} description={fund ? undefined : 'Os lançamentos são registrados depois, no histórico.'} onClose={onClose}><form onSubmit={handleForm(save)}>
    <Field label="Ticker"><input className="num" autoFocus value={ticker} onChange={(event) => setTicker(event.target.value.toUpperCase())} placeholder="Ex.: HGLG11" required maxLength={16} /></Field>
    <Field label="Nome do fundo"><input value={name} onChange={(event) => setName(event.target.value)} placeholder="Opcional" maxLength={200} /></Field>
    <Field label="Segmento"><input value={sector} onChange={(event) => setSector(event.target.value)} placeholder="Ex.: Logística" list="sectors" maxLength={100} /><datalist id="sectors">{['Logística', 'Lajes corporativas', 'Shopping', 'Papel', 'Híbrido', 'Fundo de fundos', 'Renda urbana'].map((item) => <option key={item} value={item} />)}</datalist></Field>
    {error && <p className="form-error" role="alert">{error}</p>}
    <div className="form-actions"><button className="button secondary" type="button" onClick={onClose}>Cancelar</button><Submit busy={busy}>Salvar fundo</Submit></div>
  </form></Modal>;
}

function RecordForm({ walletId, fund, entry, onClose, onSaved }: { walletId: string; fund: Fund; entry?: RecordEntry; onClose: () => void; onSaved: () => Promise<void> }) {
  const [values, setValues] = useState({ month: entry?.month.slice(0, 7) || new Date().toLocaleDateString('sv-SE').slice(0, 7), cotasAdded: entry?.cotasAdded || '0', pricePerCota: entry?.pricePerCota || '0', dividendPerCota: entry?.dividendPerCota || '0', dividendTotal: entry?.dividendTotal ?? '', notes: entry?.notes || '' });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const savePending = useRef(false);
  const update = (name: keyof typeof values, value: string) => setValues((old) => ({ ...old, [name]: value }));
  const save = async () => { if (savePending.current) return; savePending.current = true; setBusy(true); setError(''); try { await call('save_record', { walletId, input: { id: entry?.id, fundId: fund.id, month: values.month, cotasAdded: decimal(values.cotasAdded), pricePerCota: decimal(values.pricePerCota), dividendPerCota: decimal(values.dividendPerCota), dividendTotal: values.dividendTotal.trim() ? decimal(values.dividendTotal) : null, notes: values.notes.trim() } }); await onSaved(); } catch (err) { setError(errorMessage(err)); } finally { savePending.current = false; setBusy(false); } };
  useEffect(() => { if (!entry) setValues((old) => ({ ...old, pricePerCota: fund.averagePrice })); }, [entry, fund.averagePrice]);
  return <Modal title={entry ? `Editar lançamento · ${fund.ticker}` : `Novo lançamento · ${fund.ticker}`} description="O mesmo lançamento pode registrar compras e rendimentos do mês." onClose={onClose} busy={busy}><form onSubmit={handleForm(save)}>
    <fieldset className="ledger-form-fields" disabled={busy}>
      <Field label="Mês de referência" hint="Mês ao qual este registro pertence. Não representa uma data exata de compra ou pagamento."><input type="month" autoFocus value={values.month} onChange={(event) => update('month', event.target.value)} required /></Field>
      <fieldset className="ledger-form-section"><legend>Compras</legend>
        <p className="ledger-form-guide">Se comprou neste mês, informe cotas e preço. Sem compra, deixe as cotas em 0.</p>
        <div className="form-grid">
          <Field label="Cotas adicionadas" hint="Quantidade comprada neste lançamento. Use 0 quando não houve compra."><input className="num" inputMode="decimal" value={values.cotasAdded} onChange={(event) => update('cotasAdded', event.target.value)} required /></Field>
          <Field label="Preço por cota (R$)" hint="Preço pago por cota. Um preço salvo com quantidade 0 fica preservado como informação e não gera uma compra."><input className="num" inputMode="decimal" value={values.pricePerCota} onChange={(event) => update('pricePerCota', event.target.value)} required /></Field>
        </div>
      </fieldset>
      <fieldset className="ledger-form-section"><legend>Rendimentos</legend>
        <p className="ledger-form-guide">Se recebeu, informe o total ou o rendimento por cota. Sem recebimento, use total 0.</p>
        <div className="form-grid">
          <Field label="Total recebido (R$)" hint="Um total preenchido prevalece, inclusive 0. Vazio: calculado pelas cotas acumuladas até o fim do mês, incluindo as compras do mês, multiplicadas pelo rendimento por cota."><input className="num" inputMode="decimal" value={values.dividendTotal} onChange={(event) => update('dividendTotal', event.target.value)} placeholder="Vazio: calcular por cota" /></Field>
          <Field label="Rendimento por cota (R$)" hint="Use para calcular o recebimento quando o total estiver vazio. Se você preencheu o total, esse total é mantido."><input className="num" inputMode="decimal" value={values.dividendPerCota} onChange={(event) => update('dividendPerCota', event.target.value)} required /></Field>
        </div>
      </fieldset>
      <Field label="Observações"><textarea value={values.notes} onChange={(event) => update('notes', event.target.value)} placeholder="Opcional" rows={3} /></Field>
    </fieldset>
    {error && <p className="form-error" role="alert">{error}</p>}
    <div className="form-actions"><button className="button secondary" type="button" onClick={onClose} disabled={busy}>Cancelar</button><Submit busy={busy}>Salvar lançamento</Submit></div>
  </form></Modal>;
}

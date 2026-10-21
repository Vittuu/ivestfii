import { useId, useState } from 'react';
import type { ReactNode } from 'react';
import { Upload } from 'lucide-react';
import { amount, dateBR, desktop, money, percent, quantity } from '../lib/bridge';
import type { BrokerPosition } from '../lib/bridge';
import { Def, Empty, Modal, SectionHeading, ShareBar } from '../components';

interface Props {
  position?: BrokerPosition | null;
  /** Resumo (Visão geral: patrimônio e composição) ou detalhe (aba Posição: fundos listados, pós-fixado e saldo). */
  detail?: boolean;
  onImportPosition?: () => void;
  /** Mostra "Atualizar posição" no cabeçalho do grupo. A Visão geral desliga porque a ação já está na barra superior. */
  showImportAction?: boolean;
  /** Abre o detalhe do pós-fixado sem substituir a data do panorama importado. */
  onShowFixedIncome?: () => void;
}

const sign = (value: string) => Number(value) > 0 ? '+' : Number(value) < 0 ? '−' : '';
const resultClass = (value: string) => Number(value) < 0 ? 'value-negative' : Number(value) > 0 ? 'value-positive' : '';
/** Resultado com sinal antes da moeda, como no money(): "−R$ 21,51", "+R$ 116,06". */
const resultLabel = (value: string) => `${sign(value)}${money(Math.abs(Number(value) || 0))}`;
const timeLabel = (time: string | null) => time || 'horário não informado';
const positionDefinition = 'Retrato da carteira informado pela corretora na data exibida. Não é uma atualização ao vivo. O Histórico registra compras e rendimentos no aplicativo; esses lançamentos não alteram a posição importada.';
const positionNote = 'Retrato da corretora na data informada; não é atualizado ao vivo.';
const importedLabel = (value: string) => {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString('pt-BR', { dateStyle: 'short', timeStyle: 'short' });
};

/** Valor em reais com sinal e cor de resultado, no formato dos indicadores: o sinal vem antes da moeda, como na tabela. */
function ResultFigure({ value }: { value: string }) {
  return <span className={`figure-value ${resultClass(value)}`}><span className="cp-signed">{sign(value)}<span className="unit">R$</span></span>{amount(Math.abs(Number(value) || 0))}</span>;
}

type ObservationTopic = 'funds' | 'fixed' | 'pending' | 'costs' | 'percentages' | 'source';
interface ImportObservation { topic: ObservationTopic; title: string; explanation: string; affected: string; location: string; originals: string[] }

/** Free-form import notes remain verbatim; only recognizable subjects receive a plain-language heading. */
function importObservations(position: BrokerPosition): ImportObservation[] {
  const { overview, listedFunds, fixedIncome } = position;
  const groups = new Map<ObservationTopic, ImportObservation>();
  const definitions: Record<ObservationTopic, Omit<ImportObservation, 'originals'>> = {
    funds: { topic: 'funds', title: Number(listedFunds.marketValue) !== Number(overview.listedFunds) ? 'Valores diferentes nas capturas dos fundos' : 'Valores das capturas dos fundos', explanation: 'O resumo e o detalhe foram importados separadamente. Confira os valores e os horários; um não substitui o outro.', affected: `Resumo: ${money(overview.listedFunds)} · Detalhe: ${money(listedFunds.marketValue)}`, location: 'Fundos listados' },
    fixed: { topic: 'fixed', title: fixedIncome.latestObservation ? `Registro anterior do pós-fixado (${dateBR(position.referenceDate)})` : 'Nome e período do pós-fixado', explanation: fixedIncome.latestObservation ? `Esta observação é do registro de ${dateBR(position.referenceDate)}. O complemento mais recente já informa o nome completo e separa o resultado no saldo do rendimento em 12 meses. Os valores antigos permanecem no registro anterior.` : 'O rendimento informado não tem um período definido na fonte. Confira o nome completo e o período no documento da corretora. Esse valor não foi lançado como um novo pagamento.', affected: `${fixedIncome.name} · Rendimento informado${fixedIncome.latestObservation ? ' no registro anterior' : ''}: ${resultLabel(fixedIncome.reportedEarnings)}`, location: fixedIncome.latestObservation ? `Posição → Pós-fixado → Registro anterior de ${dateBR(position.referenceDate)}` : 'Posição → Pós-fixado' },
    pending: { topic: 'pending', title: 'Proventos da posição', explanation: 'Esse valor já faz parte dos investimentos da posição. Não foi acrescentado aos rendimentos recebidos no Histórico.', affected: `Proventos a receber: ${money(overview.pendingIncome)}`, location: 'Saldo e proventos / Composição' },
    costs: { topic: 'costs', title: 'Compras e posição são registros diferentes', explanation: 'Os dados da corretora não reescrevem as compras do Histórico. O custo da posição pode ser diferente do valor comprado que você registrou.', affected: `Custo informado pela posição: ${money(listedFunds.costBasis)}`, location: 'Posição → Custos dos fundos' },
    percentages: { topic: 'percentages', title: 'Percentuais do documento', explanation: 'A nota trata dos percentuais que aparecem no documento da corretora. Na composição do aplicativo, a participação usa os valores da posição importada.', affected: 'Composição do patrimônio', location: 'Composição' },
    source: { topic: 'source', title: 'Como os dados foram importados', explanation: 'A fonte trouxe uma observação sobre a importação. Consulte o texto original antes de interpretar esses dados.', affected: `Posição de ${dateBR(position.referenceDate)}`, location: 'Origem da posição' },
  };
  const ensure = (topic: ObservationTopic) => {
    if (!groups.has(topic)) groups.set(topic, { ...definitions[topic], originals: [] });
    return groups.get(topic)!;
  };
  if (Number(listedFunds.marketValue) !== Number(overview.listedFunds)) ensure('funds');
  if (fixedIncome.nameTruncated) ensure('fixed');
  for (const original of position.warnings) {
    const text = original.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase();
    const topic: ObservationTopic = /nome|pos.fixado|periodo.*rendimento|rendimento.*periodo|rendimento exibido/.test(text) ? 'fixed'
      : /cust|precos|compras histor|quantidades.*compras/.test(text) ? 'costs'
      : /proventos/.test(text) ? 'pending'
      : /participa|percent|denominador/.test(text) ? 'percentages'
      : /captur|horari|panorama|cada conjunto/.test(text) ? 'funds' : 'source';
    ensure(topic).originals.push(original);
  }
  return [...groups.values()];
}

export default function CompletePortfolio({ position, detail = false, onImportPosition, showImportAction = true, onShowFixedIncome }: Props) {
  const headingId = useId();
  const [observationsOpen, setObservationsOpen] = useState(false);

  if (!position) return <section className="group complete-portfolio" aria-labelledby={headingId}>
    <div className="group-head"><h2 id={headingId}><Def text={positionDefinition}>Posição na corretora</Def></h2></div>
    <p className="cp-position-note">{positionNote}</p>
    <div className="block cp-empty">
      <Empty title="Nenhuma posição importada" action={onImportPosition ? <button type="button" className="button" onClick={onImportPosition} disabled={!desktop}><Upload size={16} />Importar posição</button> : undefined}>
        Importe o arquivo de posição da corretora para ver patrimônio total, pós-fixado e saldo em conta, com a data de referência de cada valor.{!onImportPosition && ' A importação da posição fica em Configurações.'}
      </Empty>
    </div>
  </section>;

  const { overview, listedFunds, fixedIncome } = position;
  const latestFixed = fixedIncome.latestObservation;
  const total = Number(overview.totalWithCash);
  const pendingDef = 'Valor que a corretora classificou como proventos. Já faz parte do total de investimentos; a fonte não detalha sua natureza.';
  const reference = dateBR(position.referenceDate);
  const marketDiffers = Number(listedFunds.marketValue) !== Number(overview.listedFunds);
  const observations = importObservations(position);
  const fieldId = (topic: string) => `${headingId}-${topic}`;
  const showField = (topic: ObservationTopic) => {
    setObservationsOpen(false);
    requestAnimationFrame(() => {
      const target = document.getElementById(fieldId(topic === 'fixed' && latestFixed ? 'fixed-previous' : topic));
      if (!target) return;
      let ancestor: HTMLElement | null = target;
      while (ancestor) { if (ancestor instanceof HTMLDetailsElement) ancestor.open = true; ancestor = ancestor.parentElement; }
      target.scrollIntoView({ block: 'center', behavior: 'instant' });
      target.focus({ preventScroll: true });
    });
  };
  const canShowField = (topic: ObservationTopic) => topic === 'funds' || topic === 'pending' || topic === 'source' || (detail ? topic !== 'percentages' : topic === 'percentages');
  const origin = <>{position.institution} · ref. <time dateTime={position.referenceDate}>{reference}</time>{overview.updatedTime && <>, atualizada às {overview.updatedTime}</>}</>;
  const importAction = showImportAction && onImportPosition && <div className="group-actions"><button type="button" className="button small" onClick={onImportPosition} disabled={!desktop}><Upload size={16} />Atualizar posição</button></div>;
  const difference = marketDiffers && <div className="cp-value-note"><span>Fundos: <span className="num">{money(overview.listedFunds)}</span> no resumo · <span className="num">{money(listedFunds.marketValue)}</span> no detalhe.</span><button type="button" className="text-link" onClick={() => setObservationsOpen(true)}>Entender diferença</button></div>;
  const notes = <>
    {observations.length > 0 && <div className="cp-import-notice span-12" role="note">
      <span><strong>Observações da importação</strong><span>Confira o contexto de alguns valores da corretora.</span></span>
      <button type="button" className="button small" onClick={() => setObservationsOpen(true)}>Ver observações ({observations.length})</button>
    </div>}
    <details className="extra-details cp-note span-12 cp-field-target" id={fieldId('source')} tabIndex={-1}>
      <summary>Ver origem da posição</summary>
      <dl className="cp-source-list">
        <dt>Instituição</dt><dd>{position.institution}</dd>
        <dt>Data de referência</dt><dd className="num">{reference}</dd>
        <dt>Atualização exibida</dt><dd className="num">{timeLabel(overview.updatedTime)}</dd>
        <dt>Captura do panorama</dt><dd className="num">{timeLabel(overview.capturedTime)}</dd>
        <dt>Captura dos fundos listados</dt><dd className="num">{timeLabel(listedFunds.capturedTime)}</dd>
        <dt>Importação</dt><dd className="num">{importedLabel(position.importedAt)}</dd>
        <dt>Arquivo de origem</dt><dd><code>{position.sourcePath}</code></dd>
        <dt>SHA-256</dt><dd><code>{position.sourceHash}</code></dd>
      </dl>
    </details>
    {observationsOpen && <Modal title="Observações da importação" description="Organizadas por assunto. Os textos originais da fonte permanecem em cada observação." onClose={() => setObservationsOpen(false)}>
      <div className="cp-observations"><table className="cp-observations-table">
        <thead><tr><th scope="col">O que conferir</th><th scope="col">Onde aparece</th></tr></thead>
        <tbody>{observations.map((observation) => <tr key={observation.topic}>
          <td><h3>{observation.title}</h3><p>{observation.explanation}</p>{observation.originals.length > 0 && <details className="extra-details cp-original-notes"><summary>Texto da importação ({observation.originals.length})</summary><ul>{observation.originals.map((original, index) => <li key={index}>{original}</li>)}</ul></details>}</td>
          <td><strong className="cp-affected-value">{observation.affected}</strong><p>{observation.location}</p>{canShowField(observation.topic) && <button className="text-link" type="button" onClick={() => showField(observation.topic)}>Ver no quadro</button>}</td>
        </tr>)}</tbody>
      </table></div>
      <div className="form-actions"><button type="button" className="button" onClick={() => setObservationsOpen(false)}>Concluir</button></div>
    </Modal>}
  </>;

  if (!detail) {
    const composition: { label: ReactNode; key: string; value: string }[] = [
      { key: 'listed', label: <Def text="Fundos negociados na bolsa, como FIIs. Valor da posição importada, não uma nova cotação.">Fundos listados</Def>, value: overview.listedFunds },
      { key: 'fixed', label: <Def text="Investimento cujo rendimento acompanha uma taxa, como o CDI. Valor informado pela corretora nesta data.">Pós-fixado</Def>, value: overview.fixedIncome },
      { key: 'cash', label: 'Saldo em conta', value: overview.cash },
      { key: 'pending', label: <Def text={pendingDef}>Proventos a receber</Def>, value: overview.pendingIncome },
    ];
    const largest = Math.max(0, ...composition.map((item) => Number(item.value) || 0));
    return <section className="group complete-portfolio" aria-labelledby={headingId}>
      <div className="group-head">
        <h2 id={headingId}><Def text={positionDefinition}>Posição na corretora</Def></h2>
        {importAction}
      </div>
      <p className="cp-position-note">{positionNote}</p>
      <div className="modular">
        <div className="block span-5 cp-hero">
          <span className="figure-label"><Def text="Valor total dos investimentos e do saldo em conta na data da posição. O saldo já está incluído.">Patrimônio total</Def></span>
          <div className="cp-hero-value">
            <p className="figure-hero"><span className="unit">R$</span>{amount(overview.totalWithCash)}</p>
          </div>
          <p className="cp-source">Posição {origin}</p>
        </div>
        <div className="block span-7 cp-composition-block cp-field-target" id={fieldId('percentages')} tabIndex={-1}>
          <SectionHeading title="Composição" level={3} />
          <div className="table-wrap">
            <table className="cp-table cp-composition">
              <caption className="sr-only">Composição do patrimônio em {reference}, panorama capturado às {timeLabel(overview.capturedTime)}</caption>
              <colgroup><col /><col className="cp-col-value" /><col className="cp-col-share" /></colgroup>
              <thead><tr><th scope="col"><Def text="Tipo de investimento ou de saldo informado pela corretora.">Classe</Def></th><th scope="col" className="num">Valor</th><th scope="col"><Def text="Sobre o patrimônio total do mesmo panorama, incluindo o saldo em conta">Participação</Def></th></tr></thead>
              <tbody>{composition.map((item) => <tr key={item.key} className="cp-field-target" id={item.key === 'listed' ? fieldId('funds') : item.key === 'pending' ? fieldId('pending') : undefined} tabIndex={item.key === 'listed' || item.key === 'pending' ? -1 : undefined}>
                <td>{item.label}</td>
                <td className="num">{money(item.value)}</td>
                <td>{total > 0 ? <ShareBar value={Number(item.value) || 0} max={largest} label={percent(Number(item.value) / total * 100)} /> : <span className="dash">—</span>}</td>
              </tr>)}</tbody>
            </table>
          </div>
          {difference}
        </div>
        {latestFixed && <div className="cp-fixed-update-note span-12" role="note"><span>Pós-fixado: saldo de <time dateTime={latestFixed.referenceDate}>{dateBR(latestFixed.referenceDate)}</time>, <span className="num">{money(latestFixed.balance)}</span>. O patrimônio acima continua com os valores de {reference}.</span>{onShowFixedIncome ? <button type="button" className="text-link" onClick={onShowFixedIncome}>Ver pós-fixado</button> : <span>Consulte FIIs e posições → Posição → Pós-fixado.</span>}</div>}
        {notes}
      </div>
    </section>;
  }

  return <section className="group complete-portfolio cp-detail" aria-labelledby={headingId}>
    <div className="group-head">
      <h2 id={headingId}><Def text={positionDefinition}>Posição na corretora</Def></h2>
      <p>{origin}</p>
      {importAction}
    </div>
    <p className="cp-position-note">{positionNote}</p>
    <div className="modular">
      <div className="block span-12 cp-listed cp-field-target" id={fieldId('funds')} tabIndex={-1}>
        <SectionHeading title="Fundos listados" level={3} meta={<>captura às <span className="num">{timeLabel(listedFunds.capturedTime)}</span></>} />
        <div className="readouts cp-readouts">
          <div className="readout"><span className="figure-label"><Def text="Soma do valor dos fundos nesta captura da corretora. Não é uma cotação atualizada automaticamente.">Valor de mercado</Def></span><span className="figure-value"><span className="unit">R$</span>{amount(listedFunds.marketValue)}</span></div>
          <div className="readout"><span className="figure-label"><Def text="Ganho ou perda informado pela corretora em relação ao custo. O valor pode mudar enquanto você mantém as cotas.">Resultado sobre o custo</Def></span><ResultFigure value={listedFunds.unrealizedResult} /></div>
        </div>
        {difference}
        {listedFunds.holdings.length ? <div className="table-wrap">
          <table className="cp-table cp-holdings">
            <caption className="sr-only">Posição de FIIs e fundos listados em {reference}, captura às {timeLabel(listedFunds.capturedTime)}</caption>
            <thead><tr>
              <th scope="col">Fundo</th>
              <th scope="col" className="num">Cotas</th>
              <th scope="col" className="num"><Def text="Valor total das cotas deste fundo na posição importada.">Valor</Def></th>
              <th scope="col" className="num"><Def text="Ganho ou perda informado pela corretora; não representa um valor já realizado na venda.">Resultado</Def></th>
            </tr></thead>
            <tbody>{listedFunds.holdings.map((holding) => <tr key={holding.ticker}>
              <td className="ticker">{holding.ticker}</td>
              <td className="num">{quantity(holding.shares)}</td>
              <td className="num">{money(holding.marketValue)}</td>
              <td className={`num ${resultClass(holding.unrealizedResult)}`}>{resultLabel(holding.unrealizedResult)}</td>
            </tr>)}</tbody>
          </table>
        </div> : <p className="cp-empty-line">A fonte não traz o detalhamento dos fundos listados.</p>}
        <details className="extra-details cp-fund-extra cp-field-target" id={fieldId('costs')} tabIndex={-1}>
          <summary>Ver custos e detalhes dos fundos</summary>
          <div className="readouts cp-readouts">
            <div className="readout"><span className="figure-label"><Def text="Valor de mercado menos o resultado informado pela corretora. Pode divergir do custo das compras registradas no histórico.">Custo inferido</Def></span><span className="figure-value"><span className="unit">R$</span>{amount(listedFunds.costBasis)}</span></div>
            <div className="readout"><span className="figure-label"><Def text="Variação do dia informada na captura da corretora.">Resultado do dia</Def></span><ResultFigure value={listedFunds.dailyResult} /></div>
          </div>
          {listedFunds.holdings.length > 0 && <div className="table-wrap"><table className="cp-table cp-holding-costs">
            <caption className="sr-only">Preços e custos dos fundos em {reference}</caption>
            <thead><tr><th scope="col">Fundo</th><th scope="col" className="num"><Def text="Preço médio exibido pela corretora, arredondado.">Preço médio</Def></th><th scope="col" className="num"><Def text="Preço de uma cota na captura da corretora.">Preço de mercado</Def></th><th scope="col" className="num"><Def text="Valor de mercado menos o resultado. Pode divergir do custo das compras registradas.">Custo inferido</Def></th></tr></thead>
            <tbody>{listedFunds.holdings.map((holding) => <tr key={holding.ticker}><td className="ticker">{holding.ticker}</td><td className="num">{money(holding.averagePrice)}</td><td className="num">{money(holding.price)}</td><td className="num">{money(holding.costBasis)}</td></tr>)}</tbody>
          </table></div>}
        </details>
      </div>

      <div className={`block ${latestFixed ? 'span-12 cp-fixed-observation' : 'span-7'} cp-fixed cp-field-target`} id={fieldId('fixed')} tabIndex={-1}>
        <SectionHeading title="Pós-fixado" level={3} meta={latestFixed ? <>Saldo de <time dateTime={latestFixed.referenceDate}>{dateBR(latestFixed.referenceDate)}</time></> : undefined} />
        {latestFixed ? <>
          <p className="cp-product">{latestFixed.name}<small className="cp-fixed-classification">{latestFixed.category} · Risco: {latestFixed.riskLevel}</small></p>
          <div className="readouts cp-readouts cp-fixed-current-readouts">
            <div className="readout"><span className="figure-label"><Def text="Saldo informado pela corretora nesta data. Inclui o rendimento que permanece no fundo.">Saldo informado</Def></span><span className="figure-value"><span className="unit">R$</span>{amount(latestFixed.balance)}</span></div>
            <div className="readout cp-fixed-applied"><span className="figure-label"><Def text="Dinheiro aplicado nos aportes informados pela corretora, sem o rendimento.">Valor aplicado</Def></span><span className="figure-value"><span className="unit">R$</span>{amount(latestFixed.invested)}</span></div>
            <div className="readout"><span className="figure-label"><Def text="Diferença entre o saldo e o valor aplicado. Esse rendimento já está no saldo; não é um pagamento separado.">Rendimento no saldo</Def></span><ResultFigure value={latestFixed.totalEarnings} /></div>
          </div>
          <p className="cp-fixed-explanation">O rendimento já faz parte do saldo do fundo. Não é somado novamente como provento recebido.</p>
          <div className="block-head"><h4>Aportes</h4><span className="block-meta">{latestFixed.contributions.length} aplicações informadas</span></div>
          <div className="table-wrap"><table className="cp-table cp-fixed-contributions">
            <caption className="sr-only">Aportes do pós-fixado</caption>
            <thead><tr><th scope="col"><Def text="Data em que o dinheiro foi aplicado neste aporte.">Data da aplicação</Def></th><th scope="col" className="num cp-fixed-applied">Aplicado</th><th scope="col" className="num"><Def text="Valor atual informado pela corretora para este aporte. Inclui o rendimento do aporte.">Saldo informado</Def></th><th scope="col" className="num"><Def text="Quanto este aporte rendeu desde a aplicação. Já está incluído no saldo informado.">Rendimento</Def></th><th scope="col" className="num"><Def text="Rendimento deste aporte em relação ao valor aplicado, desde a data da aplicação. Não é uma taxa anual.">Retorno do aporte</Def></th></tr></thead>
            <tbody>{latestFixed.contributions.map((contribution, index) => <tr key={`${contribution.applicationDate}-${index}`}>
              <td className="cp-fixed-date"><time dateTime={contribution.applicationDate}>{dateBR(contribution.applicationDate)}</time></td><td className="num cp-fixed-applied">{money(contribution.invested)}</td><td className="num">{money(contribution.currentValue)}</td><td className={`num ${resultClass(contribution.earnings)}`}>{resultLabel(contribution.earnings)}</td><td className={`num ${resultClass(contribution.returnPercent)}`}>{percent(contribution.returnPercent)}</td>
            </tr>)}</tbody>
          </table></div>
          <div className="cp-fixed-period">
            <div className="block-head"><h4>Últimos 12 meses</h4><span className="block-meta">Até <time dateTime={latestFixed.returnReferenceDate}>{dateBR(latestFixed.returnReferenceDate)}</time></span></div>
            <dl className="facts"><div><dt><Def text="Rendimento em reais para os últimos 12 meses mostrados pela corretora, até a data deste quadro. É um período diferente do rendimento total no saldo.">Rendimento em 12 meses</Def></dt><dd className={resultClass(latestFixed.earnings12Months)}>{resultLabel(latestFixed.earnings12Months)}</dd></div><div><dt><Def text="Rentabilidade dos últimos 12 meses exibida pela corretora, até a data deste quadro. Não é o retorno acumulado de todos os aportes.">Rentabilidade em 12 meses</Def></dt><dd className={resultClass(latestFixed.return12Months)}>{percent(latestFixed.return12Months)}</dd></div></dl>
          </div>
          <details className="extra-details cp-fixed-extra cp-fixed-origin"><summary>Origem do complemento</summary><dl className="cp-source-list"><dt>Importação</dt><dd>{importedLabel(latestFixed.importedAt)}</dd><dt>Arquivo do complemento</dt><dd><code>{latestFixed.sourcePath}</code></dd><dt>SHA-256 do complemento</dt><dd><code>{latestFixed.sourceHash}</code></dd>{latestFixed.sources.map((source, index) => <div className="cp-fixed-source" key={`${source.sha256}-${index}`}><dt>Imagem de origem</dt><dd>{source.fileName}<code>{source.sha256}</code></dd></div>)}</dl></details>
          <details className="extra-details cp-fixed-extra cp-fixed-previous cp-field-target" id={fieldId('fixed-previous')} tabIndex={-1}>
            <summary>Registro anterior de {reference}</summary>
            <p className="cp-fixed-explanation">Valores do registro anterior. Saldo líquido e prazos abaixo foram informados nessa data.</p>
            <p className="cp-product">{fixedIncome.name}{fixedIncome.nameTruncated && <small className="cp-inline-note">Nome abreviado no registro anterior</small>}</p>
            <dl className="facts"><div><dt>Saldo bruto em {reference}</dt><dd>{money(fixedIncome.balance)}</dd></div><div><dt>Saldo líquido em {reference}</dt><dd>{money(fixedIncome.netBalance)}</dd></div><div className="cp-fact-full"><dt><Def text="Valor que a fonte antiga apresentou sem definir o período. Permanece neste registro; não é um novo pagamento.">Rendimento informado no registro anterior</Def><small className="cp-inline-note">Período não informado na fonte antiga</small></dt><dd className={resultClass(fixedIncome.reportedEarnings)}>{resultLabel(fixedIncome.reportedEarnings)}</dd></div><div><dt>Rentabilidade em 12 meses na fonte antiga</dt><dd>{percent(fixedIncome.return12Months)}</dd></div><div><dt>Cotização do resgate</dt><dd className="text">{fixedIncome.redemptionConversion}</dd></div><div><dt>Liquidação do resgate</dt><dd className="text">{fixedIncome.redemptionSettlement}</dd></div></dl>
          </details>
        </> : <>
        <p className="cp-product">{fixedIncome.nameTruncated ? <><Def text="Nome abreviado na fonte">{fixedIncome.name}</Def><small className="cp-inline-note">Nome abreviado na corretora</small></> : fixedIncome.name}</p>
        <dl className="facts">
          <div><dt><Def text="Valor do investimento antes dos descontos indicados pela corretora.">Saldo bruto</Def></dt><dd>{money(fixedIncome.balance)}</dd></div>
          <div><dt><Def text="Valor líquido informado pela corretora na data da captura.">Saldo líquido</Def></dt><dd>{money(fixedIncome.netBalance)}</dd></div>
          <div className="cp-fact-full"><dt><Def text="Valor apresentado pela corretora sem informar a qual período se refere. Não foi somado aos rendimentos recebidos no Histórico.">Rendimento informado</Def><small className="cp-inline-note">Período não informado</small></dt><dd className={resultClass(fixedIncome.reportedEarnings)}>{resultLabel(fixedIncome.reportedEarnings)}</dd></div>
        </dl>
        <details className="extra-details cp-fixed-extra">
          <summary>Ver rendimento e resgate</summary>
          <dl className="facts">
          <div><dt><Def text="Percentual dos últimos 12 meses informado pela corretora. O resultado passado não garante o próximo.">Rentabilidade em 12 meses</Def></dt><dd>{percent(fixedIncome.return12Months)}</dd></div>
          <div><dt><Def text="Prazo para definir o valor das cotas solicitadas no resgate.">Cotização do resgate</Def></dt><dd className="text">{fixedIncome.redemptionConversion}</dd></div>
          <div><dt><Def text="Prazo informado para o dinheiro do resgate ficar disponível.">Liquidação do resgate</Def></dt><dd className="text">{fixedIncome.redemptionSettlement}</dd></div>
          </dl>
        </details>
        </>}
      </div>

      <div className={`block ${latestFixed ? 'span-12' : 'span-5'} cp-cash`}>
        <SectionHeading title="Saldo e proventos" level={3} meta={latestFixed ? <>Registro de <time dateTime={position.referenceDate}>{reference}</time></> : undefined} />
        <dl className="facts">
          <div><dt><Def text="Entra no patrimônio total">Saldo em conta</Def></dt><dd>{money(overview.cash)}</dd></div>
          <div className="cp-field-target" id={fieldId('pending')} tabIndex={-1}><dt><Def text={pendingDef}>Proventos a receber</Def></dt><dd>{money(overview.pendingIncome)}</dd></div>
        </dl>
      </div>
      {notes}
    </div>
  </section>;
}

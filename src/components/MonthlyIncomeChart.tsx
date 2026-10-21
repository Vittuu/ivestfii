import { useId } from 'react';
import { amount, dateBR, money, monthShort, quantity } from '../lib/bridge';
import type { MonthlyIncomePoint } from '../lib/monthlyIncome';
import { Def } from '../components';
import PublicSourceLink from './PublicSourceLink';

const chartHeight = 216;
const plotBottom = 148;
const plotTop = 34;

function monthStatus(point: MonthlyIncomePoint): string {
  return point.incomeRecorded > 0 ? money(point.total) : !point.recorded ? 'Sem registros' : 'Sem rendimento registrado';
}

function MonthExplanation({ point, partial, until }: { point: MonthlyIncomePoint; partial: boolean; until: string }) {
  const funds = point.funds.filter((fund) => fund.status !== 'not-held' || fund.recordCount > 0);
  const hasCarriedShares = funds.some((fund) => /[1-9]/.test(fund.held)) && point.purchaseCount === 0;
  const hasZero = funds.some((fund) => fund.income !== null && Number(fund.income) === 0 && fund.recordedRecordCount > 0);
  const apiFunds = funds.filter((fund) => fund.status === 'api');
  return <div className="ov-income-tooltip">
    <p className="ov-income-tooltip-title"><strong>{monthShort(point.key)}</strong> · {monthStatus(point)}{partial && <span> · Parcial até {until}</span>}</p>
    {point.expectedFunds.length > 0 && <p>{point.incomeRecorded} de {point.expectedFunds.length} {point.expectedFunds.length === 1 ? 'fundo' : 'fundos'} com rendimento {apiFunds.length ? 'disponível' : 'registrado'}.</p>}
    {!point.recorded && apiFunds.length === 0 && <p>Nenhum lançamento neste mês. Isso não confirma rendimento zero.</p>}
    {apiFunds.length > 0 && <p>Preenchido automaticamente com proventos oficiais. Um valor informado por você tem prioridade.</p>}
    {point.recorded && point.incomeRecorded === 0 && <p>Há lançamentos, mas nenhum rendimento informado neste mês.</p>}
    {funds.length > 0 ? <table>
      <caption className="sr-only">Fundos de {monthShort(point.key)}</caption>
      <thead><tr><th scope="col">Fundo</th><th scope="col" className="num">Cotas no histórico</th><th scope="col" className="num">Rendimento</th></tr></thead>
      <tbody>{funds.map((fund) => <tr key={fund.id}>
        <th scope="row">{fund.ticker}</th>
        <td className="num">{quantity(fund.held)}</td>
        <td className={fund.income === null ? 'ov-income-missing' : 'num ov-income-known'}>
          {fund.income === null ? fund.status === 'date-required' ? 'Informe a data da compra' : 'Sem registro' : <>{money(fund.income)}{(fund.status === 'api' || fund.computedRecordCount > 0) && <span className="ov-income-origin">{fund.status === 'api' ? 'oficial · automático' : 'calculado'}</span>}</>}
        </td>
      </tr>)}</tbody>
    </table> : <p>Não há cotas acumuladas nem rendimento informado no histórico desse mês.</p>}
    {hasCarriedShares && <p>Sem novas compras: as cotas anteriores continuam no saldo.</p>}
    {apiFunds.map((fund) => <div className="ov-income-events" key={fund.id}>
      <strong>{fund.ticker} · provento oficial</strong>
      {fund.apiEvents?.map((event) => <p key={`${event.source}-${event.id}`}>
        {quantity(event.eligibleShares ?? '0')} cotas com direito × {quantity(event.amountPerShare)} R$/cota = {money(event.amount ?? '0')}.<br />
        Data-com {dateBR(event.recordDate)} · pagamento {dateBR(event.paymentDate)}.<br />
        Fonte: {event.source}. <PublicSourceLink href={event.sourceUrl}>Ver documento ou fonte</PublicSourceLink>
      </p>)}
    </div>)}
    {funds.some((fund) => fund.computedRecordCount > 0) && <p>Calculado com as cotas acumuladas e o rendimento por cota informado para este mês.</p>}
    {point.missingFunds.length > 0 && <p>Falta o rendimento do mês para: {point.missingFunds.join(', ')}. Não repetimos automaticamente o pagamento do mês anterior.</p>}
    {funds.some((fund) => fund.status === 'date-required') && <p>Informe o dia da compra no histórico para conferir se as novas cotas já tinham direito ao provento. Os meses seguintes continuam usando essas cotas.</p>}
    {hasZero && <p>R$ 0 informado no lançamento é diferente de falta de registro.</p>}
  </div>;
}

/** Observed payouts, with separate gaps and per-fund historical holdings. */
export default function MonthlyIncomeChart({ points, until }: { points: MonthlyIncomePoint[]; until: string }) {
  const hatchId = `ov-hatch-${useId().replace(/[^a-zA-Z0-9_-]/g, '')}`;
  const partial = points.length - 1;
  const highlight = points.length - 2;
  const slot = 100 / points.length;
  const barWidth = slot * 0.62;
  const left = (index: number) => slot * index + (slot - barWidth) / 2;
  const center = (index: number) => slot * (index + 0.5);
  const pct = (value: number) => `${value.toFixed(3)}%`;
  const max = Math.max(0, ...points.map((point) => point.value));
  const barTop = (value: number) => max > 0 ? plotBottom - value / max * (plotBottom - plotTop) : plotBottom;
  const closed = points.slice(0, partial).filter((point) => point.recorded || point.expectedFunds.length > 0);
  // Gaps and incomplete fund coverage must not become zero months in a mean.
  const average = closed.length >= 2 && closed.every((point) => point.incomeRecorded > 0 && point.missingFunds.length === 0)
    ? closed.reduce((sum, point) => sum + point.value, 0) / closed.length : null;

  return <>
    <div className="ov-income-scroll">
      <div className="ov-chart">
        <svg width="100%" height={chartHeight} aria-hidden="true">
          <defs><pattern id={hatchId} width="5" height="5" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
            <rect className="hatch-bg" width="5" height="5" /><line className="hatch-line" x1="1" y1="0" x2="1" y2="5" />
          </pattern></defs>
          {average !== null && <line className="avg" x1="0" x2="100%" y1={barTop(average)} y2={barTop(average)} />}
          <line className="base" x1="0" x2="100%" y1={plotBottom + 0.5} y2={plotBottom + 0.5} />
          {points.map((point, index) => {
            const isPartial = index === partial;
            const isHighlight = index === highlight;
            const height = point.value > 0 ? Math.max(1, plotBottom - barTop(point.value)) : 0;
            const january = point.key.endsWith('-01');
            const missingLabel = !point.recorded ? 'registros' : 'rendimento';
            return <g key={point.key}>
              {height > 0 && <rect className={isPartial ? 'bar-partial' : isHighlight ? 'bar bar-current' : 'bar'} x={pct(left(index))} y={plotBottom - height} width={pct(barWidth)} height={height} fill={isPartial ? `url(#${hatchId})` : undefined} />}
              {point.incomeRecorded > 0
                ? <text className={`cap${height > 0 ? '' : ' cap-zero'}${isHighlight ? ' cap-current' : ''}`} x={pct(center(index))} y={plotBottom - height - 7} textAnchor="middle">{amount(point.total)}</text>
                : <text className="cap-missing" x={pct(center(index))} y={plotBottom - 23} textAnchor="middle"><tspan x={pct(center(index))}>Sem</tspan><tspan x={pct(center(index))} dy="14">{missingLabel}</tspan></text>}
              <text className={`month${isHighlight ? ' month-current' : ''}`} x={pct(center(index))} y={168} textAnchor="middle">{monthShort(point.key).split('/')[0]}</text>
              <text className={`coverage${point.missingFunds.length > 0 ? ' coverage-missing' : ''}`} x={pct(center(index))} y={185} textAnchor="middle">{point.expectedFunds.length > 0 ? `${point.incomeRecorded}/${point.expectedFunds.length} ${point.expectedFunds.length === 1 ? 'fundo' : 'fundos'}` : '—'}</text>
              {(index === 0 || january) && <text className="year" x={pct(left(index))} y={205}>{point.key.slice(0, 4)}</text>}
              {index > 0 && january && <line className="sep" x1={pct(slot * index)} x2={pct(slot * index)} y1={154} y2={209} />}
            </g>;
          })}
        </svg>
        <div className="ov-income-targets" style={{ gridTemplateColumns: `repeat(${points.length}, minmax(0, 1fr))` }}>
          {points.map((point, index) => <div className="ov-income-month" data-month={point.key} key={point.key}>
            <Def text={<MonthExplanation point={point} partial={index === partial} until={until} />}>
              <span className="sr-only">{monthShort(point.key)} · {monthStatus(point)} · {point.incomeRecorded} de {point.expectedFunds.length} fundos com rendimento registrado{index === partial ? ` · Parcial até ${until}` : ''}</span>
            </Def>
          </div>)}
        </div>
      </div>
    </div>
    <p className="ov-income-hint">Passe o mouse, toque ou foque um mês para ver os fundos e as cotas acumuladas. <span>{points.some((point) => point.apiFundCount) ? 'Inclui proventos oficiais calculados pelas cotas com direito. Valores informados por você têm prioridade.' : 'O total soma apenas os rendimentos registrados; as lacunas serão preenchidas quando houver proventos oficiais disponíveis.'}</span></p>
    {average !== null && <p className="ov-income-average">Média dos valores registrados em {closed.length} meses fechados: <span className="num">{money(average)}</span>.</p>}
    <div className="sr-only"><table>
      <caption>Proventos por mês</caption>
      <thead><tr><th scope="col">Mês</th><th scope="col">Proventos</th><th scope="col">Fundos com rendimento registrado</th><th scope="col">Fundos sem rendimento registrado</th><th scope="col">Cotas acumuladas no histórico</th></tr></thead>
      <tbody>{points.map((point, index) => <tr key={point.key}><td>{monthShort(point.key)}{index === partial ? ` (parcial, até ${until})` : ''}</td><td>{monthStatus(point)}</td><td>{point.incomeFunds.join(', ') || 'Nenhum'}</td><td>{point.missingFunds.join(', ') || 'Nenhum entre os fundos com cotas no histórico'}</td><td>{point.funds.filter((fund) => fund.status !== 'not-held' || fund.recordCount > 0).map((fund) => `${fund.ticker}: ${quantity(fund.held)}`).join('; ') || 'Sem cotas no histórico'}</td></tr>)}</tbody>
    </table></div>
  </>;
}

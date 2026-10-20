import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { LoaderCircle } from 'lucide-react';
import { amount, call, decimal, desktop, errorMessage, money, monthShort, quantity } from '../lib/bridge';
import type { ProjectionPoint, Snapshot } from '../lib/bridge';
import { Def, Empty, Field, handleForm, SectionHeading } from '../components';

/** Premissas usadas no último cálculo, só para rotular o resultado. */
interface Scenario { fund: string; months: number; monthlyCotas: number; reinvest: boolean; window: number }

const DISCLAIMER = 'A simulação usa a média dos últimos rendimentos positivos e o preço médio histórico para reinvestir, com premissas constantes. Rendimentos, preços e liquidez podem mudar. Os valores são estimativas.';

/** Acima disto a tabela abre com uma linha por ano. */
const LONG_TABLE = 36;

export default function Projections({ walletId, snapshot }: { walletId: string; snapshot: Snapshot }) {
  const [fundId, setFundId] = useState('');
  const [months, setMonths] = useState('24');
  const [monthlyCotas, setMonthlyCotas] = useState('0');
  const [windowSize, setWindowSize] = useState('6');
  const [reinvest, setReinvest] = useState(true);
  const [points, setPoints] = useState<ProjectionPoint[]>([]);
  const [scenario, setScenario] = useState<Scenario | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [stale, setStale] = useState(false);
  const [monthlyRows, setMonthlyRows] = useState(false);
  const resultRef = useRef<HTMLHeadingElement>(null);
  const recalcRef = useRef<HTMLButtonElement>(null);
  /** O cálculo partiu do "Recalcular": devolve o foco quando ele termina. */
  const restoreFocus = useRef(false);
  const update = (action: () => void) => { action(); if (points.length) setStale(true); };
  const calculate = async () => {
    restoreFocus.current = document.activeElement instanceof HTMLElement && document.activeElement.closest('.projection-stale') !== null;
    setBusy(true); setError('');
    try {
      const input = { fundId: fundId || undefined, months: Number(months), monthlyCotas: decimal(monthlyCotas), window: Number(windowSize), reinvest };
      const result = await call<ProjectionPoint[]>('project_income', { walletId, input });
      setPoints(result);
      setScenario({ fund: snapshot.funds.find((fund) => fund.id === fundId)?.ticker ?? 'Carteira completa', months: input.months, monthlyCotas: Number(input.monthlyCotas) || 0, reinvest, window: input.window });
      setStale(false);
    }
    catch (err) { setError(errorMessage(err)); } finally { setBusy(false); }
  };
  // Com sucesso a faixa "Premissas alteradas" some com o botão focado; o foco vai para o título do resultado.
  // Com erro a faixa continua e o foco volta ao "Recalcular".
  useEffect(() => {
    if (busy || !restoreFocus.current) return;
    restoreFocus.current = false;
    (recalcRef.current ?? resultRef.current)?.focus();
  }, [busy]);
  const blocked = busy || !desktop || !snapshot.funds.length;
  const selected = snapshot.funds.find((fund) => fund.id === fundId);
  const startFunds = selected ? [selected] : snapshot.funds;
  const lastEntry = startFunds.flatMap((fund) => fund.entries.map((entry) => entry.month)).sort().at(-1);
  const first = points[0];
  const final = points[points.length - 1];
  const finalMonth = final ? monthShort(final.month) : '';
  const scenarioText = scenario && [scenario.fund, `${scenario.months} meses`, scenario.monthlyCotas > 0 ? `${quantity(scenario.monthlyCotas)} ${scenario.monthlyCotas === 1 ? 'cota' : 'cotas'}/mês` : 'sem aportes', scenario.reinvest ? 'com reinvestimento' : 'sem reinvestimento'].join(' · ');
  const showCombined = scenario?.reinvest ?? true;
  const longTable = points.length > LONG_TABLE;
  const annual = longTable && !monthlyRows;
  const rows = annual ? points.filter((point, index) => point.month.endsWith('-12') || index === points.length - 1) : points;
  const tableNote = [annual ? (final.month.endsWith('-12') ? 'Dezembro de cada ano' : `Dezembro de cada ano e ${finalMonth}`) : '', stale ? 'cálculo anterior' : ''].filter(Boolean).join(' · ');
  const startIncomeTitle = selected ? 'Cotas atuais × média dos 6 últimos rendimentos por cota do fundo' : 'Soma, por fundo, das cotas atuais × média dos 6 últimos rendimentos por cota';

  return <div className={`page projection-page${stale && final ? ' is-stale' : ''}`}>
    <section className="group" aria-labelledby="projection-simulation">
      <div className="group-head"><h2 id="projection-simulation">Simulação de renda</h2>{lastEntry && <p>Proventos lançados até {monthShort(lastEntry)}</p>}</div>
      <div className={`modular projection-panel${final ? ' has-result' : ''}`}>
        <section className="block span-4 projection-controls">
          <SectionHeading title="O que simular" />
          <form id="projection-form" className="block-body projection-form" onSubmit={handleForm(calculate)}>
            <Field label="Fundo"><select value={fundId} onChange={(event) => update(() => setFundId(event.target.value))}><option value="">Carteira completa</option>{snapshot.funds.map((fund) => <option key={fund.id} value={fund.id}>{fund.ticker}</option>)}</select></Field>
            <Field label="Horizonte"><select value={months} onChange={(event) => update(() => setMonths(event.target.value))}>{[6, 12, 24, 36, 60, 120].map((value) => <option key={value} value={value}>{value} meses{value >= 12 ? ` · ${value / 12} ${value === 12 ? 'ano' : 'anos'}` : ''}</option>)}</select></Field>
            <Field label={fundId ? 'Cotas compradas por mês' : 'Cotas por mês, em cada fundo'} hint="Use 0 para simular sem aportes."><input className="num" inputMode="decimal" value={monthlyCotas} onChange={(event) => update(() => setMonthlyCotas(event.target.value))} required /></Field>
            <details className="extra-details projection-options"><summary>Ajustar cálculo</summary><div>
            <Field label="Rendimento por cota" hint="Define quantos pagamentos anteriores entram na média usada para estimar a renda."><select value={windowSize} onChange={(event) => update(() => setWindowSize(event.target.value))}>{[3, 6, 12].map((value) => <option key={value} value={value}>Média dos últimos {value} proventos</option>)}</select></Field>
            <label className="checkbox-field"><input type="checkbox" checked={reinvest} onChange={(event) => update(() => setReinvest(event.target.checked))} /><span>Reinvestir os proventos</span></label><span className="figure-note"><Def text="Usa os proventos simulados para comprar cotas inteiras pelo preço médio histórico. A cotação futura pode ser diferente.">Como funciona o reinvestimento</Def></span></div></details>
            <button className="button primary full-width" type="submit" disabled={blocked}>{busy && <LoaderCircle className="spin" size={16} />}{busy ? 'Calculando…' : 'Calcular projeção'}</button>
            {error && <p className="form-error" role="alert">{error}</p>}
          </form>
          {final && snapshot.funds.length > 0 && <details className="extra-details projection-start"><summary>Ver ponto de partida{selected ? ` · ${selected.ticker}` : ''}</summary><div>
            <dl className="facts">
              <div><dt>Cotas hoje</dt><dd>{quantity(selected ? selected.totalCotas : snapshot.totals.cotas)}</dd></div>
              <div><dt><Def text={startIncomeTitle}>Renda estimada hoje</Def></dt><dd>{money(selected ? selected.estimatedMonthlyIncome : snapshot.totals.estimatedMonthlyIncome)}</dd></div>
              {selected && <div><dt><Def text="Preço usado para reinvestir os proventos">Preço médio</Def></dt><dd>{money(selected.averagePrice)}</dd></div>}
            </dl>
          </div></details>}
        </section>

        <section className="block span-8 projection-summary" aria-labelledby="projection-result">
          <div className="block-head"><h2 id="projection-result" ref={resultRef} tabIndex={-1}><Def text={DISCLAIMER}>Resultado estimado</Def></h2>{final && <span className="block-meta">{scenarioText}</span>}</div>
          {stale && final && <div className="alert warning projection-stale" role="status"><span>Premissas alteradas. Os números abaixo são do cálculo anterior.</span><button ref={recalcRef} className="button small" type="submit" form="projection-form" disabled={blocked}>Recalcular</button></div>}
          {final ? <div className="projection-figures">
            <div className="projection-hero">
              <span className="figure-label"><Def text={`Cotas ao fim de ${finalMonth} × rendimento médio por cota${showCombined ? ', já com o reinvestimento do mês' : ''}`}>Renda mensal estimada</Def></span>
              <div className="figure-hero"><span className="unit">R$</span>{amount(final.combinedIncome)}</div>
              <span className="figure-note">{finalMonth} · {showCombined ? 'com reinvestimento' : 'sem reinvestimento'}</span>
            </div>
            <dl className="projection-readouts">
              {showCombined && <div><dt><Def text={`Proventos pagos em ${finalMonth} pelas cotas que você já tinha, antes do reinvestimento`}>Recebida em {finalMonth}</Def></dt><dd className="figure-value"><span className="unit">R$</span>{amount(final.projectedIncome)}</dd></div>}
              <div><dt><Def text={`Soma da renda recebida de ${monthShort(first.month)} a ${finalMonth}`}>Proventos acumulados</Def></dt><dd className="figure-value"><span className="unit">R$</span>{amount(final.cumulativeIncome)}</dd></div>
            </dl>
          </div> : <>
            <Empty title={snapshot.funds.length ? 'Nenhuma projeção calculada' : 'Sem fundos nesta carteira'}>{snapshot.funds.length ? 'Escolha as premissas e clique em Calcular projeção.' : 'Adicione fundos e lançamentos à carteira para simular a renda.'}</Empty>
            {startFunds.length > 0 && <details className="extra-details projection-origin"><summary id="projection-origin">Ver ponto de partida</summary><div>
              <div className="table-wrap">
                <table aria-labelledby="projection-origin">
                  <thead><tr><th>Fundo</th><th className="num">Cotas hoje</th><th className="num"><Def text="Preço usado para reinvestir os proventos">Preço médio</Def></th><th className="num"><Def text={startIncomeTitle}>Renda estimada hoje</Def></th></tr></thead>
                  <tbody>{startFunds.map((fund) => <tr key={fund.id}><td><span className="ticker">{fund.ticker}</span></td><td className="num">{quantity(fund.totalCotas)}</td><td className="num">{money(fund.averagePrice)}</td><td className="num">{money(fund.estimatedMonthlyIncome)}</td></tr>)}</tbody>
                  {!selected && startFunds.length > 1 && <tfoot><tr><td>Total</td><td className="num">{quantity(snapshot.totals.cotas)}</td><td /><td className="num">{money(snapshot.totals.estimatedMonthlyIncome)}</td></tr></tfoot>}
                </table>
              </div>
            </div></details>}
          </>}
          <p className="projection-estimate-note"><Def text={DISCLAIMER}>Simulação, sem garantia de rendimento</Def></p>
        </section>

        {final && <section className="block span-8 projection-chart-block">
          <SectionHeading title="Renda recebida por mês" meta="Valores em R$" />
          <IncomeChart points={points} scenario={scenario} />
          <p className="projection-estimate-note"><Def text={DISCLAIMER}>Como a renda é estimada</Def></p>
        </section>}
      </div>
    </section>

    {points.length > 0 && <details className="extra-details projection-monthly-details"><summary>Ver resultado mês a mês</summary><section className="group" aria-labelledby="projection-monthly">
      <div className="group-head">
        <h2 id="projection-monthly">{annual ? 'Ano a ano' : 'Mês a mês'}</h2>
        {tableNote && <p>{tableNote.charAt(0).toUpperCase() + tableNote.slice(1)}</p>}
        {longTable && <div className="group-actions"><div className="segmented" role="group" aria-label="Linhas da tabela"><button type="button" aria-pressed={annual} onClick={() => setMonthlyRows(false)}>Anual</button><button type="button" aria-pressed={!annual} onClick={() => setMonthlyRows(true)}>Mensal</button></div></div>}
      </div>
      <div className="block">
        <dl className="facts projection-extra-totals"><div><dt>Cotas ao fim</dt><dd>{quantity(final.combinedCotas)}</dd></div>{showCombined && <div><dt><Def text="Cotas compradas com os proventos; já incluídas no total ao fim.">Cotas reinvestidas</Def></dt><dd>{quantity(final.reinvestedCotas)}</dd></div>}</dl>
        <div className="table-wrap projection-table">
          <table>
            <caption className="sr-only">{annual ? 'Projeção ano a ano, em dezembro de cada ano e no último mês' : 'Projeção mês a mês'}</caption>
            <thead><tr>
              <th>Mês</th>
              <th className="num"><Def text="Ao fim do mês, já com o reinvestimento">Cotas ao fim</Def></th>
              {showCombined && <th className="num"><Def text="Total comprado com proventos até o mês">Cotas reinvestidas</Def></th>}
              <th className="num">Renda recebida</th>
              {showCombined && <th className="num"><Def text="Cotas ao fim do mês × rendimento médio por cota">Renda após reinvestir</Def></th>}
              <th className="num">Proventos acumulados</th>
            </tr></thead>
            <tbody>{rows.map((point, index) => <tr key={point.month} className={!annual && index > 0 && point.month.endsWith('-01') ? 'year-start' : undefined}>
              <td><span className="num projection-month">{monthShort(point.month)}</span></td>
              <td className="num">{quantity(point.combinedCotas)}</td>
              {showCombined && <td className="num">{quantity(point.reinvestedCotas)}</td>}
              <td className="num">{money(point.projectedIncome)}</td>
              {showCombined && <td className="num">{money(point.combinedIncome)}</td>}
              <td className="num">{money(point.cumulativeIncome)}</td>
            </tr>)}</tbody>
          </table>
        </div>
      </div>
    </section></details>}
  </div>;
}

/** Passo "redondo" para a escala do eixo vertical. */
function niceStep(max: number, count: number) {
  const raw = max / count;
  const power = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].find((factor) => factor * power >= raw) ?? 10;
  return step * power;
}

/** Meses com rótulo no eixo: todos, trimestres, semestres ou viradas de ano, conforme o espaço.
 *  O primeiro e o último sempre aparecem; os do meio só entram se não encostarem em outro rótulo. */
function monthTicks(points: ProjectionPoint[], x: (index: number) => number, room: number) {
  const n = points.length;
  const text = 44; const space = 10;
  const fits = Math.max(2, Math.floor(room / (text + space)));
  const every = n <= fits ? null : n / fits <= 3 ? [1, 4, 7, 10] : n / fits <= 6 ? [1, 7] : n / fits <= 12 ? [1] : [];
  const kept = [0];
  let edge = x(0) + text + space;
  const limit = x(n - 1) - text - space;
  points.forEach((point, index) => {
    if (index === 0 || index === n - 1) return;
    const [year, month] = point.month.split('-').map(Number);
    const wanted = every === null || (every.length ? every.includes(month) : month === 1 && year % 2 === 0);
    if (wanted && x(index) - text / 2 >= edge && x(index) + text / 2 <= limit) { kept.push(index); edge = x(index) + text / 2 + space; }
  });
  return n > 1 ? [...kept, n - 1] : kept;
}

function IncomeChart({ points, scenario }: { points: ProjectionPoint[]; scenario: Scenario | null }) {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = () => setWidth(Math.floor(element.clientWidth));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const incomes = points.map((point) => Number(point.projectedIncome));
  const label = `Projeção de renda de ${money(incomes[0])} para ${money(incomes[incomes.length - 1])}, de ${monthShort(points[0].month)} a ${monthShort(points[points.length - 1].month)}`;
  // Sem aportes, a linha do primeiro mês é exatamente a renda sem reinvestir.
  const reference = scenario && scenario.monthlyCotas === 0 && scenario.reinvest ? 'Sem reinvestir' : monthShort(points[0].month);
  return <div className="projection-chart" role="img" aria-label={label}><div ref={ref} className="projection-chart-canvas">{width > 0 && <ChartSvg points={points} incomes={incomes} width={width} reference={reference} />}</div></div>;
}

function ChartSvg({ points, incomes, width, reference }: { points: ProjectionPoint[]; incomes: number[]; width: number; reference: string }) {
  const [hover, setHover] = useState<number | null>(null);
  const height = width < 520 ? 208 : 248;
  const step = niceStep(Math.max(...incomes, 1), 5);
  const top = Math.ceil(Math.max(...incomes, step) / step) * step;
  const ticks = Array.from({ length: Math.round(top / step) + 1 }, (_, index) => index * step);
  const digits = Number.isInteger(step) ? 0 : Number.isInteger(Math.round(step * 1000) / 100) ? 1 : 2;
  const tickText = (value: number) => amount(value, digits);
  const left = 16 + Math.max(...ticks.map((value) => tickText(value).length)) * 7.4;
  const right = 12; const plotTop = 24; const bottom = height - 30;
  const plotWidth = Math.max(width - left - right, 1);
  const n = points.length;
  const x = (index: number) => left + (n > 1 ? index / (n - 1) * plotWidth : plotWidth / 2);
  const y = (value: number) => bottom - value / top * (bottom - plotTop);
  const line = incomes.map((value, index) => `${x(index).toFixed(1)},${y(value).toFixed(1)}`).join(' ');
  const start = incomes[0]; const end = incomes[n - 1];
  const band = n > 1 ? plotWidth / (n - 1) : plotWidth;
  const labelled = monthTicks(points, x, plotWidth);
  const startY = y(incomes[0]).toFixed(1);
  const growth = `M${x(0).toFixed(1)},${startY} L${line.split(' ').join(' L')} L${x(n - 1).toFixed(1)},${startY} Z`;
  // O ponto em foco é o mês sob o mouse; sem mouse, o último mês.
  const focus = hover ?? n - 1;
  const focusX = x(focus); const focusY = y(incomes[focus]);
  const focusText = hover === null || hover === n - 1 ? amount(incomes[focus]) : `${monthShort(points[focus].month)} · ${amount(incomes[focus])}`;
  const focusAnchor = focus === n - 1 && n > 1 ? 'end' : focusX - left < 56 ? 'start' : width - right - focusX < 56 ? 'end' : 'middle';
  const focusLabelX = focusAnchor === 'end' ? focusX - (focus === n - 1 ? 8 : 0) : focusX;
  return <svg viewBox={`0 0 ${width} ${height}`} width={width} height={height} aria-hidden="true" onMouseLeave={() => setHover(null)}>
    <g className="hits">{points.map((point, index) => { const from = Math.max(left, x(index) - band / 2); const to = Math.min(width - right, x(index) + band / 2); return <rect key={point.month} className="hit" x={from} y={plotTop - 8} width={Math.max(to - from, 1)} height={bottom - plotTop + 8} onMouseEnter={() => setHover(index)}><title>{`${monthShort(point.month)} · recebida ${money(point.projectedIncome)} · após reinvestir ${money(point.combinedIncome)} · ${quantity(point.combinedCotas)} cotas`}</title></rect>; })}</g>
    {ticks.map((value) => <g key={value}>
      {value > 0 && <line className="grid-line" x1={left} x2={width - right} y1={Math.round(y(value)) + 0.5} y2={Math.round(y(value)) + 0.5} />}
      <text className="tick-label" x={left - 10} y={y(value) + 4} textAnchor="end">{tickText(value)}</text>
    </g>)}
    <line className="axis" x1={left} x2={width - right} y1={Math.round(bottom) + 0.5} y2={Math.round(bottom) + 0.5} />
    {labelled.map((index) => <g key={points[index].month}>
      <line className="axis" x1={Math.round(x(index)) + 0.5} x2={Math.round(x(index)) + 0.5} y1={bottom} y2={bottom + 4} />
      <text className="tick-label" x={x(index)} y={bottom + 18} textAnchor={index === 0 && n > 1 ? 'start' : index === n - 1 && n > 1 ? 'end' : 'middle'}>{monthShort(points[index].month)}</text>
    </g>)}
    {hover !== null && hover !== n - 1 && <line className="hover-guide" x1={Math.round(focusX) + 0.5} x2={Math.round(focusX) + 0.5} y1={plotTop - 8} y2={bottom} />}
    {end !== start && <path className="series-area" d={growth} />}
    {end !== start && <g className="reference">
      <line x1={left} x2={width - right} y1={Math.round(y(start)) + 0.5} y2={Math.round(y(start)) + 0.5} />
      <text className="reference-label" x={width - right} y={end > start && y(start) + 16 <= bottom - 8 ? y(start) + 16 : y(start) - 8} textAnchor="end">{`${reference} · ${amount(start)}`}</text>
    </g>}
    <polyline className="series" points={line} />
    <circle className="end-dot" cx={x(n - 1)} cy={y(end)} r={3.5} />
    {hover !== null && hover !== n - 1 && <circle className="end-dot" cx={focusX} cy={focusY} r={3} />}
    <text className="end-label" x={focusLabelX} y={focusY - 10 < 12 ? focusY + 18 : focusY - 10} textAnchor={focusAnchor}>{focusText}</text>
  </svg>;
}

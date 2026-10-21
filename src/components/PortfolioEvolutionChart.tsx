import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { dateBR, money } from '../lib/bridge';
import type { PortfolioEvolution, PortfolioEvolutionPoint } from '../lib/bridge';

interface Props { data?: PortfolioEvolution | null; loading?: boolean }

const value = (amount: string | null | undefined) => {
  if (amount == null || amount.trim() === '') return null;
  const parsed = Number(amount);
  return Number.isFinite(parsed) ? parsed : null;
};
const signed = (amount: string | null | undefined, missing = 'Sem dados') => {
  const parsed = value(amount);
  return parsed == null ? missing : `${parsed > 0 ? '+' : parsed < 0 ? '−' : ''}${money(Math.abs(parsed))}`;
};
const resultClass = (amount: string | null | undefined) => {
  const parsed = value(amount);
  return parsed == null || parsed === 0 ? '' : parsed > 0 ? 'value-positive' : 'value-negative';
};
const monthLabel = (month: string, long = false) => {
  const parsed = /^(\d{4})-(\d{2})$/.exec(month);
  if (!parsed) return month;
  const date = new Date(Number(parsed[1]), Number(parsed[2]) - 1, 1);
  return new Intl.DateTimeFormat('pt-BR', { month: long ? 'long' : 'short', year: long ? 'numeric' : '2-digit' }).format(date).replace('. de ', '/').replace(' de ', '/').replace('.', '');
};
const partialMonth = (point: PortfolioEvolutionPoint) => {
  if (typeof point.isPartialMonth === 'boolean') return point.isPartialMonth;
  if (!/^\d{4}-\d{2}-\d{2}$/.test(point.asOf) || !point.asOf.startsWith(point.month)) return false;
  const [year, month, day] = point.asOf.split('-').map(Number);
  return day < new Date(year, month, 0).getDate();
};
const comparison = (point: PortfolioEvolutionPoint) => {
  if (value(point.portfolioResult) == null || value(point.cdiResult) == null || value(point.difference) == null) return 'Faltam dados para comparar este mês.';
  const difference = value(point.difference)!;
  return difference > 0 ? 'Carteira acima do CDI.' : difference < 0 ? 'Carteira abaixo do CDI.' : 'Carteira no mesmo nível do CDI.';
};

/** Draws native monthly results without filling unknown history or calculating returns in the browser. */
export default function PortfolioEvolutionChart({ data, loading = false }: Props) {
  const chartId = useId();
  const viewport = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(620);
  const [range, setRange] = useState<'12' | 'all'>('12');
  const [selectedMonth, setSelectedMonth] = useState<string | null>(null);
  const [showTooltip, setShowTooltip] = useState(false);
  const [showLatestTooltip, setShowLatestTooltip] = useState(false);
  const points = range === '12' ? (data?.points ?? []).slice(-12) : data?.points ?? [];
  const matchedIndex = selectedMonth == null ? -1 : points.findIndex((point) => point.month === selectedMonth);
  const selectedIndex = Math.max(0, matchedIndex < 0 ? points.length - 1 : matchedIndex);
  const selected = points[selectedIndex];
  const warning = data?.issues.find((issue) => issue !== selected?.issues[0]);
  const knownCount = points.filter((point) => value(point.portfolioResult) != null).length;
  const hasPortfolio = points.some((point) => value(point.portfolioResult) != null);
  const hasCdi = points.some((point) => value(point.cdiResult) != null);
  const current = data?.latestKnown;
  const latest = current && value(current.portfolioResult) != null && current.components.length > 0
    && !(current.fromDate === current.toDate && (data?.points ?? []).some((point) => point.asOf === current.toDate && value(point.portfolioResult) === value(current.portfolioResult))) ? current : null;
  const latestAmount = value(latest?.portfolioResult);
  const latestIndex = latest == null ? -1 : points.findIndex((point) => point.month === latest.toDate.slice(0, 7));
  const hasLatestMarker = latest != null && (points.length === 0 || latestIndex >= 0);
  const latestDatesDiffer = latest != null && latest.fromDate !== latest.toDate;
  const latestDescription = latest == null ? '' : `Carteira · último resultado conhecido: ${signed(latest.portfolioResult)}. ${latest.components.map((component) => `${component.name}: saldo de ${dateBR(component.asOf)}`).join('. ')}. ${latestDatesDiffer ? 'Os saldos têm datas diferentes; este ponto não é um registro mensal nem uma comparação com o CDI.' : 'Este ponto mostra o último resultado conhecido, separado do histórico mensal.'}`;
  const selectedLatestGap = latest != null && selected?.month === latest.toDate.slice(0, 7) && value(selected.portfolioResult) == null;
  const selectedPortfolioLabel = selectedLatestGap ? 'Registro mensal da carteira' : 'Carteira';
  const selectedPortfolioMissing = selectedLatestGap ? 'Ainda não disponível' : 'Sem saldo registrado';
  useEffect(() => {
    const element = viewport.current;
    if (!element) return;
    const update = () => setWidth(Math.max(180, element.getBoundingClientRect().width));
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const numeric = [...points.flatMap((point) => [value(point.portfolioResult), value(point.cdiResult)]), hasLatestMarker ? latestAmount : null].filter((amount): amount is number => amount != null);
  const bottom = 218;
  const top = 18;
  const left = Math.min(76, width * 0.25);
  const right = width - 16;
  const minimum = Math.min(0, ...numeric);
  const maximum = Math.max(0, ...numeric);
  const span = maximum - minimum || 1;
  const y = (amount: number) => top + (maximum + span * 0.08 - amount) / (span * 1.16) * (bottom - top);
  const x = (index: number) => points.length < 2 ? (left + right) / 2 : left + index * (right - left) / (points.length - 1);
  const latestX = latestIndex < 0 ? (points.length ? right : (left + right) / 2) : x(latestIndex);
  const path = (key: 'portfolioResult' | 'cdiResult') => {
    let connected = false;
    return points.map((point, index) => {
      const amount = value(point[key]);
      if (amount == null) { connected = false; return ''; }
      const command = connected ? 'L' : 'M';
      connected = true;
      return `${command}${x(index).toFixed(2)},${y(amount).toFixed(2)}`;
    }).join(' ');
  };
  const axis = (amount: number) => new Intl.NumberFormat('pt-BR', { style: 'currency', currency: 'BRL', maximumFractionDigits: 0, notation: Math.abs(amount) >= 10_000 ? 'compact' : 'standard' }).format(amount);
  const ticks: number[] = [];
  for (const candidate of new Set([0, minimum, maximum])) {
    if (ticks.every((tick) => Math.abs(y(candidate) - y(tick)) >= 22)) ticks.push(candidate);
  }
  const labels = new Set(points.length ? [0, points.length - 1] : []);
  let previousLabel = 0;
  for (let index = 1; index < points.length - 1; index += 1) {
    if (x(index) - x(previousLabel) >= 62 && x(points.length - 1) - x(index) >= 62) { labels.add(index); previousLabel = index; }
  }
  const select = (index: number) => { if (points[index]) setSelectedMonth(points[index].month); };
  const keyDown = (event: KeyboardEvent<SVGSVGElement>) => {
    const next = event.key === 'ArrowRight' ? Math.min(points.length - 1, selectedIndex + 1) : event.key === 'ArrowLeft' ? Math.max(0, selectedIndex - 1) : event.key === 'Home' ? 0 : event.key === 'End' ? points.length - 1 : null;
    if (next == null) return;
    event.preventDefault(); select(next); setShowTooltip(true); setShowLatestTooltip(false);
  };
  const changeRange = (next: '12' | 'all') => { setRange(next); setSelectedMonth(null); setShowTooltip(false); setShowLatestTooltip(false); };
  return <div className="portfolio-evolution" aria-busy={loading || undefined}>
    <div className="evolution-toolbar">
      <p>Ganho acumulado em reais</p>
      {(data?.points.length ?? 0) > 12 && <div className="evolution-range" role="group" aria-label="Período do gráfico"><button type="button" aria-pressed={range === '12'} onClick={() => changeRange('12')}>12 meses</button><button type="button" aria-pressed={range === 'all'} onClick={() => changeRange('all')}>Todo o período</button></div>}
    </div>
    {latest && <div className="evolution-latest-summary" role="status">
      <p><strong>Carteira · último resultado conhecido</strong><span className={`num ${resultClass(latest.portfolioResult)}`}>{signed(latest.portfolioResult)}</span></p>
      <ul aria-label="Datas dos saldos do último resultado">{latest.components.map((component, index) => <li key={`${component.kind}-${component.id ?? index}`}><span>{component.name}</span> · <time dateTime={component.asOf}>{dateBR(component.asOf)}</time></li>)}</ul>
      <p>{!hasPortfolio && 'Ainda não há histórico mensal neste período. '}{hasLatestMarker ? hasPortfolio ? 'O ponto verde mostra o resultado conhecido, separado da linha mensal.' : 'O ponto verde mostra o resultado conhecido com as datas dos saldos disponíveis.' : 'O último resultado conhecido é de um mês fora do período exibido.'}{latestDatesDiffer && ' Os saldos têm datas diferentes; o resultado conhecido não é comparado ao CDI.'}</p>
    </div>}
    <ul className="evolution-legend" aria-label="Linhas do gráfico"><li><span className={`evolution-swatch ${!hasPortfolio && hasLatestMarker ? 'latest' : 'portfolio'}`} aria-hidden="true" />{!hasPortfolio && hasLatestMarker ? 'Carteira · último resultado' : !hasPortfolio && latest ? 'Carteira · sem registro mensal' : 'Carteira'}</li><li><span className="evolution-swatch cdi" aria-hidden="true" />100% do CDI</li>{hasPortfolio && hasLatestMarker && <li><span className="evolution-swatch latest" aria-hidden="true" />Último resultado conhecido</li>}</ul>
    <div className="evolution-viewport" ref={viewport}>
      {(points.length > 0 && (hasPortfolio || hasCdi)) || (hasLatestMarker && latestAmount != null) ? <>
        <svg width="100%" height="258" viewBox={`0 0 ${width} 258`} role="img" tabIndex={0} aria-labelledby={`${chartId}-title ${chartId}-description`} aria-describedby={selected ? `${chartId}-selected` : undefined} onKeyDown={keyDown} onFocus={() => { setShowTooltip(true); setShowLatestTooltip(false); }} onBlur={() => { setShowTooltip(false); setShowLatestTooltip(false); }} onPointerLeave={() => { setShowTooltip(false); setShowLatestTooltip(false); }}>
          <title id={`${chartId}-title`}>Evolução mensal dos ganhos da carteira inteira e de 100% do CDI</title>
          <desc id={`${chartId}-description`}>Os registros mensais mostram o ganho acumulado desde o início dos investimentos. Aportes e resgates usam as mesmas datas no CDI. Meses sem dados interrompem a linha. O losango verde, quando presente, mostra os últimos valores conhecidos, com as datas de cada saldo; ele não completa o histórico mensal. Use as setas para consultar cada mês, Home para o primeiro e End para o último.</desc>
          {ticks.map((tick) => <g key={tick} aria-hidden="true"><line className={`evolution-grid ${tick === 0 ? 'zero' : ''}`} x1={left} x2={right} y1={y(tick)} y2={y(tick)} /><text className="evolution-axis" x={left - 9} y={y(tick) + 4} textAnchor="end">{axis(tick)}</text></g>)}
          {selected && <line className="evolution-guide" x1={x(selectedIndex)} x2={x(selectedIndex)} y1={top} y2={bottom} aria-hidden="true" />}
          <path className="evolution-line cdi" d={path('cdiResult')} data-series="cdi" fill="none" />
          <path className="evolution-line portfolio" d={path('portfolioResult')} data-series="portfolio" fill="none" />
          {points.map((point, index) => {
            const portfolio = value(point.portfolioResult);
            const cdi = value(point.cdiResult);
            const selectedPoint = index === selectedIndex;
            const below = value(point.difference) != null && value(point.difference)! < 0;
            const hitLeft = index === 0 ? left - 10 : (x(index - 1) + x(index)) / 2;
            const hitRight = index === points.length - 1 ? right + 10 : (x(index) + x(index + 1)) / 2;
            return <g key={point.month} data-month={point.month} className={selectedPoint ? 'is-selected' : undefined} onPointerEnter={() => { select(index); setShowTooltip(true); setShowLatestTooltip(false); }} onClick={() => { select(index); setShowTooltip(true); setShowLatestTooltip(false); }}>
              {cdi != null && <circle className="evolution-dot cdi" cx={x(index)} cy={y(cdi)} r={selectedPoint ? 4 : 2.5} />}
              {portfolio != null && <circle className={`evolution-dot portfolio ${below ? 'is-below' : ''}`} cx={x(index)} cy={y(portfolio)} r={selectedPoint ? 4 : 2.5} />}
              {portfolio == null && <line className="evolution-missing" x1={x(index)} x2={x(index)} y1={bottom - 3} y2={bottom + 3} />}
              {labels.has(index) && <text className="evolution-month" x={x(index)} y="242" textAnchor={index === 0 ? 'start' : index === points.length - 1 ? 'end' : 'middle'}>{monthLabel(point.month)}</text>}
              <rect className="evolution-hit" x={hitLeft} y={top} width={Math.max(1, hitRight - hitLeft)} height={bottom - top + 34}><title>{monthLabel(point.month, true)}. Carteira: {signed(point.portfolioResult, 'Sem saldo registrado')}. CDI: {signed(point.cdiResult)}. {comparison(point)}{point.issues.length > 0 ? ` ${point.issues.join(' ')}` : ''}</title></rect>
            </g>;
          })}
          {latest && hasLatestMarker && latestAmount != null && <g className="evolution-latest-marker" data-latest-month={latest.toDate.slice(0, 7)} tabIndex={0} role="img" aria-label={latestDescription} onPointerEnter={() => { setShowTooltip(false); setShowLatestTooltip(true); }} onClick={() => { setShowTooltip(false); setShowLatestTooltip(true); }} onFocus={(event) => { event.stopPropagation(); setShowTooltip(false); setShowLatestTooltip(true); }} onBlur={() => setShowLatestTooltip(false)}>
            <title>{latestDescription}</title>
            <circle className="evolution-latest-hit" cx={latestX} cy={y(latestAmount)} r="17" />
            <path className="evolution-latest-diamond" d={`M${latestX},${y(latestAmount) - 6} l6,6 l-6,6 l-6,-6 Z`} />
            <text className={`evolution-latest-value ${resultClass(latest.portfolioResult)}`} x={latestX > width / 2 ? latestX - 10 : latestX + 10} y={y(latestAmount) - 11} textAnchor={latestX > width / 2 ? 'end' : 'start'}>{signed(latest.portfolioResult)}</text>
            {latestIndex < 0 && <text className="evolution-month" x={latestX} y="242" textAnchor="middle">Últimos valores</text>}
          </g>}
        </svg>
        {showTooltip && selected && <div className="evolution-tooltip" role="tooltip" style={{ left: `clamp(0px, ${x(selectedIndex) / width * 100}% - 112px, 100% - 224px)` }}>
          <strong>{monthLabel(selected.month, true)}{partialMonth(selected) ? ' · parcial' : ''}</strong><span>Até {dateBR(selected.asOf)}</span>
          <dl><div><dt>{selectedPortfolioLabel}</dt><dd className={resultClass(selected.portfolioResult)}>{signed(selected.portfolioResult, selectedPortfolioMissing)}</dd></div><div><dt>100% do CDI</dt><dd>{signed(selected.cdiResult)}</dd></div><div><dt>Diferença</dt><dd className={resultClass(selected.difference)}>{signed(selected.difference, 'Não comparável')}</dd></div></dl>
          <p className={resultClass(selected.difference)}>{comparison(selected)}</p>
          {selected.issues.length > 0 && <p className="evolution-pending">{selected.issues[0]}</p>}
        </div>}
        {showLatestTooltip && hasLatestMarker && latest && <div className="evolution-tooltip evolution-latest-tooltip" role="tooltip" style={{ left: `clamp(0px, ${latestX / width * 100}% - 112px, 100% - 224px)` }}>
          <strong>Carteira · último resultado conhecido</strong>
          <span className={`num ${resultClass(latest.portfolioResult)}`}>{signed(latest.portfolioResult)}</span>
          <ul>{latest.components.map((component, index) => <li key={`${component.kind}-${component.id ?? index}`}>{component.name} · {dateBR(component.asOf)}</li>)}</ul>
          <p>{latestDatesDiffer ? 'Os saldos têm datas diferentes. Este ponto não é um registro mensal nem uma comparação com o CDI.' : 'Último resultado conhecido, separado do histórico mensal.'}</p>
        </div>}
      </> : <p className="evolution-empty" role="status">{loading ? 'Lendo a evolução da carteira…' : 'Ainda não há dados mensais suficientes para desenhar a evolução. O histórico aparece conforme os saldos e as cotações ficam disponíveis.'}</p>}
    </div>
    {selected && <div className="evolution-selected" id={`${chartId}-selected`} role="status" aria-live="polite" aria-atomic="true">
      <p><strong>{monthLabel(selected.month, true)}</strong>{partialMonth(selected) && <span> · parcial até {dateBR(selected.asOf)}</span>}{selected.estimated && <span> · CDI com datas aproximadas</span>}</p>
      <dl><div><dt>{selectedPortfolioLabel}</dt><dd className={resultClass(selected.portfolioResult)}>{signed(selected.portfolioResult, selectedPortfolioMissing)}</dd></div><div><dt>100% do CDI</dt><dd>{signed(selected.cdiResult)}</dd></div><div><dt>Diferença</dt><dd className={resultClass(selected.difference)}>{signed(selected.difference, 'Não comparável')}</dd></div></dl>
      {selectedLatestGap && latest && <p>Último resultado conhecido: <strong className={resultClass(latest.portfolioResult)}>{signed(latest.portfolioResult)}</strong>. O ponto verde usa as datas dos saldos acima; o registro mensal ainda não está completo.</p>}
      <p className={resultClass(selected.difference)}>{comparison(selected)}</p>
      {selected.issues.length > 0 && <p className="evolution-pending">{selected.issues[0]}</p>}
    </div>}
    {points.length > 0 && <p className="evolution-hint">Passe o mouse ou use as setas no gráfico para consultar um mês. <span>{knownCount} de {points.length} meses com registro da carteira.{knownCount < points.length && ' Lacunas indicam saldos ausentes; os valores não são estimados a partir do saldo atual.'}</span></p>}
    {warning && <p className="evolution-warning">{warning}</p>}
    {points.length > 0 && <details className="extra-details evolution-table"><summary>Ver valores mês a mês</summary><div className="table-wrap"><table><caption className="sr-only">Ganhos acumulados da carteira inteira e do CDI por mês. Sem saldo registrado indica que falta uma observação real da carteira.</caption><thead><tr><th scope="col">Mês / data</th><th scope="col" className="num">Carteira</th><th scope="col" className="num">100% CDI</th><th scope="col" className="num">Diferença</th></tr></thead><tbody>{points.map((point) => <tr key={point.month}><th scope="row">{monthLabel(point.month)}{partialMonth(point) && <span className="evolution-table-note">Parcial</span>}<time className="evolution-table-note" dateTime={point.asOf}>{dateBR(point.asOf)}</time>{point.issues[0] && <span className="evolution-table-note evolution-pending">{point.issues[0]}</span>}</th><td className={`num ${resultClass(point.portfolioResult)}`}>{signed(point.portfolioResult, 'Sem saldo registrado')}</td><td className="num">{signed(point.cdiResult)}</td><td className={`num ${resultClass(point.difference)}`}>{signed(point.difference, 'Não comparável')}</td></tr>)}</tbody></table></div></details>}
  </div>;
}

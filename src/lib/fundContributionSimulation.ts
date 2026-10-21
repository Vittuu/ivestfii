import type { ToolEvent } from './bridge';

type DecimalValue = string | null;
interface QuoteReference {
  symbol: string;
  status: string;
  observation: { source: string; quotedAt: string | null; fetchedAt: string } | null;
}
export interface FundContributionRow {
  fundId: string | null;
  symbol: string;
  name: string | null;
  declaredSector: string | null;
  classificationKnown: boolean;
  budget: string;
  quote: QuoteReference | null;
  quoteStatus: string;
  quoteDate: string | null;
  price: DecimalValue;
  currentShares: DecimalValue;
  sharesToBuy: DecimalValue;
  afterShares: DecimalValue;
  spentAmount: DecimalValue;
  remainder: DecimalValue;
  currentValue: DecimalValue;
  afterValue: DecimalValue;
  beforeFiiPercent: DecimalValue;
  afterFiiPercent: DecimalValue;
  beforePortfolioPercent: DecimalValue;
  afterPortfolioPercent: DecimalValue;
  issues: string[];
}
export interface ContributionPortfolioRow {
  kind: 'fiis' | 'fixedIncome' | 'cashPockets' | 'unallocatedCash';
  name: string;
  currentValue: DecimalValue;
  contribution: DecimalValue;
  afterValue: DecimalValue;
  beforePercent: DecimalValue;
  afterPercent: DecimalValue;
}
export interface FundContributionReport {
  status: 'calculated' | 'incomplete';
  budget: string;
  spentAmount: DecimalValue;
  remainder: DecimalValue;
  fiiBasisComplete: boolean;
  rows: FundContributionRow[];
  portfolioAllocation: {
    status: 'calculated' | 'incomplete';
    totalBefore: DecimalValue;
    totalAfter: DecimalValue;
    unallocatedCash: DecimalValue;
    rows: ContributionPortfolioRow[];
    asOf?: string | null;
    fromDate?: string | null;
    toDate?: string | null;
    valuationBasis?: 'same-date' | 'mixed-dates' | 'undated';
    issues?: string[];
  };
  issues: string[];
  assumptions: string[];
}
export interface FundContributionResult {
  wallet: { id: string; name: string };
  budget: string;
  report: FundContributionReport;
}

const record = (value: unknown): value is Record<string, unknown> => Boolean(value) && typeof value === 'object' && !Array.isArray(value);
const text = (value: unknown, max = 300): value is string => typeof value === 'string' && value.length > 0 && value.length <= max;
const nullableText = (value: unknown) => value === null || text(value);
const nullableLabel = (value: unknown) => value === null || typeof value === 'string' && value.length <= 300;
const decimal = (value: unknown): value is string => typeof value === 'string' && /^\d{1,29}(?:\.\d{1,28})?$/.test(value);
const nullableDecimal = (value: unknown) => value === null || decimal(value);
const percent = (value: unknown) => value === null || decimal(value) && Number(value) <= 100;
const strings = (value: unknown): value is string[] => Array.isArray(value) && value.length <= 2048 && value.every(item => text(item, 4000));
const status = (value: unknown) => value === 'calculated' || value === 'incomplete';
const date = (value: unknown): value is string => typeof value === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(value) && Number.isFinite(Date.parse(value + 'T12:00:00Z')) && new Date(value + 'T12:00:00Z').toISOString().slice(0, 10) === value;
const nullableDate = (value: unknown) => value === null || date(value);

function quote(value: unknown): boolean {
  if (value === null) return true;
  if (!record(value) || !text(value.symbol, 32) || !text(value.status, 100)) return false;
  const observation = value.observation;
  return observation === null || record(observation) && text(observation.source) && (observation.quotedAt === null || text(observation.quotedAt, 100)) && text(observation.fetchedAt, 100);
}

function fundRow(value: unknown): boolean {
  if (!record(value) || !nullableText(value.fundId) || !text(value.symbol, 32) || !nullableLabel(value.name) || !nullableLabel(value.declaredSector) || typeof value.classificationKnown !== 'boolean' || !decimal(value.budget) || !quote(value.quote) || !text(value.quoteStatus, 100) || !nullableDate(value.quoteDate) || !strings(value.issues)) return false;
  if (!['price', 'currentShares', 'sharesToBuy', 'afterShares', 'spentAmount', 'remainder', 'currentValue', 'afterValue'].every(key => nullableDecimal(value[key]))) return false;
  if (value.price !== null && /^0(?:\.0+)?$/.test(value.price as string)) return false;
  if (value.sharesToBuy !== null && !/^\d{1,29}$/.test(value.sharesToBuy as string)) return false;
  return ['beforeFiiPercent', 'afterFiiPercent', 'beforePortfolioPercent', 'afterPortfolioPercent'].every(key => percent(value[key]));
}

const classKinds = ['fiis', 'fixedIncome', 'cashPockets', 'unallocatedCash'];
function portfolioRow(value: unknown): boolean {
  return record(value) && classKinds.includes(value.kind as string) && text(value.name) && ['currentValue', 'contribution', 'afterValue'].every(key => nullableDecimal(value[key])) && percent(value.beforePercent) && percent(value.afterPercent);
}

/** Accept only explicit MCP envelopes, never provider prose or truncated previews. */
function payload(value: unknown, depth = 0): Record<string, unknown> | null {
  if (depth > 4) return null;
  if (typeof value === 'string') {
    if (value.length > 8_000_000) return null;
    try { return payload(JSON.parse(value), depth + 1); } catch { return null; }
  }
  if (Array.isArray(value)) {
    return value.length === 1 && record(value[0]) && value[0].type === 'text' ? payload(value[0].text, depth + 1) : null;
  }
  if (!record(value) || value.isError === true) return null;
  if ('structuredContent' in value) return payload(value.structuredContent, depth + 1);
  if (value.status === 'simulation' && value.kind === 'contribution-allocation') return value;
  return 'content' in value ? payload(value.content, depth + 1) : null;
}

/** Results remain tied to their recorded wallet and quoted prices after navigation. */
export function readFundContributionSimulation(tool: ToolEvent): FundContributionResult | null {
  if (tool.status !== 'completed' || tool.toolName.split(/__|\//).at(-1) !== 'simular_aporte') return null;
  const data = payload(tool.output);
  if (!data || !record(data.wallet) || !text(data.wallet.id) || !text(data.wallet.name) || !record(data.simulation) || !decimal(data.simulation.budget) || !record(data.fundSimulation)) return null;
  const fund = data.fundSimulation;
  if (!status(fund.status) || !decimal(fund.budget) || !nullableDecimal(fund.spentAmount) || !nullableDecimal(fund.remainder) || typeof fund.fiiBasisComplete !== 'boolean' || !strings(fund.issues) || !strings(fund.assumptions) || !Array.isArray(fund.rows) || fund.rows.length > 500 || !fund.rows.every(fundRow)) return null;
  const keys = fund.rows.map(row => (row as Record<string, unknown>).fundId || (row as Record<string, unknown>).symbol);
  if (new Set(keys).size !== keys.length || !record(fund.portfolioAllocation)) return null;
  const allocation = fund.portfolioAllocation;
  if (!status(allocation.status) || !['totalBefore', 'totalAfter', 'unallocatedCash'].every(key => nullableDecimal(allocation[key])) || !Array.isArray(allocation.rows) || allocation.rows.length !== 4 || !allocation.rows.every(portfolioRow) || new Set(allocation.rows.map(row => (row as Record<string, unknown>).kind)).size !== 4) return null;
  if (allocation.issues !== undefined && !strings(allocation.issues) || ['asOf', 'fromDate', 'toDate'].some(key => allocation[key] !== undefined && !nullableDate(allocation[key])) || allocation.valuationBasis !== undefined && !['same-date', 'mixed-dates', 'undated'].includes(allocation.valuationBasis as string)) return null;
  return { wallet: data.wallet as unknown as FundContributionResult['wallet'], budget: data.simulation.budget, report: fund as unknown as FundContributionReport };
}

/** Presentation only: retain every decimal place supplied by Rust. */
export function simulationNumber(value: DecimalValue, minimumFraction = 0): string {
  if (value === null) return '—';
  const [whole, fraction = ''] = value.split('.');
  const formatted = whole.replace(/\B(?=(\d{3})+(?!\d))/g, '.');
  const decimals = fraction.padEnd(minimumFraction, '0');
  return formatted + (decimals ? ',' + decimals : '');
}

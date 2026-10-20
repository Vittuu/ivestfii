import type { Fund, MonthlyIncomeEvent, MonthlyIncomeReport } from './bridge';
import { ledgerRows } from './ledger.ts';

export type MonthlyFundIncomeStatus = 'recorded' | 'computed' | 'api' | 'date-required' | 'missing' | 'not-held';

export interface MonthlyFundIncome {
  id: string;
  ticker: string;
  /** Final quantity known from purchases through this month, never today's total. */
  held: string;
  /** Null means no income evidence for this month; an explicit zero stays "0". */
  income: string | null;
  /** Mixed explicit/computed income is recorded; the counts retain both origins. */
  status: MonthlyFundIncomeStatus;
  recordCount: number;
  purchaseCount: number;
  incomeRecordCount: number;
  recordedRecordCount: number;
  computedRecordCount: number;
  apiEvents?: MonthlyIncomeEvent[];
  reasons?: string[];
}

export interface MonthlyIncomePoint {
  key: string;
  /** Conversion for chart geometry only. Use total to format the exact amount. */
  value: number;
  total: string;
  /** Any launch, including purchases/information without income evidence. */
  recorded: boolean;
  recordCount: number;
  purchaseCount: number;
  /** Number of funds with income evidence, including explicit zero totals. */
  incomeRecorded: number;
  incomeFunds: string[];
  /** Known holdings in the month, plus any explicit payout without known holdings. */
  expectedFunds: string[];
  missingFunds: string[];
  funds: MonthlyFundIncome[];
  apiFundCount?: number;
}

/** Native calculation uses real cutoff/payment dates. Manual records retain priority. */
export function applyMonthlyIncomeReport(points: MonthlyIncomePoint[], report: MonthlyIncomeReport): MonthlyIncomePoint[] {
  return points.map((point) => {
    const month = report.months.find((item) => item.month === point.key);
    if (!month) return point;
    const funds = point.funds.map((fund): MonthlyFundIncome => {
      const item = month.funds.find((value) => value.fundId === fund.id);
      if (!item) return fund;
      // A cache-only response without API coverage must retain the local ledger presentation.
      if (item.status === 'recorded') return { ...fund, income: item.amount, held: item.holdings };
      return {
        ...fund,
        held: item.holdings,
        income: item.status === 'not_held' ? null : item.amount,
        status: item.status === 'not_held' ? 'not-held' : item.status === 'date_required' ? 'date-required' : item.status,
        apiEvents: item.events,
        reasons: item.reasons,
      };
    });
    const incomeFunds = funds.filter((fund) => fund.income !== null).map((fund) => fund.ticker);
    const total = sumExact(funds.flatMap((fund) => fund.income === null ? [] : [fund.income]));
    return {
      ...point, total, value: Number(total), funds,
      incomeRecorded: incomeFunds.length, incomeFunds,
      expectedFunds: funds.filter((fund) => fund.status !== 'not-held').map((fund) => fund.ticker),
      missingFunds: funds.filter((fund) => fund.status === 'missing' || fund.status === 'date-required').map((fund) => fund.ticker),
      apiFundCount: funds.filter((fund) => fund.status === 'api').length,
    };
  });
}

/** Sum values already validated/calculated by ledgerRows without Number rounding. */
function sumExact(values: string[]): string {
  const scale = values.reduce((highest, value) => Math.max(highest, value.split('.')[1]?.length ?? 0), 0);
  const total = values.reduce((sum, value) => {
    const [whole, fraction = ''] = value.split('.');
    return sum + BigInt(whole + fraction.padEnd(scale, '0'));
  }, 0n);
  if (!scale) return total.toString();
  const digits = total.toString().padStart(scale + 1, '0');
  return `${digits.slice(0, -scale)}.${digits.slice(-scale)}`.replace(/\.?0+$/, '') || '0';
}

const positive = (value: string) => /[1-9]/.test(value);

/**
 * Observed income only: prior purchases carry forward, but a prior per-share
 * dividend never creates a payment in an unrecorded month. All income arithmetic
 * and same-month holdings come from ledgerRows, matching portfolio-core.
 *
 * "Expected" identifies funds to review from recorded holdings; it does not
 * assert that a fund paid that month or that the import contains every holding.
 * Requested keys keep their input order, including repetitions.
 */
export function buildMonthlyIncome(funds: Fund[], monthKeys: string[]): MonthlyIncomePoint[] {
  for (const key of monthKeys) {
    if (!/^\d{4}-(?:0[1-9]|1[0-2])$/.test(key)) throw new RangeError('Mês inválido no gráfico de rendimentos.');
  }
  const histories = funds.map((fund) => ({
    fund,
    rows: ledgerRows(fund).map((row) => ({ ...row, month: row.entry.month.slice(0, 7) }))
      .sort((left, right) => left.month.localeCompare(right.month)),
  }));

  return monthKeys.map((key): MonthlyIncomePoint => {
    const details = histories.map(({ fund, rows }): MonthlyFundIncome => {
      const beforeOrInMonth = rows.filter((row) => row.month <= key);
      const held = beforeOrInMonth.at(-1)?.held ?? '0';
      const inMonth = rows.filter((row) => row.month === key);
      const knownHoldings = positive(held);
      // A per-share rate with no known holdings cannot establish a payment.
      // An explicit total is still evidence even if purchase history is absent.
      const incomeRows = inMonth.filter((row) => !row.computed || (knownHoldings && positive(row.entry.dividendPerCota)));
      const recordedRecordCount = incomeRows.filter((row) => !row.computed).length;
      const computedRecordCount = incomeRows.length - recordedRecordCount;
      return {
        id: fund.id,
        ticker: fund.ticker,
        held,
        income: incomeRows.length ? sumExact(incomeRows.map((row) => row.dividend)) : null,
        status: recordedRecordCount ? 'recorded' : computedRecordCount ? 'computed' : knownHoldings ? 'missing' : 'not-held',
        recordCount: inMonth.length,
        purchaseCount: inMonth.filter((row) => row.kind === 'purchase' || row.kind === 'both').length,
        incomeRecordCount: incomeRows.length,
        recordedRecordCount,
        computedRecordCount,
      };
    });
    const incomeFunds = details.filter((fund) => fund.income !== null).map((fund) => fund.ticker);
    const expectedFunds = details.filter((fund) => fund.status !== 'not-held').map((fund) => fund.ticker);
    const total = sumExact(details.flatMap((fund) => fund.income === null ? [] : [fund.income]));
    const recordCount = details.reduce((sum, fund) => sum + fund.recordCount, 0);
    return {
      key,
      value: Number(total),
      total,
      recorded: recordCount > 0,
      recordCount,
      purchaseCount: details.reduce((sum, fund) => sum + fund.purchaseCount, 0),
      incomeRecorded: incomeFunds.length,
      incomeFunds,
      expectedFunds,
      missingFunds: details.filter((fund) => fund.status === 'missing').map((fund) => fund.ticker),
      funds: details,
    };
  });
}

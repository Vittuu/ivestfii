import { useEffect, useState } from 'react';
import { desktop } from './bridge';
import type { MonthlyIncomeReport, Snapshot } from './bridge';
import { readPublicCache, usePublicData } from './publicData';
import { applyMonthlyIncomeReport, buildMonthlyIncome } from './monthlyIncome';

/** Reads the local B3 cache only. Network refresh is shared by the app, never by a tooltip. */
export function useMonthlyIncome(walletId: string, snapshot: Snapshot, monthKeys: string[]) {
  const publicData = usePublicData();
  const input = JSON.stringify([walletId, snapshot.funds.map((fund) => [fund.id, fund.ticker, fund.entries]), monthKeys]);
  const [result, setResult] = useState<{ input: string; report: MonthlyIncomeReport } | null>(null);
  useEffect(() => {
    if (!desktop || !walletId || monthKeys.length === 0) return;
    let cancelled = false;
    const sorted = [...monthKeys].sort();
    void readPublicCache<MonthlyIncomeReport>('get_monthly_income', { walletId, startMonth: sorted[0], endMonth: sorted.at(-1) })
      .then((report) => { if (!cancelled && report?.months) setResult({ input, report }); })
      .catch(() => { if (!cancelled) setResult(null); });
    return () => { cancelled = true; };
  }, [input, walletId, publicData.revision, publicData.dividends]);
  const points = buildMonthlyIncome(snapshot.funds, monthKeys);
  return result?.input === input ? applyMonthlyIncomeReport(points, result.report) : points;
}

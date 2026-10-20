/** Synthetic monthly-income regressions; Node 24 strips TypeScript types. */
import assert from 'node:assert/strict';
import test from 'node:test';
import { buildMonthlyIncome } from '../src/lib/monthlyIncome.ts';

const entry = (id, month, fields = {}) => ({ id, month, cotasAdded: '0', pricePerCota: '0', dividendPerCota: '0', dividendTotal: null, notes: '', ...fields });
const fund = (entries, ticker = 'TEST11') => ({ id: ticker, ticker, name: 'Fundo sintético', sector: 'Teste', entries, totalCotas: '999', totalInvested: '0', averagePrice: '0', dividendsReceived: '0', estimatedMonthlyIncome: '999' });

test('old purchases carry forward into a later payout without another purchase', () => {
  const [month] = buildMonthlyIncome([fund([
    entry('buy', '2023-03', { cotasAdded: '10', pricePerCota: '9' }),
    entry('income', '2026-07', { dividendPerCota: '0.11' }),
  ])], ['2026-07']);
  assert.equal(month.total, '1.1');
  assert.equal(month.value, 1.1);
  assert.equal(month.incomeRecorded, 1);
  assert.deepEqual(month.expectedFunds, ['TEST11']);
  assert.equal(month.funds[0].held, '10');
  assert.equal(month.funds[0].income, '1.1');
  assert.equal(month.funds[0].status, 'computed');
  assert.equal(month.purchaseCount, 0);
});

test('a gap has unknown income rather than an invented zero or prior-rate payout', () => {
  const points = buildMonthlyIncome([fund([
    entry('buy-income', '2026-01', { cotasAdded: '10', dividendPerCota: '0.2' }),
    entry('later-income', '2026-03', { dividendPerCota: '0.1' }),
  ])], ['2026-01', '2026-02', '2026-03']);
  const gap = points[1];
  assert.equal(gap.recorded, false);
  assert.equal(gap.recordCount, 0);
  assert.equal(gap.incomeRecorded, 0);
  assert.equal(gap.total, '0'); // Sum of known values; coverage distinguishes absence.
  assert.equal(gap.funds[0].income, null);
  assert.equal(gap.funds[0].held, '10');
  assert.equal(gap.funds[0].status, 'missing');
  assert.deepEqual(gap.missingFunds, ['TEST11']);
  assert.deepEqual(points.map((point) => point.total), ['2', '0', '1']);
});

test('explicit zero is recorded and suppresses a positive rate fallback', () => {
  const [month] = buildMonthlyIncome([fund([
    entry('buy', '2026-01', { cotasAdded: '10' }),
    entry('zero', '2026-02', { dividendPerCota: '1', dividendTotal: '0.00' }),
  ])], ['2026-02']);
  assert.equal(month.recorded, true);
  assert.equal(month.incomeRecorded, 1);
  assert.equal(month.total, '0');
  assert.equal(month.funds[0].income, '0');
  assert.equal(month.funds[0].status, 'recorded');
  assert.equal(month.funds[0].recordedRecordCount, 1);
  assert.equal(month.funds[0].computedRecordCount, 0);
  assert.deepEqual(month.missingFunds, []);
});

test('duplicate records sum every payout while using one final monthly holding', () => {
  const [month] = buildMonthlyIncome([fund([
    entry('buy', '2026-01', { cotasAdded: '10' }),
    entry('feb-rate', '2026-02', { cotasAdded: '2', dividendPerCota: '0.11' }),
    entry('feb-explicit', '2026-02', { cotasAdded: '2', dividendTotal: '0.97' }),
  ])], ['2026-02']);
  assert.equal(month.funds[0].held, '14');
  assert.equal(month.total, '2.51');
  assert.equal(month.recordCount, 2);
  assert.equal(month.purchaseCount, 2);
  assert.equal(month.incomeRecorded, 1);
  assert.equal(month.funds[0].incomeRecordCount, 2);
  assert.equal(month.funds[0].recordedRecordCount, 1);
  assert.equal(month.funds[0].computedRecordCount, 1);
});

test('funds bought in the future do not lower prior-month coverage or leak current totals', () => {
  const [month] = buildMonthlyIncome([
    fund([entry('old', '2026-01', { cotasAdded: '10' }), entry('paid', '2026-02', { dividendTotal: '2' })], 'OLD11'),
    fund([entry('later', '2026-04', { cotasAdded: '5', dividendPerCota: '0.2' })], 'LATER11'),
  ], ['2026-02']);
  assert.equal(month.incomeRecorded, 1);
  assert.deepEqual(month.expectedFunds, ['OLD11']);
  assert.deepEqual(month.missingFunds, []);
  assert.equal(month.funds[1].held, '0');
  assert.equal(month.funds[1].income, null);
  assert.equal(month.funds[1].status, 'not-held');
});

test('explicit payout is retained even when the purchase history is absent', () => {
  const [month] = buildMonthlyIncome([
    fund([entry('cash', '2026-02', { dividendTotal: '1.25' })], 'PAID11'),
    fund([entry('rate-only', '2026-02', { dividendPerCota: '0.1' })], 'RATE11'),
  ], ['2026-02']);
  assert.equal(month.total, '1.25');
  assert.deepEqual(month.incomeFunds, ['PAID11']);
  assert.deepEqual(month.expectedFunds, ['PAID11']);
  assert.equal(month.funds[0].held, '0');
  assert.equal(month.funds[0].status, 'recorded');
  assert.equal(month.funds[1].status, 'not-held');
});

test('purchase-only and information-only records are not confirmed zero payments', () => {
  const points = buildMonthlyIncome([fund([
    entry('buy', '2026-01', { cotasAdded: '10', pricePerCota: '9' }),
    entry('info', '2026-02', { pricePerCota: '9.5' }),
  ])], ['2026-01', '2026-02']);
  assert.deepEqual(points.map((point) => [point.recorded, point.purchaseCount, point.incomeRecorded]), [[true, 1, 0], [true, 0, 0]]);
  assert.ok(points.every((point) => point.funds[0].income === null && point.funds[0].status === 'missing'));
});

test('requested order and repetition are stable with no backward or future leakage', () => {
  const points = buildMonthlyIncome([fund([
    entry('second', '2026-04', { cotasAdded: '5', dividendPerCota: '0.3' }),
    entry('first', '2026-02', { cotasAdded: '10', dividendPerCota: '0.1' }),
  ])], ['2026-04', '2026-01', '2026-03', '2026-04']);
  assert.deepEqual(points.map((point) => point.funds[0].held), ['15', '0', '10', '15']);
  assert.deepEqual(points.map((point) => point.total), ['4.5', '0', '0', '4.5']);
  assert.notEqual(points[0], points[3]);
});

test('aggregation retains exact decimals and uses ledger rounding for computed payouts', () => {
  const [month] = buildMonthlyIncome([
    fund([entry('exact', '2026-02', { dividendTotal: '1.2345678901234567890123456789' })], 'EXACT11'),
    fund([entry('half', '2026-02', { cotasAdded: '0.1', dividendPerCota: '0.05' })], 'HALF11'),
    fund([entry('huge', '2026-01', { cotasAdded: '9007199254740993' })], 'HUGE11'),
  ], ['2026-02']);
  assert.equal(month.total, '1.2445678901234567890123456789');
  assert.equal(month.funds[1].income, '0.01');
  assert.equal(month.funds[2].held, '9007199254740993');
  assert.deepEqual(month.missingFunds, ['HUGE11']);
});

test('empty inputs stay empty and source data is never mutated', () => {
  const input = [fund([entry('later', '2026-03', { dividendTotal: '2' }), entry('earlier', '2026-01', { cotasAdded: '10' })])];
  const original = structuredClone(input);
  for (const item of input) {
    for (const record of item.entries) Object.freeze(record);
    Object.freeze(item.entries);
    Object.freeze(item);
  }
  Object.freeze(input);
  const keys = Object.freeze(['2026-03', '2026-02']);
  buildMonthlyIncome(input, keys);
  assert.deepEqual(input, original);
  assert.deepEqual(buildMonthlyIncome([], ['2026-02'])[0].expectedFunds, []);
  assert.deepEqual(buildMonthlyIncome(input, []), []);
  for (const key of ['2026-00', '2026-13', '2026-2', '', 'invalid']) {
    assert.throws(() => buildMonthlyIncome(input, [key]), RangeError);
  }
});

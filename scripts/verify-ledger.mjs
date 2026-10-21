/** Synthetic ledger regressions. Node 24 strips the helper's TypeScript types.
 * Optional: node scripts/verify-ledger.mjs --binary target/release/IvestFII.exe
 * The native comparison imports only generated fixtures into a new .local folder.
 */
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import test from 'node:test';
import { parseArgs } from 'node:util';
import { ledgerKindLabel, ledgerRows, recentLedgerRows } from '../src/lib/ledger.ts';

const { values } = parseArgs({ options: { binary: { type: 'string' } } });
const entry = (id, month, fields = {}) => ({ id, month, cotasAdded: '0', pricePerCota: '0', dividendPerCota: '0', dividendTotal: null, notes: 'Fixture sintética', ...fields });
const fund = (entries, ticker = 'TEST11') => ({ id: ticker, ticker, name: 'Fundo sintético', sector: 'Teste', entries, totalCotas: '0', totalInvested: '0', averagePrice: '0', dividendsReceived: '0', estimatedMonthlyIncome: '0' });
const rowsById = (fixture) => new Map(ledgerRows(fixture).map((row) => [row.entry.id, row]));
const mixed = fund([
  entry('jan-buy', '2026-01', { cotasAdded: '10', pricePerCota: '10', dividendTotal: '0' }),
  entry('feb-explicit-zero', '2026-02', { dividendPerCota: '1', dividendTotal: '0' }),
  entry('mar-income', '2026-03', { dividendPerCota: '0.1' }),
  entry('apr-buy', '2026-04', { cotasAdded: '2', pricePerCota: '10' }),
  entry('may-explicit', '2026-05', { dividendPerCota: '0.1', dividendTotal: '7.75' }),
  entry('jun-income', '2026-06', { dividendPerCota: '0.1' }),
  entry('jun-both', '2026-06', { cotasAdded: '3', pricePerCota: '10', dividendPerCota: '0.2' }),
  entry('jul-buy', '2026-07', { cotasAdded: '1', pricePerCota: '5' }),
  entry('aug-explicit', '2026-08', { dividendTotal: '3' }),
]);
const duplicates = fund([
  entry('base', '2025-01', { cotasAdded: '10', pricePerCota: '9.91', dividendPerCota: '0.1', dividendTotal: '0.97' }),
  entry('first', '2025-02', { cotasAdded: '2', pricePerCota: '10.02', dividendPerCota: '0.11' }),
  entry('second', '2025-02', { cotasAdded: '2', pricePerCota: '10.02', dividendPerCota: '0.11' }),
], 'DUPL11');
const large = fund([entry('large', '2025-01', { cotasAdded: '9007199254740993', pricePerCota: '1.01' })], 'LARG11');
const tiny = fund([entry('tiny', '2025-01', { cotasAdded: '0.5', pricePerCota: '0.02', dividendPerCota: '0.0099999999999999999999999999' })], 'TINY11');
const explicit = fund([entry('explicit', '2025-01', { dividendTotal: '1.2345678901234567890123456789' })], 'EXPL11');

// Aggregate only synthetic outputs in arbitrary precision for comparison with
// native totals; no input is converted into a floating-point number.
function sumDecimals(numbers) {
  const scale = Math.max(0, ...numbers.map((value) => value.split('.')[1]?.length ?? 0));
  const sum = numbers.reduce((total, value) => {
    const [whole, fraction = ''] = value.split('.');
    return total + BigInt(whole + fraction.padEnd(scale, '0'));
  }, 0n);
  if (!scale) return sum.toString();
  const digits = sum.toString().padStart(scale + 1, '0');
  return `${digits.slice(0, -scale)}.${digits.slice(-scale)}`.replace(/\.?0+$/, '');
}

test('mixed history keeps actual movement, final monthly holdings, and recorded zero', () => {
  const rows = rowsById(mixed);
  assert.equal(rows.get('jun-income').held, '15');
  assert.equal(rows.get('jun-income').dividend, '1.5');
  assert.equal(rows.get('jun-both').kind, 'both');
  assert.equal(rows.get('jun-both').purchaseCost, '30');
  assert.equal(rows.get('jun-both').dividend, '3');
  assert.equal(rows.get('feb-explicit-zero').kind, 'information');
  assert.equal(rows.get('feb-explicit-zero').dividend, '0');
  assert.equal(rows.get('feb-explicit-zero').computed, false);
  assert.equal(rows.get('mar-income').computed, true);
  assert.equal(rows.get('may-explicit').dividend, '7.75');
  assert.equal(sumDecimals([...rows.values()].map((row) => row.purchaseCost)), '155');
  assert.equal(sumDecimals([...rows.values()].map((row) => row.dividend)), '16.25');
  assert.equal(ledgerRows(mixed)[0].held, '16');
});

test('same-month records retain their source order and every dividend, as in Rust', () => {
  const rows = ledgerRows(duplicates);
  assert.deepEqual(rows.map((row) => row.entry.id), ['first', 'second', 'base']);
  assert.deepEqual(rows.slice(0, 2).map((row) => [row.held, row.dividend]), [['14', '1.54'], ['14', '1.54']]);
  assert.equal(sumDecimals(rows.map((row) => row.dividend)), '4.05');
  assert.equal(sumDecimals(rows.map((row) => row.purchaseCost)), '139.18');
});

test('information-only price, positive zero-cost quantity, and income without holdings stay distinct', () => {
  const rows = rowsById(fund([
    entry('price-only', '2025-01', { pricePerCota: '20' }),
    entry('no-held', '2025-01', { dividendPerCota: '10' }),
    entry('explicit-income', '2025-01', { dividendTotal: '2' }),
    entry('zero-price-buy', '2025-02', { cotasAdded: '3' }),
  ]));
  assert.equal(rows.get('price-only').kind, 'information');
  assert.equal(rows.get('price-only').purchaseCost, '0');
  assert.equal(rows.get('no-held').kind, 'information');
  assert.equal(rows.get('no-held').dividend, '0');
  assert.equal(rows.get('explicit-income').kind, 'income');
  assert.equal(rows.get('zero-price-buy').kind, 'purchase');
  assert.equal(rows.get('zero-price-buy').purchaseCost, '0');
});

test('explicit zero suppresses fallback even in a purchase with a positive per-share rate', () => {
  const [row] = ledgerRows(fund([entry('zero', '2025-01', { cotasAdded: '2', pricePerCota: '0.1', dividendPerCota: '10', dividendTotal: '0' })]));
  assert.equal(row.kind, 'purchase');
  assert.equal(row.dividend, '0');
  assert.equal(row.computed, false);
  assert.equal(ledgerRows(explicit)[0].dividend, '1.2345678901234567890123456789');
});

test('computed dividends round midpoint away from zero, including fractional holdings', () => {
  const [row] = ledgerRows(fund([entry('half-cent', '2025-01', { cotasAdded: '0.1', pricePerCota: '0.2', dividendPerCota: '0.05' })]));
  assert.equal(row.held, '0.1');
  assert.equal(row.purchaseCost, '0.02');
  assert.equal(row.dividend, '0.01');
  // rust_decimal first rescales multiplication to 28 places with half-even,
  // then money() rounds the resulting 0.005 away from zero to one cent.
  assert.equal(ledgerRows(tiny)[0].dividend, '0.01');
});

test('quantities above Number precision and decimal purchase costs stay exact', () => {
  const [row] = ledgerRows(large);
  assert.equal(row.held, '9007199254740993');
  assert.equal(row.purchaseCost, '9097271247288402.93');
  const fine = ledgerRows(fund([entry('fine', '2025-01', { cotasAdded: '3', pricePerCota: '1.234567890123456789' })]))[0];
  assert.equal(fine.purchaseCost, '3.703703670370370367');
});

test('retroactive ordering is stable and never mutates the source entries or objects', () => {
  const input = structuredClone(mixed);
  input.entries.reverse();
  const sourceOrder = input.entries.map((item) => item.id);
  for (const item of input.entries) Object.freeze(item);
  Object.freeze(input.entries);
  const rows = ledgerRows(input);
  assert.deepEqual(input.entries.map((item) => item.id), sourceOrder);
  assert.equal(rows.find((row) => row.entry.id === 'jun-income').held, '15');
  assert.deepEqual(rows.filter((row) => row.entry.month === '2026-06').map((row) => row.entry.id), ['jun-both', 'jun-income']);
  assert.ok(rows.every((row) => input.entries.includes(row.entry)));
  assert.deepEqual(ledgerRows(fund([])), []);
});

test('labels describe the four movements and malformed decimals never become invented zero', () => {
  assert.deepEqual(['purchase', 'income', 'both', 'information'].map(ledgerKindLabel), ['Compra', 'Rendimento', 'Compra e rendimento', 'Só informação']);
  for (const invalid of ['', 'NaN', '-1', '1,5', '1e2']) {
    assert.throws(() => ledgerRows(fund([entry('invalid', '2025-01', { cotasAdded: invalid })])), RangeError);
  }
});

test('recent movements show older actual purchases separately from the latest income without mutating records', () => {
  const input = structuredClone([mixed]);
  const before = JSON.stringify(input);
  for (const record of input[0].entries) Object.freeze(record);
  Object.freeze(input[0].entries);
  const recent = recentLedgerRows(input);
  assert.deepEqual(recent.purchases.map((row) => row.entry.id), ['jul-buy', 'jun-both', 'apr-buy', 'jan-buy']);
  assert.deepEqual(recent.income.map((row) => row.entry.id), ['aug-explicit', 'jun-both', 'jun-income', 'may-explicit']);
  assert.equal(recent.purchases.find((row) => row.entry.id === 'jun-both').entry.cotasAdded, '3');
  assert.equal(recent.purchases.find((row) => row.entry.id === 'jun-both').held, '15');
  assert.equal(recent.purchases.find((row) => row.entry.id === 'jun-both').purchaseCost, '30');
  assert.equal(recent.purchases.some((row) => row.entry.id === 'feb-explicit-zero'), false);
  assert.equal(JSON.stringify(input), before);
});

test('recent purchases retain actual dates, leave old dates unknown, and distinguish missing price from information-only rows', () => {
  const input = fund([
    entry('unknown-day', '2026-06', { cotasAdded: '2', pricePerCota: '10' }),
    entry('dated-early', '2026-06', { cotasAdded: '3', pricePerCota: '11', purchaseDate: '2026-06-04' }),
    entry('dated-late', '2026-06', { cotasAdded: '1', pricePerCota: '12', purchaseDate: '2026-06-25' }),
    entry('price-only', '2026-07', { pricePerCota: '20' }),
    entry('unknown-price', '2026-07', { cotasAdded: '1' }),
    entry('income', '2026-08', { dividendTotal: '5' }),
  ]);
  const recent = recentLedgerRows([input]);
  assert.deepEqual(recent.purchases.map((row) => row.entry.id), ['unknown-price', 'dated-late', 'dated-early', 'unknown-day']);
  assert.equal(recent.purchases.at(-1).entry.purchaseDate, undefined);
  assert.equal(recent.purchases[0].purchaseCost, '0');
  assert.equal(recent.purchases.some((row) => row.entry.id === 'price-only'), false);
  assert.deepEqual(recent.income.map((row) => row.entry.id), ['income']);
  assert.deepEqual(recentLedgerRows([]), { purchases: [], income: [] });
});

test('generated fixtures agree with the native Rust snapshot in an isolated directory', { skip: !values.binary }, () => {
  const fixtures = [mixed, duplicates, large, tiny, explicit];
  const directory = resolve('.local', `ledger-verification-${randomUUID()}`);
  mkdirSync(directory, { recursive: true });
  const source = resolve(directory, 'synthetic.json');
  const json = JSON.stringify({ fiis: fixtures.map((item) => ({ ticker: item.ticker, name: item.name, sector: item.sector, entries: item.entries.map((record) => ({ month: record.month, cotas_added: record.cotasAdded, price_per_cota: record.pricePerCota, dividend_per_cota: record.dividendPerCota, dividend_total: record.dividendTotal, notes: record.notes })) })) });
  writeFileSync(source, json, 'utf8');
  const result = JSON.parse(execFileSync(resolve(values.binary), ['--verify-storage', '--data-dir', directory, '--legacy-file', source], { encoding: 'utf8', timeout: 30000, windowsHide: true }));
  assert.equal(result.status, 'ok');
  const snapshot = JSON.parse(readFileSync(resolve(directory, 'verified-snapshot.json'), 'utf8'));
  for (const nativeFund of snapshot.funds) {
    const rows = ledgerRows(nativeFund);
    assert.equal(sumDecimals(rows.map((row) => row.purchaseCost)), nativeFund.totalInvested);
    assert.equal(sumDecimals(rows.map((row) => row.dividend)), nativeFund.dividendsReceived);
    assert.equal(rows[0].held, nativeFund.totalCotas);
  }
  assert.equal(readFileSync(source, 'utf8'), json);
});

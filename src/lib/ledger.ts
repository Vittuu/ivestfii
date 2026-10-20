import type { Fund, RecordEntry } from './bridge';

export type LedgerKind = 'purchase' | 'income' | 'both' | 'information';

export interface LedgerRow {
  entry: RecordEntry;
  kind: LedgerKind;
  /** Final quantity for the entry's month, including every entry in that month. */
  held: string;
  /** Quantity added times its recorded unit price; not rounded to cents. */
  purchaseCost: string;
  /** Explicit total, or the computed dividend rounded as in portfolio-core. */
  dividend: string;
  /** True only when dividendTotal is null; an explicit zero is never computed. */
  computed: boolean;
}

const kindLabels: Record<LedgerKind, string> = {
  purchase: 'Compra',
  income: 'Rendimento',
  both: 'Compra e rendimento',
  information: 'Só informação',
};

export function ledgerKindLabel(kind: LedgerKind): string { return kindLabels[kind]; }

interface Decimal { coefficient: bigint; scale: number }
const ZERO: Decimal = { coefficient: 0n, scale: 0 };
// rust_decimal stores a 96-bit coefficient and at most 28 decimal places.
const MAX_COEFFICIENT = (1n << 96n) - 1n;
const power = (scale: number) => 10n ** BigInt(scale);

function normalize(value: Decimal): Decimal {
  let { coefficient, scale } = value;
  if (coefficient === 0n) return ZERO;
  while (scale > 0 && coefficient % 10n === 0n) { coefficient /= 10n; scale -= 1; }
  return { coefficient, scale };
}

function decimal(value: string): Decimal {
  const match = /^(\d+)(?:\.(\d+))?$/.exec(value);
  if (!match || (match[2]?.length ?? 0) > 28) throw new RangeError('Valor decimal inválido no histórico.');
  const parsed = normalize({ coefficient: BigInt(match[1] + (match[2] ?? '')), scale: match[2]?.length ?? 0 });
  if (parsed.coefficient > MAX_COEFFICIENT) throw new RangeError('Valor fora da precisão do histórico.');
  return parsed;
}

function roundedCoefficient(coefficient: bigint, divisor: bigint, halfEven: boolean): bigint {
  const whole = coefficient / divisor;
  const remainder = coefficient % divisor;
  const twice = remainder * 2n;
  return whole + (twice > divisor || (twice === divisor && (!halfEven || whole % 2n !== 0n)) ? 1n : 0n);
}

/** Decimal arithmetic uses midpoint-to-even when a result needs rescaling. */
function fit(coefficient: bigint, scale: number): Decimal {
  for (let drop = Math.max(0, scale - 28); drop <= scale; drop += 1) {
    const reduced = drop ? roundedCoefficient(coefficient, power(drop), true) : coefficient;
    if (reduced <= MAX_COEFFICIENT) return normalize({ coefficient: reduced, scale: scale - drop });
  }
  throw new RangeError('O cálculo excede a precisão do histórico.');
}

function add(left: Decimal, right: Decimal): Decimal {
  const scale = Math.max(left.scale, right.scale);
  return fit(left.coefficient * power(scale - left.scale) + right.coefficient * power(scale - right.scale), scale);
}

function multiply(left: Decimal, right: Decimal): Decimal {
  return fit(left.coefficient * right.coefficient, left.scale + right.scale);
}

/** portfolio-core::money rounds computed dividends to cents, ties away from zero. */
function cents(value: Decimal): Decimal {
  return value.scale <= 2 ? value : normalize({ coefficient: roundedCoefficient(value.coefficient, power(value.scale - 2), false), scale: 2 });
}

function text(value: Decimal): string {
  const normalized = normalize(value);
  const digits = normalized.coefficient.toString().padStart(normalized.scale + 1, '0');
  return normalized.scale ? `${digits.slice(0, -normalized.scale)}.${digits.slice(-normalized.scale)}` : digits;
}

/**
 * Mirrors calculate_fund in portfolio-core: all additions determine the final
 * quantity for each month before any computed dividend is evaluated. Explicit
 * totals (including zero) are retained without rounding. Repeated months remain
 * separate records; ordering ties retain their source order and their IDs.
 */
export function ledgerRows(fund: Fund): LedgerRow[] {
  const entries = [...fund.entries].sort((left, right) => left.month.localeCompare(right.month));
  const heldByMonth = new Map<string, Decimal>();
  let held = ZERO;
  for (const entry of entries) {
    held = add(held, decimal(entry.cotasAdded));
    heldByMonth.set(entry.month, held);
  }
  return entries.map((entry): LedgerRow => {
    const quantity = decimal(entry.cotasAdded);
    const monthHolding = heldByMonth.get(entry.month) ?? ZERO;
    const purchaseCost = multiply(quantity, decimal(entry.pricePerCota));
    const computed = entry.dividendTotal === null;
    const dividend = computed ? cents(multiply(monthHolding, decimal(entry.dividendPerCota))) : decimal(entry.dividendTotal!);
    const purchase = quantity.coefficient > 0n;
    const income = dividend.coefficient > 0n;
    return {
      entry, kind: purchase ? (income ? 'both' : 'purchase') : (income ? 'income' : 'information'),
      held: text(monthHolding), purchaseCost: text(purchaseCost), dividend: text(dividend), computed,
    };
  }).sort((left, right) => right.entry.month.localeCompare(left.entry.month));
}

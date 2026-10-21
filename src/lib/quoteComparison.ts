import type { Fund, QuoteItem } from './bridge';

export type PriceTone = 'below' | 'near' | 'above' | 'unknown';
export interface QuoteComparison {
  tone: PriceTone;
  label: string;
  average: number | null;
  differencePercent: number | null;
  explanation: string;
}

interface Decimal { coefficient: bigint; scale: number }
const zero: Decimal = { coefficient: 0n, scale: 0 };
const maxCoefficient = 79228162514264337593543950335n;
const power = (scale: number) => 10n ** BigInt(scale);

// Match the nonnegative decimal values accepted by the ledger. Numbers are used
// only for display; the weighted cost and the inclusive ±1% bounds stay exact.
function decimal(value: string): Decimal | null {
  if (!/^\d{1,29}(?:\.\d{1,28})?$/.test(value)) return null;
  const [whole, fraction = ''] = value.split('.');
  const coefficient = BigInt(whole + fraction);
  return coefficient <= maxCoefficient ? { coefficient, scale: fraction.length } : null;
}

function add(left: Decimal, right: Decimal): Decimal {
  const scale = Math.max(left.scale, right.scale);
  return { coefficient: left.coefficient * power(scale - left.scale) + right.coefficient * power(scale - right.scale), scale };
}

function multiply(left: Decimal, right: Decimal): Decimal {
  return { coefficient: left.coefficient * right.coefficient, scale: left.scale + right.scale };
}

function compare(left: Decimal, right: Decimal): number {
  const scale = Math.max(left.scale, right.scale);
  const difference = left.coefficient * power(scale - left.scale) - right.coefficient * power(scale - right.scale);
  return difference < 0n ? -1 : difference > 0n ? 1 : 0;
}

function validDate(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const date = new Date(`${value}T12:00:00Z`);
  return Number.isFinite(date.getTime()) && date.toISOString().slice(0, 10) === value;
}

export function marketToday(): string {
  const parts = new Intl.DateTimeFormat('en-CA', { timeZone: 'America/Sao_Paulo', year: 'numeric', month: '2-digit', day: '2-digit' }).formatToParts(new Date());
  return ['year', 'month', 'day'].map((type) => parts.find((part) => part.type === type)!.value).join('-');
}

export function compareQuote(item: QuoteItem, funds: Fund[], today: string): QuoteComparison {
  const unknown = (explanation: string, average: number | null = null, label = 'Sem preço médio'): QuoteComparison => ({ tone: 'unknown', label, average, differencePercent: null, explanation });
  const matching = funds.filter((fund) => fund.ticker.trim().toUpperCase() === item.symbol.trim().toUpperCase());
  if (matching.length !== 1) return unknown('Não há uma base única de compras para este fundo na carteira selecionada.');

  let shares = zero;
  let cost = zero;
  for (const entry of matching[0].entries) {
    if (!/^\d{4}-(?:0[1-9]|1[0-2])$/.test(entry.month)) return unknown('Confira o mês das compras para calcular o preço médio.');
    if (entry.month > today.slice(0, 7)) continue;
    const quantity = decimal(entry.cotasAdded);
    if (!quantity) return unknown('Confira a quantidade das compras para calcular o preço médio.');
    if (quantity.coefficient === 0n) continue;
    if (entry.purchaseDate) {
      if (!validDate(entry.purchaseDate) || !entry.purchaseDate.startsWith(entry.month + '-')) return unknown('Confira a data das compras para calcular o preço médio.');
      if (entry.purchaseDate > today) continue;
    }
    const price = decimal(entry.pricePerCota);
    if (!price || price.coefficient === 0n) return unknown('Há compras sem preço válido. Complete esses valores para comparar a cotação.');
    shares = add(shares, quantity);
    cost = add(cost, multiply(quantity, price));
  }
  if (shares.coefficient === 0n) return unknown('Registre as compras deste fundo nesta carteira para comparar com seu preço médio.');

  const average = (Number(cost.coefficient) / 10 ** cost.scale) / (Number(shares.coefficient) / 10 ** shares.scale);
  const observation = item.observation;
  const quote = observation && observation.currency === 'BRL' && observation.symbol.trim().toUpperCase() === item.symbol.trim().toUpperCase() ? decimal(observation.price) : null;
  if (!quote || quote.coefficient === 0n) return unknown('A comparação precisa de uma cotação positiva em reais.', average, 'Sem cotação comparável');

  const quotedCost = multiply(quote, shares);
  const compared = multiply(quotedCost, { coefficient: 100n, scale: 0 });
  const lower = multiply(cost, { coefficient: 99n, scale: 0 });
  const upper = multiply(cost, { coefficient: 101n, scale: 0 });
  const tone: PriceTone = compare(compared, lower) < 0 ? 'below' : compare(compared, upper) > 0 ? 'above' : 'near';
  const scale = Math.max(quotedCost.scale, cost.scale);
  const numerator = quotedCost.coefficient * power(scale - quotedCost.scale) - cost.coefficient * power(scale - cost.scale);
  const denominator = cost.coefficient * power(scale - cost.scale);
  // Round the displayed percentage to two decimal places without changing tone.
  const absolute = numerator < 0n ? -numerator : numerator;
  const hundredths = (absolute * 10000n + denominator / 2n) / denominator;
  const differencePercent = Number(hundredths) / 100 * (numerator < 0n ? -1 : 1);
  return {
    tone,
    label: tone === 'below' ? 'Abaixo da sua média' : tone === 'above' ? 'Acima da sua média' : 'Próximo da sua média',
    average,
    differencePercent,
    explanation: 'Cotação comparada ao valor total das compras dividido pelas cotas compradas nesta carteira. Compras futuras e rendimentos não alteram esta média. A faixa amarela vai de 1% abaixo até 1% acima.',
  };
}

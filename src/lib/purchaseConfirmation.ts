import type { FundContributionResult, FundContributionRow } from './fundContributionSimulation';

export interface SimulationPurchase {
  fundId: string;
  purchaseDate: string;
  shares: string;
  pricePerShare: string;
}

export interface SimulationPurchaseInput {
  sourceToolId: string;
  purchases: SimulationPurchase[];
}

export interface PurchaseConfirmationRow extends SimulationPurchase {
  ticker: string;
  total: string;
  status: 'new' | 'already_confirmed' | 'already_saved' | 'conflict';
  recordId: string | null;
  message: string | null;
}

export interface PurchaseConfirmationReport {
  sourceToolId: string;
  previewToken: string;
  rows: PurchaseConfirmationRow[];
  canConfirm: boolean;
  createdCount: number;
  matchedCount: number;
}

export interface PurchaseDraft {
  fundId: string;
  selected: boolean;
  purchaseDate: string;
  shares: string;
  pricePerShare: string;
}

/** Local calendar date: neither the UTC day nor a quote date is a purchase date. */
export function localToday(): string {
  const now = new Date();
  return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
}

export function eligibleSimulationFunds(result: FundContributionResult): FundContributionRow[] {
  return result.report.rows.filter(row => row.fundId !== null && /^\d+(?:\.\d+)?$/.test(row.budget) && /[1-9]/.test(row.budget));
}

export function purchaseDrafts(rows: FundContributionRow[]): PurchaseDraft[] {
  return rows.map(row => ({
    fundId: row.fundId!, selected: false, purchaseDate: '',
    shares: row.sharesToBuy !== null && /^[1-9]\d*$/.test(row.sharesToBuy) ? row.sharesToBuy : '',
    pricePerShare: row.price === null ? '' : row.price.replace('.', ','),
  }));
}

function actualDate(value: string, today: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value) || value > today || value < '1900-01-01') return false;
  const timestamp = Date.parse(`${value}T12:00:00Z`);
  return Number.isFinite(timestamp) && new Date(timestamp).toISOString().slice(0, 10) === value;
}

/** Validation only. Quantity, cost and deduplication are calculated by Rust. */
export function purchasesFromDrafts(drafts: PurchaseDraft[], today = localToday()): SimulationPurchase[] {
  const selected = drafts.filter(row => row.selected);
  if (!selected.length) throw new Error('Selecione pelo menos uma compra que você realizou.');
  return selected.map(row => {
    if (!actualDate(row.purchaseDate, today)) throw new Error('Informe uma data de compra válida, até hoje, para cada fundo selecionado.');
    const shares = row.shares.trim();
    if (!/^[1-9]\d{0,28}$/.test(shares)) throw new Error('A quantidade comprada deve ser um número inteiro maior que zero.');
    const pricePerShare = row.pricePerShare.trim().replace(',', '.');
    if (!/^\d{1,27}(?:\.\d{1,2})?$/.test(pricePerShare) || !/[1-9]/.test(pricePerShare)) throw new Error('Informe o preço real por cota, maior que zero e com até duas casas decimais, sem separador de milhar.');
    return { fundId: row.fundId, purchaseDate: row.purchaseDate, shares, pricePerShare };
  });
}

export function purchaseStatusLabel(status: PurchaseConfirmationRow['status']): string {
  switch (status) {
    case 'new': return 'Novo lançamento';
    case 'already_saved': return 'Já salvo no histórico';
    case 'already_confirmed': return 'Compra registrada';
    case 'conflict': return 'Precisa conferir';
  }
}

export function purchaseStatusDescription(row: PurchaseConfirmationRow): string {
  if (row.message) return row.message;
  switch (row.status) {
    case 'new': return 'Será criado um lançamento com os dados reais informados.';
    case 'already_saved': return 'Já salvo no histórico; será vinculado a esta simulação, sem novo lançamento.';
    case 'already_confirmed': return 'Esta compra já foi confirmada; nenhum novo lançamento será criado.';
    case 'conflict': return 'Os dados diferem do registro salvo. Confira o histórico antes de continuar.';
  }
}

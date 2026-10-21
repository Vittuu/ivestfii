import { invoke, isTauri } from '@tauri-apps/api/core';

export const desktop = isTauri();

export interface RecordEntry {
  id: string;
  month: string;
  cotasAdded: string;
  pricePerCota: string;
  dividendPerCota: string;
  dividendTotal: string | null;
  purchaseDate?: string | null;
  notes: string;
}

export interface Fund {
  id: string;
  ticker: string;
  name: string;
  sector: string;
  entries: RecordEntry[];
  totalCotas: string;
  totalInvested: string;
  averagePrice: string;
  dividendsReceived: string;
  estimatedMonthlyIncome: string;
}

export interface Snapshot {
  funds: Fund[];
  totals: { invested: string; cotas: string; dividendsReceived: string; estimatedMonthlyIncome: string };
  recordCount: number;
  brokerPosition?: BrokerPosition | null;
  cashPockets?: CashPocket[];
  cashBalance?: string;
}

export interface PortfolioWallet {
  id: string;
  name: string;
  institution: string;
  isDefault: boolean;
}

export interface InvestmentProfileInput {
  riskTolerance: 'unknown' | 'conservative' | 'moderate' | 'aggressive';
  shortTermObjective: string;
  shortTermMonths: number | null;
  longTermObjective: string;
  longTermMonths: number | null;
  liquidityNeeds: string;
  emergencyReserveStatus: 'unknown' | 'not_started' | 'partial' | 'complete';
  monthlyContribution: string | null;
  knowledgeLevel: 'beginner' | 'intermediate' | 'advanced' | null;
  constraints: string;
  notes: string;
}
export interface InvestmentProfile extends InvestmentProfileInput { updatedAt: string }

export interface WealthOverview {
  wallets: { wallet: PortfolioWallet; fiiInvested: string; cashBalance: string }[];
  totals: { fiiInvested: string; cashBalance: string };
}

export interface CashFlow { date: string; kind: 'deposit' | 'withdrawal'; amount: string }

export interface CashPocket {
  id: string;
  name: string;
  institution: string;
  balance: string;
  totalDeposits?: string | null;
  totalWithdrawals?: string | null;
  balanceDate?: string | null;
  cashFlows?: CashFlow[] | null;
  cdiPercent: string;
  annualCdiRate: string | null;
  rateDate: string | null;
  rateSource?: 'manual' | 'bcb-sgs';
  rateSourceUrl?: string | null;
  rateFetchedAt?: string | null;
  notes: string;
}

export interface EconomicIndicator {
  id: 'cdi' | 'selic' | 'ipca';
  name: string;
  value: string;
  unit: string;
  referenceDate: string;
  source: string;
  sourceUrl: string;
  fetchedAt: string;
}

export interface EconomicPanel {
  indicators: EconomicIndicator[];
  errors: { id: string; message: string }[];
  fetchedAt?: string | null;
}

export interface NewsSnapshot {
  items: { id: string; title: string; url: string; publishedAt: string | null; source: string }[];
  source: string;
  sourceUrl: string;
  fetchedAt: string;
}

export interface MarketDataSettings {
  brapiKeyConfigured: boolean;
  brapiKeySource: 'local' | 'environment' | null;
  cvmMappings: Record<string, string>;
}

export type MarketDataStatus = 'mapping_required' | 'invalid_mapping' | 'not_found' | 'unsupported' | 'cached' | 'available' | 'unavailable' | 'not_configured' | 'unauthorized' | 'forbidden' | 'rate_limited';

export interface QuoteObservation {
  symbol: string;
  price: string;
  currency: string;
  quotedAt: string | null;
  fetchedAt: string;
  source: string;
  sourceUrl: string;
  delayMinutes: number | null;
}

export interface QuoteItem {
  symbol: string;
  status: MarketDataStatus;
  observation: QuoteObservation | null;
  error: string | null;
}

export interface QuotePanel {
  items: QuoteItem[];
  fetchedAt: string | null;
  credentialConfigured: boolean;
  errors: string[];
}

export interface DividendPanel {
  items: { symbol: string; status: MarketDataStatus; observation: { fetchedAt: string; source: string; sourceUrl: string; events: unknown[] } | null; error: string | null }[];
  fetchedAt: string | null;
  errors: string[];
}

export interface MonthlyIncomeEvent {
  id: string;
  symbol: string;
  amountPerShare: string;
  recordDate: string;
  paymentDate: string;
  eligibleShares: string | null;
  amount: string | null;
  source: string;
  sourceUrl: string;
}
export interface MonthlyIncomeReport {
  months: {
    month: string;
    total: string | null;
    knownTotal: string;
    status: string;
    funds: {
      fundId: string;
      symbol: string;
      status: 'recorded' | 'api' | 'missing' | 'date_required' | 'not_held';
      amount: string | null;
      holdings: string;
      eligibleShares: string | null;
      events: MonthlyIncomeEvent[];
      manualRecordIds: string[];
      reasons: string[];
    }[];
  }[];
}

export type PublicDataSourceId = 'economics' | 'news' | 'quotes' | 'cvm' | 'cdiHistory' | 'dividends' | 'historicalValuations' | 'historicalIncome';
export interface PublicDataSourceState {
  id: PublicDataSourceId;
  status: 'updated' | 'cached' | 'unavailable' | 'not_configured' | 'deferred' | 'unsupported';
  message?: string | null;
}
export interface PublicDataRefresh {
  economics: EconomicPanel | null;
  news: NewsSnapshot | null;
  quotes: QuotePanel;
  cvm: CvmPanel;
  dividends?: DividendPanel;
  sources: PublicDataSourceState[];
  checkedAt: string;
}

/** Accumulated result for the recorded FII purchases, before taxes/fees.
 * Percentages are gains over purchase cost, never annualized returns. */
export interface FiiPerformance {
  status: 'available' | 'incomplete' | 'empty';
  asOf: string | null;
  startMonth: string | null;
  invested: string;
  dividends: string;
  marketValue: string | null;
  priceResult: string | null;
  totalResult: string | null;
  totalReturnPercent: string | null;
  cdi: { endingValue: string; result: string; returnPercent: string } | null;
  difference: string | null;
  income?: FiiIncomeBreakdown | null;
  issues: string[];
  assumptions: string[];
  quoteSource: string | null;
  quoteFetchedAt: string | null;
  cdiSource: string | null;
  cdiSourceUrl: string | null;
  cdiFetchedAt: string | null;
  dividendSource?: string | null;
  dividendFetchedAt?: string | null;
  dividendSources?: { symbol: string; status: string; source: string; sourceUrl: string; fetchedAt: string; coverageWarning: string }[];
  portfolio: PortfolioPerformance;
  portfolioBenchmark?: PortfolioBenchmark | null;
  /** Last complete closed-month comparison with all investments on one shared date.
   * Separate from the latest balances and never combined with their results. */
  comparableBenchmark?: PortfolioBenchmark | null;
  portfolioEvolution?: PortfolioEvolution | null;
  historicalData?: { status: string; priceObservations: number; pricePending: number; navObservations: number; navPendingMonths: number; issues: string[] } | null;
  historicalIncome?: {
    status: string;
    eventCount: number;
    pendingDocuments: number;
    sources: { symbol: string; startDate: string; endDate: string; listingComplete: boolean; documentsTotal: number; documentsPending: number; complete: boolean; fetchedAt: string | null }[];
    issues: string[];
  } | null;
}

export interface CdiPerformance {
  endingValue: string;
  result: string;
  returnPercent: string;
}

/** Monthly accumulated gains with matching cash movements. Missing observations remain null. */
export interface PortfolioEvolutionPoint {
  month: string;
  asOf: string;
  portfolioResult: string | null;
  cdiResult: string | null;
  invested: string | null;
  difference: string | null;
  status: 'available' | 'incomplete';
  issues: string[];
  estimated?: boolean;
  isPartialMonth: boolean;
  provenance?: {
    kind: 'reconstructed';
    incomeComplete: boolean;
    components: { kind: string; id?: string | null; name: string; asOf: string; invested: string; marketValue: string; result: string; source: string; sourceUrl: string; fetchedAt: string }[];
    assumptions: string[];
  } | null;
}

export interface PortfolioEvolution {
  status: 'available' | 'partial' | 'empty';
  startDate?: string | null;
  asOf?: string | null;
  points: PortfolioEvolutionPoint[];
  /** Latest complete result, keeping each component's own balance date.
   * Separate from real monthly observations; never implies a same-date CDI comparison. */
  latestKnown?: {
    fromDate: string;
    toDate: string;
    invested: string;
    portfolioResult: string;
    components: { kind: string; id: string | null; name: string; asOf: string; result: string }[];
  } | null;
  issues: string[];
  assumptions: string[];
}

/** Compares each investment's dated movements to CDI through its own balance date. */
export interface PortfolioBenchmark {
  status: 'available' | 'incomplete' | 'empty';
  startDate: string | null;
  asOf: string | null;
  result: string | null;
  invested: string | null;
  returnPercent: string | null;
  cdi: CdiPerformance | null;
  difference: string | null;
  components: {
    kind: 'fiis' | 'fixedIncome' | 'cash';
    id?: string | null;
    name: string;
    startDate: string | null;
    asOf: string | null;
    invested: string | null;
    result: string | null;
    cdi: CdiPerformance | null;
    issues: string[];
  }[];
  issues: string[];
  assumptions: string[];
}

export interface FiiIncomeBreakdown {
  recordedAmount: string;
  apiAmount: string;
  asOf: string;
  startMonth: string | null;
  endMonth: string | null;
  status: 'complete' | 'partial';
  missingFundMonthCount: number;
  dateRequiredFundMonthCount: number;
  ignoredEventCount: number;
  byFund: { fundId: string; symbol: string; recordedAmount: string; apiAmount: string; totalAmount: string }[];
}

export interface CashPerformanceItem {
  id: string;
  name: string;
  balance: string;
  invested: string | null;
  withdrawals: string | null;
  income: string | null;
  balanceDate: string | null;
}

export interface CashPerformance {
  balance: string;
  invested: string | null;
  income: string | null;
  withdrawals: string | null;
  knownInvested: string;
  knownIncome: string;
  count: number;
  missingBasisCount: number;
  items: CashPerformanceItem[];
}

export interface FixedIncomePerformance {
  status: 'available' | 'incomplete' | 'empty';
  invested: string | null;
  balance: string;
  income: string | null;
  result: string | null;
  referenceDate: string | null;
  sourceHash: string | null;
  name: string | null;
  issues: string[];
}

export interface PortfolioPerformance {
  status: 'available' | 'incomplete' | 'empty';
  invested: string | null;
  knownInvested: string;
  income: string | null;
  knownIncome: string;
  currentValue: string | null;
  knownCurrentValue: string;
  totalResult: string | null;
  knownResult: string;
  totalReturnPercent: string | null;
  withdrawals: string | null;
  cash: CashPerformance;
  fixedIncome?: FixedIncomePerformance;
  issues: string[];
}

export interface CvmFundReport {
  cnpj: string;
  fundName: string;
  fundType?: 'FII' | 'FIAGRO';
  referenceDate: string;
  filedAt: string | null;
  netAssets: string | null;
  equityPerShare: string | null;
  shareholders: number | null;
  issuedShares: string | null;
  source: string;
  sourceUrl: string;
  fetchedAt: string;
}

export interface CvmFundItem {
  symbol: string;
  cnpj: string | null;
  status: MarketDataStatus;
  report: CvmFundReport | null;
  error: string | null;
}

export interface CvmPanel {
  items: CvmFundItem[];
  fetchedAt: string | null;
  errors: string[];
}

export interface CashSimulation {
  pocketId: string;
  startingBalance: string;
  endingBalance: string;
  grossEarnings: string;
  annualCdiRate: string;
  cdiPercent: string;
  businessDays: number;
  rateDate: string | null;
  assumptions: string[];
}

export interface BrokerHolding {
  ticker: string;
  shares: string;
  averagePrice: string;
  price: string;
  marketValue: string;
  unrealizedResult: string;
  costBasis: string;
}

export interface BrokerFixedIncomeObservation {
  referenceDate: string;
  name: string;
  category: string;
  riskLevel: string;
  balance: string;
  invested: string;
  totalEarnings: string;
  earnings12Months: string;
  return12Months: string;
  returnReferenceDate: string;
  contributions: {
    applicationDate: string;
    invested: string;
    currentValue: string;
    earnings: string;
    returnPercent: string;
  }[];
  sources: { fileName: string; sha256: string }[];
  importedAt: string;
  sourcePath: string;
  sourceHash: string;
}

export interface BrokerPosition {
  referenceDate: string;
  institution: string;
  importedAt: string;
  sourcePath: string;
  sourceHash: string;
  overview: {
    updatedTime: string | null;
    capturedTime: string | null;
    totalWithCash: string;
    invested: string;
    cash: string;
    listedFunds: string;
    fixedIncome: string;
    pendingIncome: string;
  };
  listedFunds: {
    capturedTime: string | null;
    marketValue: string;
    costBasis: string;
    unrealizedResult: string;
    dailyResult: string;
    holdings: BrokerHolding[];
  };
  fixedIncome: {
    name: string;
    nameTruncated: boolean;
    balance: string;
    netBalance: string;
    reportedEarnings: string;
    return12Months: string;
    redemptionConversion: string;
    redemptionSettlement: string;
    latestObservation?: BrokerFixedIncomeObservation | null;
  };
  warnings: string[];
}

export interface Settings { autostart: boolean; startMinimized: boolean; closeToTray: boolean; autoUpdate: boolean }
export interface UpdateStatus {
  status: 'idle' | 'disabled' | 'checking' | 'up_to_date' | 'available' | 'downloading' | 'ready' | 'waiting_for_idle' | 'installing' | 'error';
  currentVersion: string;
  availableVersion: string | null;
  downloadedBytes: number;
  totalBytes: number | null;
  lastCheckedAt: string | null;
  error: string | null;
}
export interface AppInfo {
  dataDirectory: string;
  databasePath: string;
  legacyPath: string | null;
  positionPath?: string | null;
  positionImport?: Record<string, unknown> | null;
  migration: Record<string, unknown> | null;
  sync: { status: string; pendingChanges: number };
}
export interface Conversation { id: string; title: string; createdAt: string; updatedAt: string }
export interface ChatMessage {
  id: string;
  conversationId: string;
  role: string;
  content: string;
  provider?: string | null;
  model?: string | null;
  createdAt: string;
}
export interface Provider {
  id: string;
  name: string;
  installed: boolean;
  executable: string | null;
  transport: string;
  ready?: boolean;
  version?: string | null;
  error?: string | null;
}
export interface ChatRun {
  id: string;
  walletId?: string | null;
  conversationId: string;
  provider: string;
  model: string | null;
  status: 'running' | 'completed' | 'failed' | 'cancelled';
  text: string;
  error: string | null;
  statusMessage?: string | null;
  createdAt: string;
  updatedAt: string;
}
export interface ToolEvent {
  id: string;
  conversationId: string;
  taskId: string | null;
  toolName: string;
  input: unknown;
  output: unknown | null;
  status: 'running' | 'completed' | 'failed' | 'cancelled';
  startedAt: string;
  completedAt: string | null;
}
export interface ProjectionPoint {
  month: string;
  projectedCotas: string;
  projectedIncome: string;
  cumulativeIncome: string;
  reinvestedCotas: string;
  combinedCotas: string;
  combinedIncome: string;
}

export const emptySnapshot: Snapshot = {
  funds: [],
  totals: { invested: '0', cotas: '0', dividendsReceived: '0', estimatedMonthlyIncome: '0' },
  recordCount: 0,
  brokerPosition: null,
};

export function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!desktop) return Promise.reject(new Error('Disponível no aplicativo Windows. A conexão web ainda não foi configurada.'));
  return invoke<T>(command, args);
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'Não foi possível concluir a operação. Tente novamente.';
}

export const money = (value: string | number) => new Intl.NumberFormat('pt-BR', { style: 'currency', currency: 'BRL' }).format(Number(value) || 0);
export const quantity = (value: string | number) => new Intl.NumberFormat('pt-BR', { maximumFractionDigits: 4 }).format(Number(value) || 0);
export const decimal = (value: string) => value.trim().replace(',', '.');
export const monthLabel = (value: string) => {
  const [year, month] = value.slice(0, 7).split('-');
  if (!year || !month) return value;
  return new Date(Number(year), Number(month) - 1).toLocaleDateString('pt-BR', { month: 'short', year: 'numeric' }).replace('.', '');
};
/** Percentual em pt-BR com duas casas: 56,65%. */
export const percent = (value: string | number, digits = 2) => `${new Intl.NumberFormat('pt-BR', { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(Number(value) || 0)}%`;
/** Número em pt-BR com casas fixas, sem símbolo de moeda: 1.316,33. */
export const amount = (value: string | number, digits = 2) => new Intl.NumberFormat('pt-BR', { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(Number(value) || 0);
/** Mês curto: "out/26". */
export const monthShort = (value: string) => {
  const [year, month] = value.slice(0, 7).split('-');
  if (!year || !month) return value;
  const name = new Date(Number(year), Number(month) - 1).toLocaleDateString('pt-BR', { month: 'short' }).replace('.', '');
  return `${name}/${year.slice(2)}`;
};
/** Data ISO (AAAA-MM-DD) em pt-BR: 16/10/2026. */
export const dateBR = (value: string) => {
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(value);
  return match ? `${match[3]}/${match[2]}/${match[1]}` : value;
};

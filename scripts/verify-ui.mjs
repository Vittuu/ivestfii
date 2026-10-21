/** UI smoke test with synthetic IPC data; no portfolio is read or bundled. */
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createServer } from 'vite';
import { verifyTooltips } from './verify-tooltip.mjs';
import { verifyVitePrivacy } from './verify-vite-privacy.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const output = path.join(root, '.local', 'ui-verification');
const bundledPlaywright = path.join(os.homedir(), '.cache', 'codex-runtimes', 'codex-primary-runtime', 'dependencies', 'node', 'node_modules', 'playwright', 'index.js');
const playwrightModule = process.env.IVESTFII_PLAYWRIGHT_PATH
  ? await import(pathToFileURL(process.env.IVESTFII_PLAYWRIGHT_PATH).href)
  : await import('playwright').catch((error) => {
    if (error.code !== 'ERR_MODULE_NOT_FOUND') throw error;
    return import(pathToFileURL(bundledPlaywright).href);
  });
const { chromium } = playwrightModule.default || playwrightModule;
const uiVersion = 'pos-fixado-no-resultado-e-compras-separadas-dos-rendimentos';
const checks = [];
const browserErrors = [];
const screenshots = [];
const focusedRowMotion=process.env.IVESTFII_UI_FOCUS==='rows-motion';
const focusedShell=process.env.IVESTFII_UI_FOCUS==='shell';
const focusedCompact=process.env.IVESTFII_UI_FOCUS==='compact';
const focusedPublicData=process.env.IVESTFII_UI_FOCUS==='public-data';
const focusedProfile=process.env.IVESTFII_UI_FOCUS==='profile';
const focusedIncome=process.env.IVESTFII_UI_FOCUS==='income';
const focusedIncomeApi=process.env.IVESTFII_UI_FOCUS==='income-api';
const focusedIncomePerformance=process.env.IVESTFII_UI_FOCUS==='income-performance';
const focusedFixedIncome=process.env.IVESTFII_UI_FOCUS==='fixed-income';
const reportName=focusedRowMotion?'rows-motion-report.json':focusedShell?'shell-report.json':focusedCompact?'compact-report.json':focusedPublicData?'public-data-report.json':focusedProfile?'profile-report.json':focusedIncome?'income-report.json':focusedIncomeApi?'income-api-report.json':focusedIncomePerformance?'income-performance-report.json':focusedFixedIncome?'fixed-income-report.json':'report.json';
const normalize = (value) => value.replace(/\s+/g, ' ').trim();
/** Figures render the currency as a separate unit ("R$" + "201,00"); compare them without spacing. */
const compact = (value) => value.replace(/\s+/g, '');
const escapeRegExp = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

const entry = (id, price, dividend) => ({ id, month: '2020-02', cotasAdded: id === 'entry-a' ? '10' : '2', pricePerCota: price, dividendPerCota: id === 'entry-a' ? '1' : '0.5', dividendTotal: dividend, notes: 'Lançamento fictício usado apenas na verificação.' });
const position = {
  referenceDate: '2020-02-03', institution: 'Corretora de teste', importedAt: '2020-02-04T12:00:00Z',
  sourcePath: 'X:\\fixture\\synthetic-position.json', sourceHash: 'a'.repeat(64),
  overview: { updatedTime: '02:00', capturedTime: '02:01', totalWithCash: '201', invested: '191', cash: '10', listedFunds: '139', fixedIncome: '50', pendingIncome: '2' },
  listedFunds: {
    capturedTime: '02:02', marketValue: '140', costBasis: '1040', unrealizedResult: '-900', dailyResult: '1.5',
    holdings: [
      { ticker: 'TEST11', shares: '10', averagePrice: '100', price: '9', marketValue: '90', costBasis: '1000', unrealizedResult: '-910' },
      { ticker: 'MOCK11', shares: '2', averagePrice: '20', price: '25', marketValue: '50', costBasis: '40', unrealizedResult: '10' },
    ],
  },
  fixedIncome: { name: 'Fundo sintético pós-fixado', nameTruncated: false, balance: '50', netBalance: '49', reportedEarnings: '4', return12Months: '5', redemptionConversion: 'D+0', redemptionSettlement: 'D+0' },
  warnings: ['O panorama e o detalhe foram capturados em horários diferentes. Os valores são fictícios.'],
};
const snapshot = {
  funds: [
    { id: 'fund-a', ticker: 'TEST11', name: 'Fundo sintético A', sector: 'Teste', entries: [entry('entry-a', '100', '10')], totalCotas: '10', totalInvested: '1000', averagePrice: '100', dividendsReceived: '10', estimatedMonthlyIncome: '10' },
    { id: 'fund-b', ticker: 'MOCK11', name: 'Fundo sintético B', sector: 'Teste', entries: [entry('entry-b', '20', '1')], totalCotas: '2', totalInvested: '40', averagePrice: '20', dividendsReceived: '1', estimatedMonthlyIncome: '1' },
  ],
  totals: { invested: '1040', cotas: '12', dividendsReceived: '11', estimatedMonthlyIncome: '11' },
  recordCount: 2, brokerPosition: position,
};

// Supplemental broker screenshots carry their own dates and never rewrite the
// older overview or create cash/dividend records. All amounts are synthetic.
const fixedIncomeObservation={
  referenceDate:'2026-10-20',name:'Pós-fixado sintético com aportes',category:'Renda Fixa',riskLevel:'Risco Baixo 1',
  balance:'360',invested:'300',totalEarnings:'60',earnings12Months:'30',return12Months:'8.5',returnReferenceDate:'2026-10-15',
  contributions:[
    {applicationDate:'2023-02-01',invested:'100',currentValue:'130',earnings:'30',returnPercent:'30'},
    {applicationDate:'2024-06-01',invested:'110',currentValue:'132',earnings:'22',returnPercent:'20'},
    {applicationDate:'2025-01-10',invested:'90',currentValue:'98',earnings:'8',returnPercent:'8.89'},
  ],
  sources:[{fileName:'synthetic-fixed-balance.png',sha256:'b'.repeat(64)},{fileName:'synthetic-fixed-contributions.jpg',sha256:'c'.repeat(64)}],
  importedAt:'2026-10-20T22:30:00Z',sourcePath:'X:\\fixture\\synthetic-fixed-income.json',sourceHash:'d'.repeat(64),
};

// Deliberately independent of the default financial fixtures. Repeated months,
// explicit zero and a missing total must remain distinct in the presentation.
const ledgerEntries = [
  {id:'ledger-purchase',month:'2020-01',cotasAdded:'10',pricePerCota:'10',dividendPerCota:'0',dividendTotal:'0',notes:'Compra sintética.'},
  {id:'ledger-income',month:'2020-02',cotasAdded:'0',pricePerCota:'10',dividendPerCota:'0.5',dividendTotal:null,notes:'Recebimento calculado sintético.'},
  {id:'ledger-both',month:'2020-03',cotasAdded:'2',pricePerCota:'12.50',dividendPerCota:'0.5',dividendTotal:null,notes:'Compra e recebimento no mesmo lançamento.'},
  {id:'ledger-explicit-zero',month:'2020-04',cotasAdded:'0',pricePerCota:'10',dividendPerCota:'1',dividendTotal:'0',notes:'Zero informado prevalece sobre o valor por cota.'},
  {id:'ledger-manual-income',month:'2020-05',cotasAdded:'0',pricePerCota:'10',dividendPerCota:'0',dividendTotal:'3.75',notes:'Recebimento explicitamente informado.'},
  {id:'ledger-same-month-income',month:'2020-06',cotasAdded:'0',pricePerCota:'0',dividendPerCota:'0.10',dividendTotal:null,notes:'Usa o saldo final de cotas do mês, como o núcleo.'},
  {id:'ledger-same-month-purchase',month:'2020-06',cotasAdded:'3',pricePerCota:'10',dividendPerCota:'0',dividendTotal:'0',notes:'Compra distinta no mesmo mês.'},
  {id:'ledger-price-only',month:'2020-07',cotasAdded:'0',pricePerCota:'13',dividendPerCota:'0',dividendTotal:null,notes:'Preço histórico, sem movimentação.'},
  {id:'ledger-zero-price',month:'2020-08',cotasAdded:'1',pricePerCota:'0',dividendPerCota:'0',dividendTotal:null,notes:'Quantidade registrada com preço zero.'},
];
const incomeEntry=(id,month,{added='0',price='10',perShare='0',received=null}={})=>({id,month,cotasAdded:added,pricePerCota:price,dividendPerCota:perShare,dividendTotal:received,notes:'Registro sintético do gráfico mensal.'});
const incomeFunds=[
  {id:'income-alpha',ticker:'ALFA11',name:'Fundo sintético Alfa',sector:'Teste',entries:[
    incomeEntry('alpha-buy','2025-10',{added:'10'}),
    incomeEntry('alpha-nov','2025-11',{perShare:'0.5'}),
    incomeEntry('alpha-jan','2026-01',{perShare:'0.6'}),
    incomeEntry('alpha-zero','2026-02',{perShare:'1',received:'0'}),
    incomeEntry('alpha-buy-only','2026-03',{added:'2'}),
    incomeEntry('alpha-apr-computed','2026-04',{perShare:'0.25'}),
    incomeEntry('alpha-apr-extra','2026-04',{received:'1.5'}),
    incomeEntry('alpha-may','2026-05',{received:'7'}),
    incomeEntry('alpha-sep','2026-09',{perShare:'0.1'}),
    incomeEntry('alpha-oct','2026-10',{received:'2'}),
  ],totalCotas:'12',totalInvested:'120',averagePrice:'10',dividendsReceived:'25.7',estimatedMonthlyIncome:'0'},
  {id:'income-beta',ticker:'BETA11',name:'Fundo sintético Beta',sector:'Teste',entries:[
    incomeEntry('beta-buy','2025-10',{added:'5',price:'20'}),
    incomeEntry('beta-nov','2025-11',{price:'20',perShare:'0.2'}),
    incomeEntry('beta-zero','2026-02',{price:'20',received:'0'}),
    incomeEntry('beta-apr','2026-04',{price:'20',perShare:'0.25'}),
    incomeEntry('beta-may','2026-05',{price:'20',received:'1'}),
    incomeEntry('beta-sep','2026-09',{price:'20',received:'0'}),
  ],totalCotas:'5',totalInvested:'100',averagePrice:'20',dividendsReceived:'3.25',estimatedMonthlyIncome:'0'},
  {id:'income-future',ticker:'FUTR11',name:'Fundo sintético ainda sem cotas no período',sector:'Teste',entries:[incomeEntry('future-buy','2026-11',{added:'3',price:'30'})],totalCotas:'3',totalInvested:'90',averagePrice:'30',dividendsReceived:'0',estimatedMonthlyIncome:'0'},
];
const incomeExpected=[
  ['2025-11','6.00'],['2025-12',null],['2026-01','6.00'],['2026-02','0.00'],['2026-03',null],['2026-04','5.75'],
  ['2026-05','8.00'],['2026-06',null],['2026-07',null],['2026-08',null],['2026-09','1.20'],['2026-10','2.00'],
];
// Fixed native-contract responses, rather than a second implementation of the
// entitlement calculator. Core tests separately verify dated purchase arithmetic.
const incomeApiEvent=(symbol,month,rate,shares,amount)=>({id:`synthetic-${symbol}-${month}`,symbol,amountPerShare:rate,recordDate:`${month}-10`,paymentDate:`${month}-20`,eligibleShares:shares,amount,source:'B3',sourceUrl:'https://sistemaswebb3-listados.b3.com.br/fundsListedPage/events-main/'+symbol.slice(0,4),status:'api',missingPurchaseDates:[],reasons:[]});
const incomeApiFund=(fundId,symbol,holdings,event)=>({fundId,symbol,status:'api',amount:event.amount,holdings,eligibleShares:event.eligibleShares,events:[event],manualRecordIds:[],reasons:[]});
const incomeApiReport={startMonth:'2025-10',endMonth:'2026-10',asOf:'2026-10-20',ignoredEventCount:0,months:[
  {month:'2026-06',total:'5',knownTotal:'5',recordedAmount:'0',apiAmount:'5',status:'complete',recordedFundCount:1,apiFundCount:1,missingFundCount:0,funds:[
    incomeApiFund('income-alpha','ALFA11','12',incomeApiEvent('ALFA11','2026-06','0.5','10','5')),
    {fundId:'income-beta',symbol:'BETA11',status:'recorded',amount:'0',holdings:'5',eligibleShares:null,events:[],manualRecordIds:['beta-june-zero'],reasons:[]},
  ]},
  {month:'2026-07',total:'7',knownTotal:'7',recordedAmount:'0',apiAmount:'7',status:'complete',recordedFundCount:0,apiFundCount:2,missingFundCount:0,funds:[
    incomeApiFund('income-alpha','ALFA11','15',incomeApiEvent('ALFA11','2026-07','0.5','12','6')),
    incomeApiFund('income-beta','BETA11','5',incomeApiEvent('BETA11','2026-07','0.2','5','1')),
  ]},
  {month:'2026-08',total:'12',knownTotal:'12',recordedAmount:'0',apiAmount:'12',status:'complete',recordedFundCount:0,apiFundCount:2,missingFundCount:0,funds:[
    incomeApiFund('income-alpha','ALFA11','15',incomeApiEvent('ALFA11','2026-08','0.7','15','10.5')),
    incomeApiFund('income-beta','BETA11','5',incomeApiEvent('BETA11','2026-08','0.3','5','1.5')),
  ]},
]};
const incomeApiDividendPanel={items:['ALFA11','BETA11'].map(symbol=>({symbol,status:'cached',observation:{symbol,fetchedAt:'2026-10-20T12:00:00Z',source:'B3',sourceUrl:'https://sistemaswebb3-listados.b3.com.br/fundsListedPage/events-main/'+symbol.slice(0,4),events:[...incomeApiReport.months.flatMap(month=>month.funds.filter(fund=>fund.symbol===symbol).flatMap(fund=>fund.events)),...(symbol==='BETA11'?[incomeApiEvent('BETA11','2026-06','0.8','5','4')]:[])]},error:null})),fetchedAt:'2026-10-20T12:00:00Z',errors:[]};
const incomeApiUnknownReport={...incomeApiReport,months:[{...incomeApiReport.months[1],total:null,knownTotal:'1',apiAmount:'1',status:'partial',apiFundCount:1,missingFundCount:1,funds:[
  {...incomeApiReport.months[1].funds[0],status:'date_required',amount:null,eligibleShares:null,events:[{...incomeApiReport.months[1].funds[0].events[0],status:'date_required',amount:null,eligibleShares:null,missingPurchaseDates:['alpha-july-purchase'],reasons:['Informe o dia da compra para conferir a data-com.']}],reasons:['Informe o dia da compra para conferir a data-com.']},
  incomeApiReport.months[1].funds[1],
]}]};
const importWarnings = [
  'Os horários das capturas dos fundos são diferentes.',
  'O panorama e os fundos listados foram capturados separadamente.',
  'Cada conjunto de valores tem seu horário de referência.',
  'O nome do pós-fixado está abreviado na fonte.',
  'O período do rendimento exibido não foi informado.',
  'Os proventos da posição não são um novo recebimento.',
  'A natureza dos proventos não foi detalhada pela fonte.',
  'O custo da posição pode divergir das compras históricas.',
  'Os percentuais do documento usam outro denominador.',
  'Nota original sem assunto reconhecido.\nSegunda linha preservada integralmente.',
];

const wallets = [{id:'wallet-original',name:'Carteira de teste',institution:'Corretora de teste',isDefault:true},{id:'wallet-other',name:'Carteira Nubank teste',institution:'Nubank teste',isDefault:false}];
const blankProfile={riskTolerance:'unknown',shortTermObjective:'',shortTermMonths:null,longTermObjective:'',longTermMonths:null,liquidityNeeds:'',emergencyReserveStatus:'unknown',monthlyContribution:null,knowledgeLevel:null,constraints:'',notes:''};
const otherProfile={...blankProfile,riskTolerance:'conservative',longTermObjective:'Objetivo exclusivo da outra carteira sintética.',longTermMonths:240,liquidityNeeds:'Liquidez exclusiva da carteira B.',emergencyReserveStatus:'complete',monthlyContribution:'250',knowledgeLevel:'intermediate',notes:'Nota privada sintética B.',updatedAt:'2026-10-20T10:00:00Z'};
const cash = {id:'pocket-a',name:'Reserva sintética',institution:'Instituição de teste',balance:'1234.56',totalDeposits:'1200',totalWithdrawals:'0',balanceDate:'2020-02-03',cdiPercent:'120',annualCdiRate:'10',rateDate:'2020-02-03',rateSource:'manual',rateSourceUrl:null,rateFetchedAt:null,notes:'Saldo fictício de teste offline.'};
const cdi = {id:'cdi',name:'CDI anualizado',value:'8.2',unit:'% a.a.',referenceDate:'2020-02-03',source:'Banco Central · SGS',sourceUrl:'https://api.bcb.gov.br/dados/serie/bcdata.sgs.4389/dados/ultimos/1?formato=json',fetchedAt:'2020-02-04T12:00:00Z'};
const economics = {indicators:[cdi,{...cdi,id:'selic',name:'Meta Selic',value:'8.5'},{...cdi,id:'ipca',name:'IPCA',value:'0.2',unit:'% no mês',referenceDate:'2020-01-01'}],errors:[],fetchedAt:cdi.fetchedAt};
const news = {items:[{id:'news-a',title:'Manchete econômica fictícia para teste de interface',url:'https://agenciabrasil.ebc.com.br/economia/noticia/2020-02/noticia-sintetica',publishedAt:'2020-02-03T12:00:00Z',source:'Agência Brasil'}],source:'Agência Brasil',sourceUrl:'https://agenciabrasil.ebc.com.br/rss/economia/feed.xml',fetchedAt:cdi.fetchedAt};
const quoteObservation = {symbol:'TEST11',price:'8.99',currency:'BRL',quotedAt:'2020-02-03T18:00:00Z',fetchedAt:'2020-02-04T12:00:00Z',source:'brapi',sourceUrl:'https://brapi.dev/',delayMinutes:30};
// A successful fetch can still contain an older trading observation; never conflate these dates.
const recentQuoteObservation = {...quoteObservation,fetchedAt:'2026-10-20T12:00:00Z'};
const quotePanel = {items:[{symbol:'TEST11',status:'not_configured',observation:quoteObservation,error:'Cadastre a chave para atualizar.'},{symbol:'MOCK11',status:'not_configured',observation:null,error:null}],fetchedAt:quoteObservation.fetchedAt,credentialConfigured:false,errors:[]};
const cvmReport = {cnpj:'12345678000195',fundName:'Fundo CVM sintético A',referenceDate:'2020-01-31',filedAt:'2020-02-03T12:00:00Z',netAssets:'1000000',equityPerShare:'10.25',shareholders:123,issuedShares:'97560.9756',source:'CVM · informe mensal',sourceUrl:'https://dados.cvm.gov.br/dataset/fii-doc-inf_mensal',fetchedAt:'2020-02-04T12:00:00Z'};
// The instrument label is real; all report contents and the CNPJ remain synthetic.
const fiagroReport = {...cvmReport,cnpj:'11111111000191',fundName:'FIAGRO sintético VGIA11',fundType:'FIAGRO',referenceDate:'2026-09-30',filedAt:'2026-10-08T12:00:00Z',equityPerShare:null,shareholders:null,issuedShares:null,source:'CVM · Informe Mensal de FIAGRO',sourceUrl:'https://dados.cvm.gov.br/dados/FIAGRO/DOC/INF_MENSAL/DADOS/inf_mensal_fiagro_202609.zip',fetchedAt:'2026-10-20T12:00:00Z'};
const cvmPanel = {items:[{symbol:'TEST11',cnpj:cvmReport.cnpj,status:'cached',report:cvmReport,error:null},{symbol:'MOCK11',cnpj:null,status:'mapping_required',report:null,error:null}],fetchedAt:cvmReport.fetchedAt,errors:[]};
const projection = [{month:'2020-03',projectedCotas:'12',projectedIncome:'10',cumulativeIncome:'10',reinvestedCotas:'1',combinedCotas:'13',combinedIncome:'11'},{month:'2020-04',projectedCotas:'13',projectedIncome:'12',cumulativeIncome:'22',reinvestedCotas:'2',combinedCotas:'15',combinedIncome:'14'}];
const performance = {status:'available',asOf:'2020-02-03',startMonth:'2020-02',invested:'1040',dividends:'11',marketValue:'139.90',priceResult:'-900.10',totalResult:'-889.10',totalReturnPercent:'-85.4903846154',cdi:{endingValue:'1120',result:'80',returnPercent:'7.6923076923'},difference:'-969.10',issues:[],assumptions:['Comparação sintética, com compras no fim do mês.'],quoteSource:'brapi',quoteFetchedAt:'2020-02-04T12:00:00Z',cdiSource:'Banco Central · CDI',cdiSourceUrl:'https://api.bcb.gov.br/dados/serie/bcdata.sgs.4391/dados?formato=json',cdiFetchedAt:'2020-02-04T12:00:00Z'};
// Literal native-contract responses: the UI must display the reconciled result,
// not add the public panel to recorded totals or reproduce entitlement arithmetic.
const incomePerformance={
  ...performance,asOf:'2026-10-20',dividends:'30',totalResult:'-870.10',totalReturnPercent:'-83.6634615385',difference:'-950.10',
  income:{recordedAmount:'11',apiAmount:'19',asOf:'2026-10-20',startMonth:'2020-02',endMonth:'2026-10',status:'complete',missingFundMonthCount:0,dateRequiredFundMonthCount:0,ignoredEventCount:0,byFund:[
    {fundId:'fund-a',symbol:'TEST11',recordedAmount:'10',apiAmount:'15',totalAmount:'25',missingMonthCount:0,dateRequiredMonthCount:0},
    {fundId:'fund-b',symbol:'MOCK11',recordedAmount:'1',apiAmount:'4',totalAmount:'5',missingMonthCount:0,dateRequiredMonthCount:0},
  ]},
  portfolio:{status:'available',invested:'2240',knownInvested:'2240',income:'64.56',knownIncome:'64.56',currentValue:'1374.46',knownCurrentValue:'1374.46',totalResult:'-835.54',knownResult:'-835.54',totalReturnPercent:'-37.3008928571',withdrawals:'0',issues:[],cash:{balance:'1234.56',invested:'1200',income:'34.56',withdrawals:'0',knownInvested:'1200',knownIncome:'34.56',count:1,missingBasisCount:0,items:[{id:'pocket-a',name:'Reserva sintética',balance:'1234.56',invested:'1200',withdrawals:'0',income:'34.56',balanceDate:'2020-02-03'}]}},
};
const incomePerformanceOverridden={...incomePerformance,income:{...incomePerformance.income,recordedAmount:'26',apiAmount:'4',byFund:[
  {fundId:'fund-a',symbol:'TEST11',recordedAmount:'25',apiAmount:'0',totalAmount:'25',missingMonthCount:0,dateRequiredMonthCount:0},
  incomePerformance.income.byFund[1],
]}};
const incomePerformanceZero={...incomePerformance,dividends:'15',totalResult:'-885.10',totalReturnPercent:'-85.1057692308',difference:'-965.10',
  income:{...incomePerformance.income,apiAmount:'4',byFund:[{...incomePerformance.income.byFund[0],apiAmount:'0',totalAmount:'10'},incomePerformance.income.byFund[1]]},
  portfolio:{...incomePerformance.portfolio,income:'49.56',knownIncome:'49.56',totalResult:'-850.54',knownResult:'-850.54',totalReturnPercent:'-37.9705357143'},
};
// Literal native totals protect against adding the old broker balance, the
// contribution detail, or the independent 12-month gain a second time in React.
const fixedIncomePerformance={...performance,portfolio:{
  status:'available',invested:'2540',knownInvested:'2540',income:'105.56',knownIncome:'105.56',currentValue:'1734.46',knownCurrentValue:'1734.46',totalResult:'-794.54',knownResult:'-794.54',totalReturnPercent:'-31.2811023622',withdrawals:'0',issues:[],
  cash:incomePerformance.portfolio.cash,
  fixedIncome:{status:'available',name:fixedIncomeObservation.name,balance:'360',invested:'300',income:'60',result:'60',referenceDate:'2026-10-20',sourceHash:'d'.repeat(64),issues:[]},
}};
const fixedIncomeApiPerformance={...incomePerformance,portfolio:{...fixedIncomePerformance.portfolio,
  income:'124.56',knownIncome:'124.56',totalResult:'-775.54',knownResult:'-775.54',totalReturnPercent:'-30.5330708661',
}};
const fixedIncomeLegacyPerformance={...performance,portfolio:{
  status:'incomplete',invested:null,knownInvested:'2240',income:null,knownIncome:'45.56',currentValue:'1424.46',knownCurrentValue:'1424.46',totalResult:null,knownResult:'-854.54',totalReturnPercent:null,withdrawals:'0',issues:['Faltam o valor aplicado e o ganho acumulado do pós-fixado para fechar os totais.'],
  cash:incomePerformance.portfolio.cash,
  fixedIncome:{status:'incomplete',name:position.fixedIncome.name,balance:'50',invested:null,income:null,result:null,referenceDate:position.referenceDate,sourceHash:position.sourceHash,issues:['O registro antigo não informa o valor aplicado.']},
}};
const fixedIncomeAbsentPerformance={...performance,portfolio:{
  status:'available',invested:'2240',knownInvested:'2240',income:'45.56',knownIncome:'45.56',currentValue:'1374.46',knownCurrentValue:'1374.46',totalResult:'-854.54',knownResult:'-854.54',totalReturnPercent:'-38.1491071429',withdrawals:'0',issues:[],cash:incomePerformance.portfolio.cash,
  fixedIncome:{status:'empty',name:null,balance:'0',invested:null,income:null,result:null,referenceDate:null,sourceHash:null,issues:[]},
}};
const incomePerformanceDividendPanel={items:['TEST11','MOCK11'].map(symbol=>({symbol,status:'cached',observation:{symbol,fetchedAt:'2026-10-20T12:00:00Z',source:'B3',sourceUrl:'https://sistemaswebb3-listados.b3.com.br/fundsListedPage/events-main/'+symbol.slice(0,4),events:[incomeApiEvent(symbol,'2026-03',symbol==='TEST11'?'1.5':'2',symbol==='TEST11'?'10':'2',symbol==='TEST11'?'15':'4'),incomeApiEvent(symbol,'2020-02','1','10','10'),incomeApiEvent(symbol,'2026-11','1','10','10')]},error:null})),fetchedAt:'2026-10-20T12:00:00Z',errors:[]};
const chatFixture={
  conversations:[{id:'conversation-a',title:'Conversa sintética preservada',createdAt:'2020-02-03T12:00:00Z',updatedAt:'2020-02-03T12:02:00Z'}],
  messages:[{id:'message-a',conversationId:'conversation-a',role:'user',content:'Pergunta sintética sobre a carteira de teste.',provider:'codex',model:null,createdAt:'2020-02-03T12:00:00Z'},{id:'message-b',conversationId:'conversation-a',role:'assistant',content:'Resposta sintética preservada entre provedores.',provider:'codex',model:null,createdAt:'2020-02-03T12:02:00Z'}],
  tools:[{id:'tool-a',conversationId:'conversation-a',taskId:null,toolName:'preparar_analise_carteira',input:{},output:{status:'fixture'},status:'completed',startedAt:'2020-02-03T12:01:00Z',completedAt:'2020-02-03T12:01:01Z'}],runs:{},sequence:0,
};
const pages = ['Visão geral', 'FIIs e posições', 'Caixinhas CDI', 'Mercado', 'Projeções', 'Assistente', 'Configurações'];

// Older IPC fixtures compose cash with the selected FII response so edits can
// change either side. New fixed-income cases use literal native response DTOs.
function cashPerformance(pockets) {
  const decimal = value => String(Number(value.toFixed(10)));
  const items = pockets.map(pocket => {
    const known = [pocket.totalDeposits,pocket.totalWithdrawals,pocket.balanceDate].every(value=>value!==null&&value!==undefined&&value!=='');
    return {id:pocket.id,name:pocket.name,balance:pocket.balance,invested:known?pocket.totalDeposits:null,withdrawals:known?pocket.totalWithdrawals:null,income:known?decimal(Number(pocket.balance)+Number(pocket.totalWithdrawals)-Number(pocket.totalDeposits)):null,balanceDate:pocket.balanceDate??null};
  });
  const sum = key => decimal(items.reduce((total,item)=>total+Number(item[key]??0),0));
  const missingBasisCount = items.filter(item=>item.invested===null).length;
  return {balance:sum('balance'),invested:missingBasisCount?null:sum('invested'),income:missingBasisCount?null:sum('income'),withdrawals:missingBasisCount?null:sum('withdrawals'),knownInvested:sum('invested'),knownIncome:sum('income'),count:items.length,missingBasisCount,items};
}
function withPortfolio(snapshot, fii) {
  if(fii.portfolio)return structuredClone(fii);
  const decimal = value => String(Number(value.toFixed(10)));
  const cash = cashPerformance(snapshot.cashPockets);
  const empty = fii.status==='empty';
  const cutoff=fii.asOf?.slice(0,7);
  const future=snapshot.funds.some(fund=>fund.entries.some(entry=>cutoff&&entry.month>cutoff));
  const fullCost=snapshot.funds.reduce((sum,fund)=>sum+fund.entries.reduce((cost,entry)=>cost+Number(entry.cotasAdded)*Number(entry.pricePerCota),0),0);
  const fullIncome=snapshot.funds.reduce((sum,fund)=>sum+Number(fund.dividendsReceived),0);
  const costComplete=!future&&!snapshot.funds.some(fund=>fund.entries.some(entry=>Number(entry.cotasAdded)>0&&Number(entry.pricePerCota)===0))&&(!snapshot.funds.length||Math.abs(fullCost-Number(fii.invested))<0.0000001);
  const incomeComplete=!future&&(!snapshot.funds.length||Math.abs(fullIncome-Number(fii.income?.recordedAmount??fii.dividends))<0.0000001);
  const fiiInvested = empty?0:Number(fii.invested);
  const fiiIncome = empty?0:Number(fii.dividends);
  const market = empty?0:fii.marketValue===null?null:Number(fii.marketValue);
  const result = empty?0:!costComplete||!incomeComplete||fii.totalResult===null?null:Number(fii.totalResult);
  const invested = !costComplete||cash.invested===null?null:decimal(fiiInvested+Number(cash.invested));
  const income = !incomeComplete||cash.income===null?null:decimal(fiiIncome+Number(cash.income));
  const currentValue = market===null?null:decimal(market+Number(cash.balance));
  const totalResult = result===null||cash.income===null?null:decimal(result+Number(cash.income));
  const complete = [invested,income,currentValue,totalResult].every(value=>value!==null);
  const issues=[];
  if(!costComplete)issues.push('O custo completo dos FIIs não está disponível: há compras sem preço ou valores fora do período calculado.');
  if(!incomeComplete)issues.push('Os proventos de FIIs calculados não cobrem todo o histórico; a parcela conhecida foi mantida separada.');
  if(!empty&&result===null)issues.push('O resultado dos FIIs ainda está incompleto. Consulte as cotações e a conciliação no painel de FIIs.');
  if(cash.missingBasisCount)issues.push(`Faltam os depósitos, as retiradas ou a data do saldo de ${cash.missingBasisCount} caixinha(s). Os totais completos de custo e resultado permanecem indisponíveis.`);
  return {...structuredClone(fii),portfolio:{status:empty&&!cash.count?'empty':complete?'available':'incomplete',invested,knownInvested:decimal(fiiInvested+Number(cash.knownInvested)),income,knownIncome:decimal(fiiIncome+Number(cash.knownIncome)),currentValue,knownCurrentValue:decimal((market??0)+Number(cash.balance)),totalResult,knownResult:decimal((result??0)+Number(cash.knownIncome)),totalReturnPercent:invested!==null&&Number(invested)>0&&totalResult!==null?decimal(Number(totalResult)/Number(invested)*100):null,withdrawals:cash.withdrawals,cash,issues}};
}

function bootstrap(mode) {
  if (mode === 'web') return `globalThis.isTauri = false; await import('/src/main.tsx?ui-original=web');`;
  const initial = structuredClone(snapshot);
  initial.cashPockets = [cash]; initial.cashBalance = '1234.56';
  if(mode==='fixed-income'||mode==='fixed-income-api')initial.brokerPosition.fixedIncome.latestObservation=structuredClone(fixedIncomeObservation);
  if(mode==='fixed-income-none')initial.brokerPosition=null;
  if (mode === 'position-notes') initial.brokerPosition.warnings = [...importWarnings];
  if (mode === 'ledger'||mode==='ledger-recent') {
    initial.funds = [{...initial.funds[0],entries:structuredClone(ledgerEntries),totalCotas:'16',totalInvested:'155',averagePrice:'9.6875',dividendsReceived:'16.25',estimatedMonthlyIncome:'0'}];
    if(mode==='ledger-recent')for(const item of initial.funds[0].entries){if(item.id==='ledger-purchase')item.purchaseDate='2020-01-21';if(item.id==='ledger-both')item.purchaseDate='2020-03-11';}
    initial.totals = {invested:'155',cotas:'16',dividendsReceived:'16.25',estimatedMonthlyIncome:'0'};
    initial.recordCount = ledgerEntries.length;
  }
  if(mode.startsWith('income')&&!mode.startsWith('income-performance')){
    initial.funds=structuredClone(incomeFunds);initial.brokerPosition=null;initial.cashPockets=[];initial.cashBalance='0';
    initial.totals={invested:'310',cotas:'20',dividendsReceived:'28.95',estimatedMonthlyIncome:'0'};
    if(mode==='income-complete'){
      // A separate control proves that a real, fully observed zero month belongs
      // in the mean, while the current partial month remains outside it.
      for(const [month,total] of [['2025-12','4'],['2026-03','0'],['2026-06','2'],['2026-07','2'],['2026-08','2']]){
        initial.funds[0].entries.push(incomeEntry('alpha-complete-'+month,month,{received:total}));
        initial.funds[1].entries.push(incomeEntry('beta-complete-'+month,month,{price:'20',received:'0'}));
      }
      initial.funds[1].entries.push(incomeEntry('beta-complete-jan','2026-01',{price:'20',received:'0'}));
      initial.funds[0].dividendsReceived='35.7';initial.totals.dividendsReceived='38.95';
    }
    if(mode==='income-unknown'||mode==='income-zero'){
      for(const fund of initial.funds){fund.entries=fund.entries.filter(item=>Number(item.cotasAdded)>0);fund.dividendsReceived='0';}
      if(mode==='income-zero')for(const fund of initial.funds.slice(0,2))fund.entries.push(incomeEntry(fund.id+'-zero','2026-02',{received:'0'}));
      initial.totals.dividendsReceived='0';
    }
    if(mode.startsWith('income-api')){
      initial.funds[0].entries=[
        {...incomeEntry('alpha-buy','2025-10',{added:'10'}),purchaseDate:'2025-10-01'},
        {...incomeEntry('alpha-june-purchase','2026-06',{added:'2'}),purchaseDate:'2026-06-25'},
        {...incomeEntry('alpha-july-purchase','2026-07',{added:'3'}),...(mode==='income-api-date-required'?{}:{purchaseDate:'2026-07-25'})},
      ];
      initial.funds[0].totalCotas='15';initial.funds[0].dividendsReceived='0';
      initial.funds[1].entries=[{...incomeEntry('beta-buy','2025-10',{added:'5',price:'20'}),purchaseDate:'2025-10-01'},incomeEntry('beta-june-zero','2026-06',{price:'20',received:'0'})];
      initial.funds[1].dividendsReceived='0';initial.totals.dividendsReceived='0';
    }
    initial.recordCount=initial.funds.reduce((sum,fund)=>sum+fund.entries.length,0);
  }
  if (mode === 'connected') initial.funds.push({...initial.funds[0],id:'fund-fiagro',ticker:'VGIA11',name:'FIAGRO sintético VGIA11',entries:[],totalCotas:'0',totalInvested:'0',averagePrice:'0',dividendsReceived:'0',estimatedMonthlyIncome:'0'});
  const other = structuredClone(initial);
  other.brokerPosition = null; other.funds = [{...initial.funds[0],id:'fund-other',ticker:'BANK11',name:'Fundo de outra carteira',entries:[{...entry('entry-other','100','0'),cotasAdded:'2',dividendPerCota:'0'}],totalCotas:'2',totalInvested:'200',averagePrice:'100',dividendsReceived:'0',estimatedMonthlyIncome:'0'}];
  other.totals = {invested:'200',cotas:'2',dividendsReceived:'0',estimatedMonthlyIncome:'0'};other.recordCount=0;
  other.cashPockets=[{...cash,id:'pocket-other',name:'Reserva da outra carteira',balance:'300',totalDeposits:'280'}];other.cashBalance='300';
  if (mode === 'without-position') initial.brokerPosition = null;
  const automaticMode=mode.startsWith('auto-');
  const keyConfigured = mode === 'connected' || mode === 'corrupt-quotes' || (automaticMode&&mode!=='auto-no-key');
  let walletPerformance = (mode === 'ledger'||mode==='ledger-recent') ? {...performance,status:'incomplete',invested:'155',dividends:'16.25',marketValue:null,priceResult:null,totalResult:null,totalReturnPercent:null,cdi:null,difference:null,issues:['Cotações ausentes no cenário sintético de lançamentos.']} : performance;
  if(mode.startsWith('fixed-income'))walletPerformance=structuredClone(mode==='fixed-income-api'?fixedIncomeApiPerformance:mode==='fixed-income-legacy'?fixedIncomeLegacyPerformance:mode==='fixed-income-none'?fixedIncomeAbsentPerformance:fixedIncomePerformance);
  if(mode.startsWith('income')&&!mode.startsWith('income-performance'))walletPerformance={...performance,status:'incomplete',asOf:'2026-10-20',startMonth:'2025-10',invested:'310',dividends:initial.totals.dividendsReceived,marketValue:null,priceResult:null,totalResult:null,totalReturnPercent:null,cdi:null,difference:null,issues:['Cotações indisponíveis no cenário sintético do gráfico mensal.']};
  if(mode.startsWith('income-performance')){
    walletPerformance=structuredClone(mode==='income-performance-overridden'?incomePerformanceOverridden:mode==='income-performance-zero'?incomePerformanceZero:incomePerformance);
    if(mode==='income-performance-overridden'){
      initial.funds[0].entries.push({...entry('entry-confirmed','0','15'),month:'2026-03',cotasAdded:'0',dividendPerCota:'0'});
      initial.funds[0].dividendsReceived='25';initial.totals.dividendsReceived='26';initial.recordCount=3;
    }
    if(mode==='income-performance-zero'){
      initial.funds[0].entries.push({...entry('entry-zero','0','0'),month:'2026-03',cotasAdded:'0',dividendPerCota:'0'});initial.recordCount=3;
    }
  }
  if(mode.startsWith('cash-')) {
    initial.brokerPosition=null;
    initial.funds=[{...initial.funds[0],entries:[{...entry('entry-a','10','5'),cotasAdded:'10'}],totalCotas:'10',totalInvested:'100',averagePrice:'10',dividendsReceived:'5',estimatedMonthlyIncome:'5'}];
    initial.totals={invested:'100',cotas:'10',dividendsReceived:'5',estimatedMonthlyIncome:'5'};
    initial.recordCount=1;
    initial.cashPockets=[{...cash,balance:'220',totalDeposits:'200',totalWithdrawals:'10'}];initial.cashBalance='220';
    walletPerformance={...performance,invested:'100',dividends:'5',marketValue:'90',priceResult:'-10',totalResult:'-5',totalReturnPercent:'-5',cdi:{endingValue:'108',result:'8',returnPercent:'8'},difference:'-13'};
    if(mode==='cash-legacy') for(const key of ['totalDeposits','totalWithdrawals','balanceDate'])delete initial.cashPockets[0][key];
    if(mode==='cash-negative') initial.cashPockets=[{...initial.cashPockets[0],balance:'80',totalDeposits:'100',totalWithdrawals:'0'}];
    if(mode==='cash-partial') {
      const unknown={...cash,id:'pocket-legacy',name:'Reserva sem histórico',balance:'50'};
      for(const key of ['totalDeposits','totalWithdrawals','balanceDate'])delete unknown[key];
      initial.cashPockets.push(unknown);initial.cashBalance='270';
    }
    if(mode==='cash-only') {
      initial.funds=[];initial.totals={invested:'0',cotas:'0',dividendsReceived:'0',estimatedMonthlyIncome:'0'};initial.recordCount=0;
      walletPerformance={...walletPerformance,status:'empty',invested:'0',dividends:'0',marketValue:null,priceResult:null,totalResult:null,totalReturnPercent:null,cdi:null,difference:null,issues:[]};
    }
    initial.cashBalance=cashPerformance(initial.cashPockets).balance;
  }
  return `
import { mockIPC, mockWindows } from '/node_modules/@tauri-apps/api/mocks.js';
import { emit } from '/node_modules/@tauri-apps/api/event.js';
globalThis.isTauri = true;
globalThis.__uiTest = { snapshot: ${JSON.stringify(initial)}, otherSnapshot: ${JSON.stringify(other)}, calls: [], unexpected: [], failImports: 1, deferSaves: false, saveWaiters: [], deferImports: false, importWaiters: [], deferMarket: false, marketWaiters: [], connected:${mode === 'connected'}, corruptQuoteCache:${mode === 'corrupt-quotes'}, marketSettings: {brapiKeyConfigured:${keyConfigured},brapiKeySource:${keyConfigured ? "'local'" : 'null'},cvmMappings:{TEST11:'12345678000195'}}, settings: {autostart:false,startMinimized:false,closeToTray:true,autoUpdate:true}, update: {status:'up_to_date',currentVersion:'0.1.0',availableVersion:null,downloadedBytes:0,totalBytes:null,lastCheckedAt:'2020-02-04T12:00:00Z',error:null}, failUpdateCheck:false, failUpdateInstall:false };
const wallets=${JSON.stringify(wallets)};
Object.assign(globalThis.__uiTest,{performance:${JSON.stringify(walletPerformance)},otherPerformance:{...${JSON.stringify(performance)},invested:'200',dividends:'0',marketValue:'220',priceResult:'20',totalResult:'20',totalReturnPercent:'10',cdi:{endingValue:'210',result:'10',returnPercent:'5'},difference:'10'},deferPerformance:false,performanceWaiters:[],deferPerformanceRead:${mode === 'performance-loading'},performanceReadWaiters:[],failPerformance:false});
Object.assign(globalThis.__uiTest,{chat:${JSON.stringify(chatFixture)},deferChatStart:false,chatStartWaiters:[],eventSubscriptions:[],eventCalls:[]});
Object.assign(globalThis.__uiTest,{automaticMode:${JSON.stringify(mode)},deferAutoWallets:${JSON.stringify(automaticMode?['wallet-original']:[])},autoWaiters:[],publicCache:{shared:{economics:${JSON.stringify(economics)},news:${JSON.stringify(news)}},wallets:{}},autoCheckedAt:'2026-10-20T12:00:00Z'});
Object.assign(globalThis.__uiTest,{incomeReport:${JSON.stringify(mode==='income-api-date-required'?incomeApiUnknownReport:mode==='income-api'?incomeApiReport:{months:[]})}});
Object.assign(globalThis.__uiTest,{dividendPanel:${JSON.stringify(mode.startsWith('income-api')?incomeApiDividendPanel:mode.startsWith('income-performance')||mode==='fixed-income-api'?incomePerformanceDividendPanel:{items:[],fetchedAt:null,errors:[]})}});
${mode.startsWith('income')?`globalThis.__uiTest.publicCache.wallets['wallet-original']={quotes:{items:[],fetchedAt:null,credentialConfigured:false,errors:[]},cvm:{items:[],fetchedAt:null,errors:[]}};`:''}
Object.assign(globalThis.__uiTest,{profiles:{'wallet-original':${JSON.stringify(mode==='profile-stale'?{...otherProfile,riskTolerance:'aggressive',notes:'Nota da resposta antiga da carteira A.'}:null)},'wallet-other':${JSON.stringify(mode.startsWith('profile')?otherProfile:null)}},deferProfileWallets:${JSON.stringify(mode==='profile-stale'?['wallet-original']:[])},profileReadWaiters:[],deferProfileSaves:false,profileSaveWaiters:[],profileSaveError:null,profileReadError:${JSON.stringify(mode==='profile-error'?'Perfil sintético temporariamente indisponível.':null)}});
${mode.startsWith('profile')?`globalThis.__uiTest.publicCache.wallets['wallet-original']={quotes:{...${JSON.stringify(quotePanel)},items:[{symbol:'TEST11',status:'cached',observation:{...${JSON.stringify(quoteObservation)},quotedAt:'2026-10-20T00:30:00Z'},error:null},{symbol:'MOCK11',status:'cached',observation:{...${JSON.stringify(quoteObservation)},symbol:'MOCK11',price:'25',quotedAt:'not-a-date'},error:null}]}};`:''}
globalThis.__uiTest.readQuotes=(walletId)=>{
  const test=globalThis.__uiTest,stored=test.publicCache.wallets[walletId]?.quotes;
  if(stored)return {...structuredClone(stored),credentialConfigured:test.marketSettings.brapiKeyConfigured};
  const result=${JSON.stringify(quotePanel)};result.credentialConfigured=test.marketSettings.brapiKeyConfigured;
  if(test.corruptQuoteCache){result.items=result.items.map(item=>({...item,status:'unavailable',observation:null,error:'Cache sintético inválido.'}));result.fetchedAt=null;result.errors=['Cache de cotações sintético inválido. Atualize para consultar a fonte.'];}
  if(test.connected){result.items=result.items.map((item,index)=>({...item,status:'cached',observation:{...${JSON.stringify(recentQuoteObservation)},symbol:item.symbol,price:index?'25':'8.99'},error:null}));result.items.push({symbol:'VGIA11',status:'available',observation:{...${JSON.stringify(recentQuoteObservation)},symbol:'VGIA11',price:'9.80'},error:null});result.fetchedAt=${JSON.stringify(recentQuoteObservation.fetchedAt)};}
  if(walletId==='wallet-other')result.items=[{symbol:'BANK11',status:'cached',observation:{...result.items[0].observation,symbol:'BANK11',price:'20'},error:null}];
  return result;
};
globalThis.__uiTest.readCvm=(walletId)=>{
  const test=globalThis.__uiTest,stored=test.publicCache.wallets[walletId]?.cvm;
  if(stored)return structuredClone(stored);
  const result=${JSON.stringify(cvmPanel)};
  if(test.connected){result.items[1]={...result.items[1],cnpj:'11222333000181',status:'cached',report:{...${JSON.stringify(cvmReport)},cnpj:'11222333000181',fundName:'Fundo CVM sintético B',fundType:'FII'},error:null};result.items.push({symbol:'VGIA11',cnpj:${JSON.stringify(fiagroReport.cnpj)},status:'available',report:${JSON.stringify(fiagroReport)},error:null});result.fetchedAt=${JSON.stringify(fiagroReport.fetchedAt)};}
  if(walletId==='wallet-other')result.items=[{symbol:'BANK11',cnpj:null,status:'mapping_required',report:null,error:null}];return result;
};
globalThis.__uiTest.buildAutomaticResult=(walletId)=>{
  const test=globalThis.__uiTest;
  const result={economics:structuredClone(test.publicCache.shared.economics),news:structuredClone(test.publicCache.shared.news),quotes:test.readQuotes(walletId),cvm:test.readCvm(walletId),sources:['economics','news','quotes','cvm','cdiHistory'].map(id=>({id,status:id==='quotes'&&!test.marketSettings.brapiKeyConfigured?'not_configured':'cached'})),checkedAt:test.autoCheckedAt};
  if(!test.automaticMode.startsWith('auto-'))return result;
  if(test.automaticMode==='auto-offline')return {...result,sources:result.sources.map(source=>({...source,status:'unavailable',message:'Sem conexão sintética; mantendo a observação salva.'}))};
  result.sources=result.sources.map(source=>({...source,status:source.id==='quotes'&&!test.marketSettings.brapiKeyConfigured?'not_configured':'updated'}));
  result.economics={...result.economics,indicators:result.economics.indicators.map(indicator=>({...indicator,value:indicator.id==='cdi'?'14.9':indicator.value,referenceDate:'2026-10-19',fetchedAt:test.autoCheckedAt})),fetchedAt:test.autoCheckedAt};
  result.news={...result.news,items:Array.from({length:4},(_,index)=>({id:'auto-news-'+index,title:'Notícia automática sintética '+(index+1),url:'https://agenciabrasil.ebc.com.br/economia/noticia/2026-10/noticia-automatica-'+index,publishedAt:'2026-10-19T12:00:00Z',source:'Agência Brasil'})),fetchedAt:test.autoCheckedAt};
  result.quotes={...result.quotes,items:result.quotes.items.map((item,index)=>({...item,status:'available',observation:{...${JSON.stringify(quoteObservation)},symbol:item.symbol,price:walletId==='wallet-other'?'120':index?'25':'9.25',quotedAt:'2026-10-16T18:00:00Z',fetchedAt:test.autoCheckedAt},error:null})),credentialConfigured:test.marketSettings.brapiKeyConfigured,fetchedAt:test.autoCheckedAt};
  if(!test.marketSettings.brapiKeyConfigured)result.quotes=test.readQuotes(walletId);
  result.cvm={...result.cvm,items:result.cvm.items.map(item=>{const cnpj=item.cnpj||'11222333000181';return {...item,cnpj,status:'available',report:{...${JSON.stringify(cvmReport)},cnpj,referenceDate:'2026-09-30',filedAt:'2026-10-08T12:00:00Z',fetchedAt:test.autoCheckedAt},error:null};}),fetchedAt:test.autoCheckedAt};
  return result;
};
globalThis.__uiTest.emitChatRun=async(patch)=>{
  const test=globalThis.__uiTest;
  const previous=patch.conversationId?test.chat.runs[patch.conversationId]:Object.values(test.chat.runs).at(-1);
  if(!previous)throw new Error('No synthetic chat run to update.');
  const run={...previous,...patch,updatedAt:patch.updatedAt||new Date(Date.parse(previous.updatedAt)+1000).toISOString()};
  test.chat.runs[run.conversationId]=run;
  if(run.status==='completed'&&!test.chat.messages.some(message=>message.id==='assistant-'+run.id))
    test.chat.messages.push({id:'assistant-'+run.id,conversationId:run.conversationId,role:'assistant',content:run.text,provider:run.provider,model:run.model,createdAt:run.updatedAt});
  await emit('chat-run',structuredClone(run));return structuredClone(run);
};
const cashPerformance=${cashPerformance.toString()};
const withPortfolio=${withPortfolio.toString()};
mockIPC(async (command, args={}) => {
  const test = globalThis.__uiTest;
  const current=args.walletId==='wallet-other'?test.otherSnapshot:test.snapshot;
  test.calls.push({command, args});
  switch (command) {
    case 'get_wallets': return structuredClone(wallets);
    case 'get_wealth_overview': {const rows=wallets.map((wallet,index)=>{const value=index?test.otherSnapshot:test.snapshot;return {wallet,fiiInvested:value.totals.invested,cashBalance:cashPerformance(value.cashPockets).balance};});return {wallets:rows,totals:{fiiInvested:String(rows.reduce((sum,row)=>sum+Number(row.fiiInvested),0)),cashBalance:String(rows.reduce((sum,row)=>sum+Number(row.cashBalance),0))}};}
    case 'get_portfolio': return structuredClone(current);
    case 'get_fii_performance': {const result=withPortfolio(current,args.walletId==='wallet-other'?test.otherPerformance:test.performance);if(test.deferPerformanceRead)await new Promise(resolve=>test.performanceReadWaiters.push(resolve));return result;}
    case 'refresh_fii_performance': {const result=withPortfolio(current,args.walletId==='wallet-other'?test.otherPerformance:test.performance);if(test.deferPerformance)await new Promise(resolve=>test.performanceWaiters.push(resolve));if(test.failPerformance)throw new Error('Fontes de resultado temporariamente indisponíveis no teste.');return result;}
    case 'get_app_info': return {
      dataDirectory: 'X:\\\\fixture', databasePath: 'X:\\\\fixture\\\\portfolio.sqlite',
      legacyPath: 'X:\\\\fixture\\\\synthetic-ledger.json',
      positionPath: 'X:\\\\fixture\\\\synthetic-position.json', migration: null,
      positionImport: null, sync: {status: 'not_configured', pendingChanges: 0}
    };
    case 'get_settings': return structuredClone(test.settings);
    case 'save_settings': test.settings=structuredClone(args.settings);return structuredClone(test.settings);
    case 'get_update_status': return structuredClone(test.update);
    case 'check_for_updates': if(test.failUpdateCheck)throw new Error('Servidor de atualização indisponível no teste.');return structuredClone(test.update);
    case 'install_pending_update': if(test.failUpdateInstall)throw new Error('Aguarde o assistente encerrar a tarefa antes de reiniciar.');test.update.status='installing';return structuredClone(test.update);
    case 'set_update_activity': test.updateActivityBlocked=args.blocked;return null;
    case 'get_market_data_settings': return structuredClone(test.marketSettings);
    case 'get_investment_profile': {
      const result=structuredClone(test.profiles[args.walletId]??null);
      if(test.deferProfileWallets.includes(args.walletId))await new Promise(resolve=>test.profileReadWaiters.push({walletId:args.walletId,resolve}));
      if(test.profileReadError)throw new Error(test.profileReadError);
      return result;
    }
    case 'save_investment_profile': {
      if(test.deferProfileSaves)await new Promise(resolve=>test.profileSaveWaiters.push({walletId:args.walletId,resolve}));
      if(test.profileSaveError)throw new Error(test.profileSaveError);
      const saved={...structuredClone(args.input),updatedAt:'2026-10-20T12:30:00Z'};
      test.profiles[args.walletId]=saved;return structuredClone(saved);
    }
    case 'refresh_public_data_if_due': {
      const result=test.buildAutomaticResult(args.walletId);
      if(test.deferAutoWallets.includes(args.walletId))await new Promise(resolve=>test.autoWaiters.push({walletId:args.walletId,resolve}));
      if(test.automaticMode==='auto-rejected')throw new Error('Falha sintética na atualização automática. Os dados salvos continuam disponíveis.');
      if(test.automaticMode.startsWith('auto-')&&test.automaticMode!=='auto-offline'){
        test.publicCache.shared={economics:structuredClone(result.economics),news:structuredClone(result.news)};
        test.publicCache.wallets[args.walletId]={quotes:structuredClone(result.quotes),cvm:structuredClone(result.cvm)};
        if(args.walletId==='wallet-other')test.otherPerformance={...test.otherPerformance,asOf:'2026-10-16',marketValue:'240',priceResult:'40',totalResult:'40',totalReturnPercent:'20',difference:'30'};
        else if(test.marketSettings.brapiKeyConfigured)test.performance={...test.performance,asOf:'2026-10-16',marketValue:'142.50',priceResult:'-897.50',totalResult:'-886.50',totalReturnPercent:'-85.2403846154',difference:'-976.50',cdi:{endingValue:'1130',result:'90',returnPercent:'8.6538461538'},quoteFetchedAt:test.autoCheckedAt,cdiFetchedAt:test.autoCheckedAt};
      }
      return structuredClone(result);
    }
    case 'save_brapi_key': test.marketSettings.brapiKeyConfigured=true;test.marketSettings.brapiKeySource='local';return structuredClone(test.marketSettings);
    case 'clear_brapi_key': test.marketSettings.brapiKeyConfigured=false;test.marketSettings.brapiKeySource=null;return structuredClone(test.marketSettings);
    case 'save_cvm_mapping': if(args.cnpj)test.marketSettings.cvmMappings[args.symbol]=args.cnpj;else delete test.marketSettings.cvmMappings[args.symbol];return structuredClone(test.marketSettings);
    case 'get_quote_cache': return test.readQuotes(args.walletId);
    case 'refresh_quotes': {if(test.deferMarket)await new Promise(resolve=>test.marketWaiters.push(resolve));const result=${JSON.stringify(quotePanel)};result.credentialConfigured=true;result.items[0].status='available';result.items[0].observation.price='9.25';result.items[0].error=null;result.items[1].status='forbidden';result.items[1].error='Ativo não incluído no plano sintético.';result.errors=['Cobertura parcial no cenário sintético.'];test.publicCache.wallets[args.walletId]={...test.publicCache.wallets[args.walletId],quotes:structuredClone(result)};return result;}
    case 'get_cvm_cache': return test.readCvm(args.walletId);
    case 'get_dividend_cache': return args.walletId==='wallet-original'?structuredClone(test.dividendPanel):{items:[],fetchedAt:null,errors:[]};
    case 'get_monthly_income': return args.walletId==='wallet-original'?structuredClone(test.incomeReport):{months:[]};
    case 'refresh_cvm_data': {const result=${JSON.stringify(cvmPanel)};result.items[0].error='CVM temporariamente indisponível no teste.';result.errors=['Consulta parcial da CVM no teste.'];for(const item of result.items){const mapping=test.marketSettings.cvmMappings[item.symbol];if(mapping && !item.report){item.cnpj=mapping;item.report={...${JSON.stringify(cvmReport)},cnpj:mapping,fundName:'Fundo CVM sintético B'};item.status='available';}}test.publicCache.wallets[args.walletId]={...test.publicCache.wallets[args.walletId],cvm:structuredClone(result)};return result;}
    case 'upsert_fund':
      if(test.deferSaves)await new Promise(resolve=>test.saveWaiters.push(resolve));
      current.funds.push({...args.input,id:'fund-ui-new',entries:[],totalCotas:'0',totalInvested:'0',averagePrice:'0',dividendsReceived:'0',estimatedMonthlyIncome:'0'});return null;
    case 'save_record': {
      if(test.deferSaves)await new Promise(resolve=>test.saveWaiters.push(resolve));
      const fund=current.funds.find(item=>item.id===args.input.fundId);
      if(!fund)throw new Error('Fundo sintético inexistente.');
      const saved={...args.input,id:args.input.id||'ledger-new'};
      const index=fund.entries.findIndex(item=>item.id===saved.id);
      if(index>=0)fund.entries[index]=saved;else fund.entries.push(saved);
      return null;
    }
    case 'get_cash_pockets': return structuredClone(current.cashPockets);
    case 'get_cash_performance': return cashPerformance(current.cashPockets);
    case 'save_cash_pocket': {if(test.deferSaves)await new Promise(resolve=>test.saveWaiters.push(resolve));const index=current.cashPockets.findIndex(item=>item.id===args.input.id);const previous=index>=0?current.cashPockets[index]:{};const saved={...previous,...args.input,id:args.input.id||'pocket-new'};if(args.input.clearReconciliation)for(const key of ['totalDeposits','totalWithdrawals','balanceDate'])delete saved[key];delete saved.clearReconciliation;if(index>=0)current.cashPockets[index]=saved;else current.cashPockets.push(saved);current.cashBalance=cashPerformance(current.cashPockets).balance;return structuredClone(saved);}
    case 'fetch_cdi_reference': await new Promise(resolve=>setTimeout(resolve,300));return ${JSON.stringify(cdi)};
    case 'simulate_cash_pocket': return {pocketId:args.input.pocketId,startingBalance:'1234.56',endingBalance:'1246.90',grossEarnings:'12.34',annualCdiRate:'10',cdiPercent:'120',businessDays:args.input.businessDays,rateDate:'2020-02-03',assumptions:['Cenário bruto fictício de teste.']};
    case 'project_income': return ${JSON.stringify(projection)};
    case 'get_economic_cache': return structuredClone(test.publicCache.shared.economics);
    case 'get_news_cache': return structuredClone(test.publicCache.shared.news);
    case 'refresh_economic_indicators': {const result={...${JSON.stringify(economics)},errors:[{id:'cdi',message:'Fonte temporariamente indisponível no teste.'}]};test.publicCache.shared.economics=structuredClone(result);return result;}
    case 'refresh_news': throw new Error('Sem conexão sintética.');
    case 'open_public_source': return null;
    case 'get_providers': return ['codex','claude','antigravity'].map((id,index)=>({id,name:['Codex','Claude','Antigravity'][index],installed:true,ready:true,executable:'X:/fixture/'+id+'.exe',transport:'test',version:'fixture'}));
    case 'list_conversations': return structuredClone(test.chat.conversations);
    case 'create_conversation': {const conversation={id:'conversation-'+(++test.chat.sequence),title:args.title,createdAt:'2026-10-20T12:10:00Z',updatedAt:'2026-10-20T12:10:00Z'};test.chat.conversations.push(conversation);return structuredClone(conversation);}
    case 'get_messages': return structuredClone(test.chat.messages.filter(message=>message.conversationId===args.conversationId));
    case 'get_chat_run': return structuredClone(test.chat.runs[args.conversationId]??null);
    case 'get_tool_events': return structuredClone(test.chat.tools.filter(tool=>tool.conversationId===args.conversationId));
    case 'start_chat_turn': {
      const sequence=++test.chat.sequence,createdAt=new Date(Date.parse('2026-10-20T12:10:00Z')+sequence*10000).toISOString();
      const run={id:'run-'+sequence,conversationId:args.conversationId,walletId:args.walletId??null,provider:args.provider,model:args.model??null,status:'running',text:'',error:null,statusMessage:'Preparando resposta sintética.',createdAt,updatedAt:createdAt};
      test.chat.runs[args.conversationId]=run;
      test.chat.messages.push({id:'question-'+run.id,conversationId:args.conversationId,role:'user',content:args.content,provider:args.provider,model:args.model,createdAt});
      const conversation=test.chat.conversations.find(item=>item.id===args.conversationId);if(conversation)conversation.updatedAt=createdAt;
      await emit('chat-run',structuredClone(run));
      if(test.deferChatStart)await new Promise(resolve=>test.chatStartWaiters.push(resolve));
      return structuredClone(run);
    }
    case 'cancel_chat_turn': {const run=Object.values(test.chat.runs).find(item=>item.id===args.runId);if(!run)throw new Error('Synthetic chat run not found.');await test.emitChatRun({conversationId:run.conversationId,status:'cancelled',statusMessage:'Interrompido no teste.'});return null;}
    case 'build_chat_context': return {conversationId:args.conversationId,messages:[],summary:'Contexto sintético'};
    case 'choose_import_file': return 'X:\\\\fixture\\\\synthetic-position.json';
    case 'import_broker_position':
      if (test.deferImports) await new Promise(resolve=>test.importWaiters.push(resolve));
      if (test.failImports-- > 0) throw new Error('Arquivo sintético inválido.');
      test.snapshot.brokerPosition = ${JSON.stringify(position)};
      test.snapshot.brokerPosition.overview.cash = '15';
      test.snapshot.brokerPosition.overview.totalWithCash = '206';
      return structuredClone(test.snapshot.brokerPosition);
    case 'export_portfolio_file': return 'X:\\\\fixture\\\\synthetic-backup.json';
    default: test.unexpected.push(command); throw new Error('Unexpected UI command: ' + command);
  }
}, {shouldMockEvents: true});
const mockInvoke=globalThis.__TAURI_INTERNALS__.invoke;
globalThis.__TAURI_INTERNALS__.invoke=async(command,args={},options)=>{
  const test=globalThis.__uiTest;
  if(command.startsWith('plugin:event|'))test.eventCalls.push({command,args:structuredClone(args)});
  // The installed mock expects id, while the public event API passes eventId.
  const normalized=command==='plugin:event|unlisten'?{...args,id:args.eventId??args.id}:args;
  const result=await mockInvoke(command,normalized,options);
  if(command==='plugin:event|listen')test.eventSubscriptions.push({event:args.event,id:result,active:true});
  if(command==='plugin:event|unlisten')for(const subscription of test.eventSubscriptions)if(subscription.event===args.event&&subscription.id===(args.eventId??args.id))subscription.active=false;
  return result;
};
mockWindows('main');
await import('/src/main.tsx?ui-original=${mode}');
`;
}

let server;
let browser;
await fs.mkdir(output, {recursive:true});
// Captures from earlier UI versions would be mistaken for the current interface.
if(!focusedRowMotion&&!focusedShell&&!focusedCompact&&!focusedPublicData&&!focusedProfile&&!focusedIncome&&!focusedIncomeApi&&!focusedIncomePerformance&&!focusedFixedIncome)for (const name of await fs.readdir(output)) if (/\.(png|json)$/.test(name)) await fs.rm(path.join(output, name));
async function run(){
try {
  server=await createServer({configFile:path.join(root,'vite.config.ts'),root,server:{port:1421,strictPort:true,host:'127.0.0.1',open:false},logLevel:'error',clearScreen:false});
  await server.listen();
  const base='http://127.0.0.1:1421';
  const channel=process.env.IVESTFII_BROWSER_CHANNEL||(process.platform==='win32'?'msedge':undefined);
  browser=await chromium.launch({headless:true,...(channel?{channel}:{})});
  const check=name=>{checks.push(name);console.log('PASS '+name);};

  // ---------- Page helpers ----------
  const banner=page=>page.getByRole('banner');
  const sidebar=page=>page.getByRole('complementary',{name:'Carteira e navegação',exact:true});
  const region=(page,name)=>page.getByRole('region',{name,exact:true});
  // Labels anchored at the start: definitions (Def) carry hidden text that a plain hasText would also match.
  const readout=(scope,label)=>scope.locator('.readout').filter({has:scope.page().locator('.figure-label',{hasText:new RegExp(`^${escapeRegExp(label)}`)})}).locator('.figure-value');
  const fact=(scope,label)=>scope.locator('dt',{hasText:new RegExp(`^${escapeRegExp(label)}`)}).locator('xpath=following-sibling::dd[1]');
  /** Text of the tooltip (Def) attached to a visible label; the explanations that used to be gray notes live there. */
  const tip=async(scope,label)=>{
    const page=scope.page();
    const anchor=scope.locator('.def',{hasText:new RegExp(`^${escapeRegExp(label)}$`)}).first();
    await anchor.hover();
    const bubble=page.getByRole('tooltip');await bubble.waitFor();
    const text=normalize(await bubble.innerText());
    await page.keyboard.press('Escape');await bubble.waitFor({state:'hidden'});await page.mouse.move(1,1);
    return text;
  };
  const tab=(page,name)=>page.getByRole('tab',{name,exact:true});
  const panel=(page,name)=>page.getByRole('tabpanel',{name,exact:true});
  async function openTab(page,name){await tab(page,name).click();assert.equal(await tab(page,name).getAttribute('aria-selected'),'true');await panel(page,name).waitFor();await openExtra(panel(page,name));}
  async function detailsClosed(page,label){
    await page.waitForFunction(()=>!document.querySelector('#main-content .loading'));
    assert.equal(await page.locator('details.extra-details[open]:visible').count(),0,`${label}: advanced information on the visible page should start collapsed`);
    if(await page.locator('details.extra-details:visible').count())check('details-default-closed-'+label);
  }
  async function openExtra(scope){
    const page=typeof scope.page==='function'?scope.page():scope;
    await page.waitForFunction(()=>!document.querySelector('#main-content .loading'));
    const details=scope.locator('details.extra-details');
    for(let index=0;index<await details.count();index++){
      const summary=details.nth(index).locator(':scope > summary');
      if(await summary.isVisible()&&!await details.nth(index).evaluate(element=>element.open))await summary.click();
    }
  }
  const toast=(page,text)=>page.getByRole('status').filter({hasText:text});
  /** Closes a confirmed notice so it does not cover the next screens' captures. */
  async function dismissToast(page,text){const notice=toast(page,text);await notice.getByRole('button',{name:'Fechar aviso',exact:true}).click();await notice.waitFor({state:'detached'});}
  const importDialog=page=>page.getByRole('dialog',{name:'Importar posição da corretora',exact:true});
  const calls=(page,command)=>page.evaluate(command=>globalThis.__uiTest.calls.filter(call=>call.command===command),command);
  const updateBlocked=(page,expected)=>page.waitForFunction(expected=>globalThis.__uiTest.updateActivityBlocked===expected,expected);
  const chatSubscriptions=page=>page.evaluate(()=>globalThis.__uiTest.eventSubscriptions.filter(item=>item.event==='chat-run'&&item.active).length);
  const chatListenCalls=page=>page.evaluate(()=>globalThis.__uiTest.eventCalls.filter(item=>item.command==='plugin:event|listen'&&item.args.event==='chat-run').length);
  const notificationBell=page=>page.getByRole('button',{name:'Notificações',exact:true});
  const notificationPanel=page=>page.getByRole('dialog',{name:'Notificações',exact:true});
  async function notificationCount(page,expected){
    await page.waitForFunction(expected=>{
      const button=document.querySelector('button[aria-label="Notificações"]');
      const description=button?.getAttribute('aria-describedby')?.split(/\s+/).map(id=>document.getElementById(id)?.textContent||'').join(' ')||'';
      return Number(description.match(/\d+/)?.[0]||0)===expected;
    },expected);
  }
  async function targetVisibleAndClickable(locator,label){
    await locator.waitFor({state:'visible'});
    const target=await locator.evaluate(element=>{
      const box=element.getBoundingClientRect(),hit=document.elementFromPoint(box.x+box.width/2,box.y+box.height/2);
      return {left:box.left,top:box.top,right:box.right,bottom:box.bottom,width:innerWidth,height:innerHeight,hit:!!hit&&(element===hit||element.contains(hit))};
    });
    assert.ok(target.left>=0&&target.top>=0&&target.right<=target.width+1&&target.bottom<=target.height+1,`${label}: target fits the visible viewport ${JSON.stringify(target)}`);
    assert.equal(target.hit,true,`${label}: another overlay must not cover the control`);
  }
  async function revealInViewport(locator){
    await locator.evaluate(element=>{element.scrollIntoView({block:'center',inline:'nearest',behavior:'instant'});return new Promise(resolve=>requestAnimationFrame(resolve));});
  }
  async function openPage(mode,viewport,{reducedMotion='reduce',expandedSidebar=true,sidebarPreference,touch=false}={}){
    const context=await browser.newContext({viewport,reducedMotion,hasTouch:touch,...(mode.includes('zoom-200')?{deviceScaleFactor:2}:{})});
    if(mode.startsWith('income'))await context.addInitScript(()=>{const NativeDate=Date;const fixed=NativeDate.parse('2026-10-20T12:00:00Z');globalThis.Date=class extends NativeDate{constructor(...args){super(...(args.length?args:[fixed]));}static now(){return fixed;}};});
    if(sidebarPreference!==undefined)await context.addInitScript(value=>localStorage.setItem('ivestfii.sidebarCollapsed',value),sidebarPreference);
    const page=await context.newPage();page.setDefaultTimeout(12000);
    const externalRequests=[];
    if(mode==='web'||mode.startsWith('income'))await page.route('**/*',route=>{
      const url=new URL(route.request().url());
      if(/^https?:$/.test(url.protocol)&&url.origin!==base){externalRequests.push(url.href);return route.fulfill({status:503,contentType:'text/plain',body:'External requests are blocked in the synthetic web preview.'});}
      return route.fallback();
    });
    page.on('pageerror',error=>browserErrors.push({mode,message:error.message}));
    page.on('console',message=>{if(message.type()==='error')browserErrors.push({mode,message:message.text()});});
    await page.route('**/src/main.tsx*',route=>new URL(route.request().url()).searchParams.has('ui-original')?route.continue():route.fulfill({status:200,contentType:'application/javascript',body:bootstrap(mode)}));
    // Vite may compile its first graph while the Windows release build occupies the machine.
    await page.goto(base,{waitUntil:'networkidle',timeout:60000});
    await title(page,'Visão geral');
    await detailsClosed(page,'Visão geral');
    assert.match(await page.locator('.app-shell').getAttribute('class'),/\bsidebar-collapsed\b/,'Every new opening starts compact, including an old expanded preference');
    if(expandedSidebar&&viewport.width>760){
      await page.getByRole('button',{name:'Expandir barra lateral',exact:true}).click();
      // Legacy form scenarios need the selector visible; restore the opening's keyboard starting point.
      await page.locator('body').evaluate(element=>{element.setAttribute('tabindex','-1');element.focus();element.removeAttribute('tabindex');});
    }
    return {context,page,externalRequests};
  }
  async function title(page,name){
    await banner(page).getByRole('heading',{level:1,name,exact:true}).waitFor();
  }
  /** Opens a screen from the sidebar (through the mobile menu when it is collapsed) and confirms the route changed. */
  async function navigate(page,name){
    const trigger=page.getByRole('button',{name:'Abrir menu',exact:true});
    if(await trigger.isVisible())await trigger.click();
    const item=sidebar(page).getByRole('button',{name,exact:true});
    await item.click();await title(page,name);
    assert.equal(await item.getAttribute('aria-current'),'page');
    assert.equal(await page.title(),`${name} · IvestFII`);
    assert.equal(await sidebar(page).locator('[aria-current="page"]').count(),1);
    await detailsClosed(page,name);await openExtra(page);
  }
  async function textEquals(locator,expected){await locator.waitFor({state:'visible'});assert.equal(normalize(await locator.innerText()),expected);}
  async function figureEquals(locator,expected){await locator.waitFor({state:'visible'});assert.equal(compact(await locator.innerText()),compact(expected));}
  async function textMatches(locator,pattern){await locator.waitFor({state:'visible'});assert.match(normalize(await locator.innerText()),pattern);}
  async function isFocused(locator){return locator.evaluate(element=>element===document.activeElement);}
  async function focusReturnsTo(page,locator){await page.waitForFunction(element=>element===document.activeElement,await locator.elementHandle());}
  async function noOverflow(page,label){
    const dimensions=await page.evaluate(()=>({viewport:innerWidth,document:document.documentElement.scrollWidth,body:document.body.scrollWidth}));
    const floatingSpace=await page.evaluate(()=>{
      const button=document.querySelector('.assistant-fab'),content=document.querySelector('.page-content');
      if(!button||!content||!button.getClientRects().length)return null;
      const box=button.getBoundingClientRect(),header=document.querySelector('.topbar')?.getBoundingClientRect();
      const inHeader=!!header&&box.top>=header.top&&box.bottom<=header.bottom;
      const headerOverlap=inHeader&&Array.from(document.querySelectorAll('.topbar :is(button,h1,input,select)')).some(element=>{
        const target=element.getBoundingClientRect();return target.width>1&&target.height>1&&box.left<target.right&&box.right>target.left&&box.top<target.bottom&&box.bottom>target.top;
      });
      return {contentRight:content.getBoundingClientRect().right,buttonLeft:box.left,inHeader,headerOverlap};
    });
    if(floatingSpace)assert.ok(floatingSpace.inHeader?!floatingSpace.headerOverlap:floatingSpace.contentRight<=floatingSpace.buttonLeft-8,`${label}: the floating assistant has its own space and cannot cover values or controls ${JSON.stringify(floatingSpace)}`);
    if(dimensions.document>dimensions.viewport+1||dimensions.body>dimensions.viewport+1){
      const offenders=await page.evaluate(()=>Array.from(document.querySelectorAll('body *')).filter(element=>element.getBoundingClientRect().right>innerWidth+1).filter(element=>{let ancestor=element.parentElement;while(ancestor&&ancestor!==document.body){if(['auto','scroll','hidden','clip'].includes(getComputedStyle(ancestor).overflowX)&&ancestor.getBoundingClientRect().right<=innerWidth+1)return false;ancestor=ancestor.parentElement;}return true;}).slice(0,18).map(element=>({tag:element.tagName,class:typeof element.className==='string'?element.className:'',right:Math.round(element.getBoundingClientRect().right),width:Math.round(element.getBoundingClientRect().width),text:element.textContent?.slice(0,90)})));
      await fs.writeFile(path.join(output,'overflow.json'),JSON.stringify({label,dimensions,offenders},null,2));await shot(page,label+'-failure');    }
    assert.ok(dimensions.document<=dimensions.viewport+1&&dimensions.body<=dimensions.viewport+1,`${label}: horizontal overflow ${JSON.stringify(dimensions)}`);check(label);
  }
  async function sidebarGeometry(page,label){
    const metrics=await sidebar(page).evaluate(element=>{
      const box=element.getBoundingClientRect();
      const controls=Array.from(element.querySelectorAll('.sidebar-header, .sidebar-header .brand, .sidebar-toggle, .compact-wallet, nav, .nav-item, .sidebar-bottom, .sidebar-status')).flatMap(control=>{
        const rect=control.getBoundingClientRect(),css=getComputedStyle(control);
        if(rect.width<1||rect.height<1||css.display==='none'||css.visibility==='hidden')return [];
        return [{name:control.getAttribute('aria-label')||control.className,left:rect.left,right:rect.right,width:rect.width}];
      });
      element.scrollLeft=999;const horizontalTravel=element.scrollLeft;element.scrollLeft=0;
      return {left:box.left,right:box.right,width:box.width,scrollWidth:element.scrollWidth,clientWidth:element.clientWidth,horizontalTravel,controls};
    });
    assert.ok(metrics.scrollWidth<=metrics.clientWidth,`${label}: the sidebar's content must fit, including the original one-pixel regression, even if overflow-x is hidden ${JSON.stringify(metrics)}`);
    assert.equal(metrics.horizontalTravel,0,`${label}: the sidebar must not contain a hidden horizontal scroll range`);
    for(const control of metrics.controls)assert.ok(control.left>=metrics.left-0.75&&control.right<=metrics.right+0.75,`${label}: ${control.name} extends beyond the actual sidebar ${JSON.stringify(control)}`);
    check(label+'-sidebar-content-fits-without-hidden-horizontal-scroll');return metrics;
  }
  async function contentGeometry(page,label){
    const metrics=await page.evaluate(()=>{
      const content=document.querySelector('.main-content'),body=document.querySelector('.page-content'),aside=document.querySelector('.sidebar');
      const outer=content.getBoundingClientRect(),inner=body.getBoundingClientRect();
      return {viewport:innerWidth,outerLeft:outer.left,outerRight:outer.right,innerLeft:inner.left,innerRight:inner.right,innerWidth:inner.width,maxWidth:getComputedStyle(content).maxWidth,sidebarWidth:aside.getBoundingClientRect().width};
    });
    const expectedRightGap=metrics.viewport>760?80:16;
    assert.ok(Math.abs(metrics.outerRight-metrics.viewport)<=1,`${label}: the main content reaches the viewport edge instead of stopping at an old width cap ${JSON.stringify(metrics)}`);
    assert.ok(Math.abs(metrics.viewport-metrics.innerRight-expectedRightGap)<=1,`${label}: only the necessary assistant/padding gutter remains ${JSON.stringify(metrics)}`);
    assert.ok(metrics.innerWidth>=metrics.viewport-metrics.innerLeft-expectedRightGap-1,`${label}: the available horizontal space is used`);
    check(label+'-content-uses-available-width-with-only-required-assistant-gutter');return metrics;
  }
  /** Row headers must paint as part of the body row, including stripes, hover and totals. */
  async function rowPaintContinuity(page,table,label,{capture=false}={}){
    await table.waitFor({state:'visible'});
    const rows=table.locator('tbody > tr');
    assert.ok(await rows.count()>=2,`${label}: exercise odd, even and final rows`);
    const settled=()=>page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
    const paint=row=>row.evaluate(element=>Array.from(element.children).map(cell=>{
      const css=getComputedStyle(cell),box=cell.getBoundingClientRect();
      const border=side=>Number.parseFloat(css.getPropertyValue(`border-${side}-width`))>0&&css.getPropertyValue(`border-${side}-style`)!=='none'
        ? [css.getPropertyValue(`border-${side}-width`),css.getPropertyValue(`border-${side}-style`),css.getPropertyValue(`border-${side}-color`)].join(' '):'none';
      return {tag:cell.tagName,scope:cell.getAttribute('scope'),background:css.backgroundColor,image:css.backgroundImage,topBorder:border('top'),bottomBorder:border('bottom'),padding:[css.paddingTop,css.paddingRight,css.paddingBottom,css.paddingLeft],align:css.verticalAlign,top:box.top,bottom:box.bottom,left:box.left,right:box.right};
    }));
    const continuous=(cells,state)=>{
      assert.equal(cells[0].tag,'TH',`${label} ${state}: keep the semantic row header`);
      assert.equal(cells[0].scope,'row');assert.ok(cells.slice(1).every(cell=>cell.tag==='TD'));
      for(let index=1;index<cells.length;index++){
        for(const property of ['background','image','topBorder','bottomBorder','padding','align'])
          assert.deepEqual(cells[0][property],cells[index][property],`${label} ${state}: th/td ${property} must be continuous`);
        assert.ok(Math.abs(cells[0].top-cells[index].top)<=0.75&&Math.abs(cells[0].bottom-cells[index].bottom)<=0.75,`${label} ${state}: cell edges must align`);
        assert.ok(Math.abs(cells[index-1].right-cells[index].left)<=0.75,`${label} ${state}: no gap between adjacent cells`);
      }
    };
    const normal=[];
    await page.mouse.move(1,1);await settled();
    for(let index=0;index<await rows.count();index++){
      const cells=await paint(rows.nth(index));continuous(cells,`normal row ${index+1}`);normal.push(cells);
    }
    assert.notEqual(normal[0][0].background,normal[1][0].background,`${label}: preserve the alternating row backgrounds`);
    assert.ok(normal.at(-1).every(cell=>cell.bottomBorder==='none'),`${label}: final row must not retain a partial bottom border`);
    if(await rows.last().evaluate(element=>element.classList.contains('performance-total')))
      assert.ok(normal.at(-1).every(cell=>cell.topBorder!=='none'),`${label}: the total keeps its complete top separator`);
    if(capture){const filename=label+'-normal.png';await table.screenshot({path:path.join(output,filename),animations:'disabled'});screenshots.push(filename);}
    for(let index=0;index<await rows.count();index++){
      const row=rows.nth(index),header=row.locator(':scope > th[scope="row"]');
      const size=await header.boundingBox();await header.hover({position:{x:3,y:Math.max(2,size.height-3)}});await settled();
      assert.equal(await row.evaluate(element=>element.matches(':hover')),true);
      const cells=await paint(row);continuous(cells,`hover row ${index+1}`);
      assert.notEqual(cells[0].background,normal[index][0].background,`${label}: hover must reach both the label and every value`);
      if(capture&&index===await rows.count()-1){const filename=label+'-hover-total.png';await table.screenshot({path:path.join(output,filename),animations:'disabled'});screenshots.push(filename);}
    }
    await page.mouse.move(1,1);await page.keyboard.press('Escape');await settled();
    check(label+'-normal-hover-and-final-row-continuity');
  }
  /** Sample all cells in the same animation frame, with real hover motion enabled. */
  async function rowBackgroundMotion(page,table,label){
    const rows=table.locator('tbody > tr'),evidence=[];
    for(const index of [...new Set([0,1,await rows.count()-1])]){
      const row=rows.nth(index);await row.scrollIntoViewIfNeeded();await page.mouse.move(1,1);
      await row.evaluate(async element=>{
        getComputedStyle(element.firstElementChild).backgroundColor;
        await Promise.all(element.getAnimations({subtree:true}).map(animation=>animation.finished.catch(()=>{})));
      });
      const initial=await row.evaluate(element=>{
        const cells=Array.from(element.children);
        const read=cell=>{const css=getComputedStyle(cell);return {background:css.backgroundColor,transition:[css.transitionProperty,css.transitionDuration,css.transitionTimingFunction,css.transitionDelay]};};
        return cells.map(read);
      });
      const descriptor=initial[0].transition;
      assert.equal(descriptor[0],'background-color');
      const duration=Number.parseFloat(descriptor[1])*(descriptor[1].endsWith('ms')?1:1000);
      assert.ok(duration>0,`${label}: exercise the actual background transition`);
      for(const cell of initial.slice(1))assert.deepEqual(cell.transition,descriptor,`${label}: th and td use the same background animation`);
      for(const direction of ['pointerenter','pointerleave']){
      const before=await row.evaluate(element=>getComputedStyle(element.firstElementChild).backgroundColor);
      await row.evaluate((element,{duration,direction})=>{
        globalThis.__rowMotionDone=new Promise((resolve,reject)=>{
          const timeout=setTimeout(()=>reject(new Error('The hover motion sample did not finish.')),3000);
          element.addEventListener(direction,()=>{
            const started=performance.now(),samples=[];
            const frame=now=>{
              samples.push({elapsed:now-started,cells:Array.from(element.children).map(cell=>{
                const css=getComputedStyle(cell),box=cell.getBoundingClientRect();
                return {background:css.backgroundColor,top:box.top,bottom:box.bottom,left:box.left,right:box.right,borderTop:css.borderTop,borderBottom:css.borderBottom};
              })});
              if(now-started<duration+100)requestAnimationFrame(frame);
              else{clearTimeout(timeout);resolve(samples);}
            };
            requestAnimationFrame(frame);
          },{once:true});
        });
      },{duration,direction});
      if(direction==='pointerenter'){
        const header=row.locator(':scope > th[scope="row"]'),box=await header.boundingBox();
        await header.hover({position:{x:3,y:Math.max(2,box.height-3)}});
      }else await page.mouse.move(1,1);
      const frames=await page.evaluate(()=>globalThis.__rowMotionDone);
      assert.ok(frames.length>=3,`${label}: collect multiple animation frames`);
      const final=frames.at(-1).cells[0].background;
      assert.notEqual(final,before,`${label}: ${direction} changes the row background`);
      assert.ok(frames.some(frame=>frame.cells[0].background!==before&&frame.cells[0].background!==final),`${label}: capture an intermediate color, not just the final state`);
      for(const [frameIndex,frame] of frames.entries())for(let cellIndex=1;cellIndex<frame.cells.length;cellIndex++){
        const first=frame.cells[0],cell=frame.cells[cellIndex];
        assert.equal(cell.background,first.background,`${label} row ${index+1} frame ${frameIndex}: no th/td background split while animating`);
        assert.ok(Math.abs(cell.top-first.top)<=0.75&&Math.abs(cell.bottom-first.bottom)<=0.75&&Math.abs(frame.cells[cellIndex-1].right-cell.left)<=0.75,`${label}: aligned edges throughout the transition`);
        // A zero-width border's computed color may inherit the text color. Only visible borders matter.
        for(const side of ['Top','Bottom']){
          const visible=value=>!value.startsWith('0px')&&!value.startsWith('0 ');
          if(visible(first['border'+side])||visible(cell['border'+side]))assert.equal(cell['border'+side],first['border'+side],`${label}: continuous border during hover`);
        }
      }
      evidence.push({row:index+1,direction,transition:descriptor,initialBackground:before,frames});
      }
    }
    await page.mouse.move(1,1);check(label+'-synchronized-background-transition-no-preference');return evidence;
  }
  async function shot(page,name,{fullPage=true,preserveFocus=false,collapsed=true}={}){
    // Screenshots show the less dense default view; detailed assertions above still inspect every moved value.
    const opened=collapsed?await page.evaluate(()=>Array.from(document.querySelectorAll('details.extra-details')).flatMap((element,index)=>{if(!element.open)return[];element.open=false;return[index];})):[];
    await page.evaluate(preserveFocus=>{if(!preserveFocus&&document.activeElement instanceof HTMLElement&&!document.activeElement.closest('dialog[open]'))document.activeElement.blur();window.scrollTo({top:0,left:0,behavior:'instant'});return new Promise(resolve=>requestAnimationFrame(resolve));},preserveFocus);
    const filename=name+'.png';
    await page.screenshot({path:path.join(output,filename),fullPage,animations:'disabled'});screenshots.push(filename);
    await page.evaluate(opened=>{const details=document.querySelectorAll('details.extra-details');for(const index of opened)if(details[index])details[index].open=true;},opened);
  }
  async function noUnexpectedCommands(page){assert.deepEqual(await page.evaluate(()=>globalThis.__uiTest.unexpected),[]);}
  async function verifyImportNotice(page,broker,originals,{capture=false}={}){
    const notice=broker.locator('.cp-import-notice');
    assert.equal(await notice.count(),1,'Import notes should have one compact entry point');
    assert.equal(await notice.locator('li').count(),0,'Original paragraphs should not crowd the main view');
    const body=await broker.textContent();for(const original of originals)assert.equal(body.includes(original),false,'Original notes belong inside their notification');
    const trigger=notice.getByRole('button',{name:/^Ver observações \(\d+\)$/});
    await trigger.click();
    const dialog=page.getByRole('dialog',{name:'Observações da importação',exact:true});await dialog.waitFor();
    assert.equal(await dialog.locator('.cp-original-notes[open]').count(),0);
    const affected=await dialog.locator('.cp-affected-value').allInnerTexts();
    assert.ok(affected.some(value=>value.includes('139,00')&&value.includes('140,00')),'The notice should identify both affected fund values');
    if(capture)await shot(page,typeof capture==='string'?capture:'desktop-import-notification',{fullPage:false});
    await openExtra(dialog);
    assert.deepEqual((await dialog.locator('.cp-original-notes li').allTextContents()).sort(),[...originals].sort(),'Every original note must remain available verbatim, including repeated topics and line breaks');
    await page.keyboard.press('Escape');await dialog.waitFor({state:'detached'});await focusReturnsTo(page,trigger);
    assert.equal(await broker.locator('.cp-original-notes').count(),0,'Original notes should leave the main DOM when the notification closes');
    await trigger.click();await dialog.waitFor();
    const funds=dialog.locator('tbody tr').filter({has:page.locator('.cp-affected-value',{hasText:/Resumo: R\$\s*139,00/})});
    await funds.getByRole('button',{name:'Ver no quadro',exact:true}).click();await dialog.waitFor({state:'detached'});
    await page.waitForFunction(()=>document.activeElement?.classList.contains('cp-field-target'));
    assert.match(await page.evaluate(()=>document.activeElement.textContent),/139,00|140,00/,'The notice must lead to its affected value');
  }

  /** Imported position (patrimônio) and purchase history (custo) stay in separate groups with separate totals. */
  async function verifyOverview(page){
    const broker=region(page,'Posição na corretora');
    await textMatches(broker.locator('.cp-position-note'),/Retrato da corretora.*data.*não.*ao vivo/i);
    assert.match(await tip(broker,'Posição na corretora'),/Histórico.*compras.*rendimentos.*não alteram.*posição/i);
    await figureEquals(broker.locator('.figure-hero'),'R$ 201,00');
    assert.match(await tip(broker,'Patrimônio total'),/investimentos.*saldo.*(?:já.*incluído|R\$ 10,00)/i);
    await textMatches(broker.locator('.cp-source'),/^Posição Corretora de teste · ref\. 03\/02\/2020, atualizada às 02:00$/);
    const composition=broker.getByRole('table',{name:/^Composição do patrimônio em 03\/02\/2020/});
    for(const [label,value] of [['Fundos listados','R$ 139,00'],['Pós-fixado','R$ 50,00'],['Saldo em conta','R$ 10,00'],['Proventos a receber','R$ 2,00']])
      assert.match(normalize(await composition.getByRole('row',{name:new RegExp(`^${label}`)}).innerText()),new RegExp(`^${label} ${escapeRegExp(value)}\\b`));
    const currencyNumber=text=>Number(text.replace(/[^\d,-]/g,'').replace(',','.'));
    const componentValues=await composition.locator('tbody tr td:nth-child(2)').allInnerTexts();
    assert.equal(Math.round(componentValues.reduce((sum,value)=>sum+currencyNumber(value),0)*100),Math.round(currencyNumber(await broker.locator('.figure-hero').innerText())*100),'Displayed components must equal total wealth, including cash once');
    const history=region(page,'Histórico de FIIs');
    await openExtra(history);
    await textEquals(history.locator('.group-head p'),'Não somado ao patrimônio');
    assert.match(await tip(history,'Não somado ao patrimônio'),/proventos acumulados.*B3.*não são somados novamente ao patrimônio importado/i);
    assert.equal(await history.locator('.figure-note, .block-foot').count(),0,'Overview history should keep explanations in tooltips');
    const result=region(page,'Resultado da carteira');
    await figureEquals(readout(result,'Total aplicado'),'R$ 2.240,00');
    await figureEquals(readout(result,'Rendimentos acumulados'),'+R$ 45,56');
    await figureEquals(readout(result,'Resultado da carteira'),'−R$ 854,54');
    await textEquals(result.getByRole('table',{name:'Composição do resultado total dos FIIs',exact:true}).getByRole('row',{name:/^= Resultado total/}).locator('td'),'−R$ 889,10');
    const funds=history.getByRole('table',{name:/^Fundos do histórico, do maior/});
    assert.equal(await funds.evaluate(element=>!!element.closest('details')),false,'Main fund values must remain visible without an accordion');
    assert.match(await tip(funds,'Parte do valor comprado'),/200.*1\.000.*20%/);
    assert.equal(await funds.locator('tbody tr').count(),2);
    assert.match(normalize(await funds.locator('tbody tr').first().innerText()),/^TEST11\b.* 10 R\$ 1\.000,00 /);
    assert.deepEqual(await funds.locator('tbody td.num').evaluateAll(cells=>cells.filter(cell=>cell.scrollWidth>cell.clientWidth+1).map(cell=>cell.textContent)),[],'Financial values must remain readable without ellipsis');
    assert.ok(Math.abs(await funds.locator('tbody tr .share-fill').first().evaluate(element=>parseFloat(element.style.width))-1000/1040*100)<0.01,'Purchase share bar must represent its percentage of all purchases');
    const recentPurchases=history.locator('.ov-recent-purchases tbody tr'),recentIncome=history.locator('.ov-recent-income tbody tr');
    assert.equal(await recentPurchases.count(),2);assert.equal(await recentIncome.count(),2);
    assert.equal(await recentPurchases.first().evaluate(element=>!!element.closest('details')),false,'Recent purchases must be visible by default');
    assert.equal(await recentIncome.first().evaluate(element=>!!element.closest('details')),false,'Recent income must be visible by default');
    const testPurchase=recentPurchases.filter({has:page.locator('.ticker',{hasText:/^TEST11$/})}),testIncome=recentIncome.filter({has:page.locator('.ticker',{hasText:/^TEST11$/})});
    await textMatches(testPurchase.locator('.ov-purchase'),/^R\$ 1\.000,00 10 cotas × R\$ 100,00$/);
    await textEquals(testIncome.locator('.ov-received'),'R$ 10,00');
  }
  async function verifyWalletTotals(page){
    await openExtra(page);
    const table=page.getByRole('table',{name:'Custo dos FIIs e saldo das caixinhas por carteira',exact:true});
    await textMatches(table.getByRole('row',{name:/^Carteira de teste/}),/^Carteira de teste · aberta Corretora de teste R\$ 1\.040,00 R\$ 1\.234,56 Abrir$/);
    await textMatches(table.getByRole('row',{name:/^Carteira Nubank teste/}),/^Carteira Nubank teste Nubank teste R\$ 200,00 R\$ 300,00 Abrir$/);
    await textMatches(table.locator('tfoot tr'),/^Total R\$ 1\.240,00 R\$ 1\.534,56$/);
    assert.match(await tip(page.locator('.wallets-overview'),'Carteiras'),/A posição importada não entra nestes totais\.$/);
  }
  async function releaseAutomatic(page,walletId='wallet-original'){
    await page.waitForFunction(walletId=>globalThis.__uiTest.autoWaiters.some(item=>item.walletId===walletId),walletId);
    await page.evaluate(walletId=>{
      const test=globalThis.__uiTest;
      test.deferAutoWallets=test.deferAutoWallets.filter(id=>id!==walletId);
      const waiters=test.autoWaiters.filter(item=>item.walletId===walletId);
      test.autoWaiters=test.autoWaiters.filter(item=>item.walletId!==walletId);
      waiters.forEach(item=>item.resolve());
    },walletId);
  }
  async function waitFigure(locator,expected){
    await locator.waitFor({state:'visible'});
    await locator.page().waitForFunction(({element,expected})=>element.textContent.replace(/\s+/g,'')===expected.replace(/\s+/g,''),{element:await locator.elementHandle(),expected});
    await figureEquals(locator,expected);
  }
  async function automaticCount(page,count){
    await page.waitForFunction(count=>globalThis.__uiTest.calls.filter(call=>call.command==='refresh_public_data_if_due').length===count,count);
  }
  async function noIndividualAutomaticRefresh(page){
    for(const command of ['refresh_quotes','refresh_cvm_data','refresh_economic_indicators','refresh_news','refresh_fii_performance'])
      assert.equal((await calls(page,command)).length,0,`${command}: automatic refresh must use one native coordinator and cached performance reads`);
  }
  async function sectionShot(page,name,scope){
    // Element screenshots scroll the section beneath a sticky mobile header.
    // Capture its document rectangle from the top so the crop shows the real
    // section, without moving a fixed header on top of its first indicator.
    await page.evaluate(()=>{window.scrollTo({top:0,left:0,behavior:'instant'});return new Promise(resolve=>requestAnimationFrame(resolve));});
    const clip=await scope.boundingBox();
    const session=await page.context().newCDPSession(page);
    try{
      const captured=await session.send('Page.captureScreenshot',{format:'png',fromSurface:true,captureBeyondViewport:true,clip:{...clip,scale:1}});
      await fs.writeFile(path.join(output,name+'.png'),Buffer.from(captured.data,'base64'));screenshots.push(name+'.png');
    }finally{await session.detach();}
  }
  async function verifyInvestmentProfiles(){
    const profileSection=page=>region(page,'Perfil de investimento');
    const form=scope=>scope.getByRole('form',{name:'Perfil de investimento',exact:true});
    const analysis=page=>page.locator('.assistant-page:visible .chat-analysis-bar');
    const draft=page=>page.locator('#chat-draft');
    const openProfileAssistant=async page=>{
      // navigate() opened these advanced panels for earlier assertions. The
      // persistent chat keeps them mounted; restore this fixture's clean view
      // before asking the generic route helper to check its collapsed default.
      await page.locator('.assistant-page details.extra-details[open]').evaluateAll(elements=>elements.forEach(element=>{element.open=false;}));
      await navigate(page,'Assistente');
    };
    const noAnalysis=async(page,label)=>assert.equal((await calls(page,'start_chat_turn')).length,0,`${label}: only an explicit chat submission may consume AI`);
    const fixture=await openPage('profile',{width:1440,height:1000}),page=fixture.page;
    await noAnalysis(page,'startup');
    await openProfileAssistant(page);
    await analysis(page).getByText('Perfil não preenchido.',{exact:true}).waitFor();
    await analysis(page).getByRole('button',{name:'Analisar carteira',exact:true}).waitFor();
    await textMatches(analysis(page).locator('.chat-analysis-data'),/Cotações de 19\/10\/2026.*CDI de 03\/02\/2020.*CVM jan\/20/);
    assert.doesNotMatch(await analysis(page).locator('.chat-analysis-data').innerText(),/datas diferentes|not-a-date|20\/10\/2026/,'Invalid market timestamps do not enter the available dates, and a 00:30Z trade belongs to the prior BRT day');
    await page.getByRole('combobox',{name:'Provedor de IA',exact:true}).selectOption('claude');
    await page.getByRole('combobox',{name:'Provedor de IA',exact:true}).selectOption('antigravity');
    await noAnalysis(page,'opening assistant and switching providers');
    check('profile-unset-is-explicit-no-risk-or-reserve-assumed-and-dates-use-BRT-observations');

    const complete=analysis(page).getByRole('button',{name:'Completar perfil',exact:true});await complete.click();
    const dialog=page.getByRole('dialog',{name:'Perfil de investimento',exact:true});await dialog.waitFor();
    const editor=form(dialog);await editor.waitFor();
    assert.equal(await editor.getByRole('combobox',{name:'Perfil de risco',exact:true}).inputValue(),'unknown');
    assert.equal(await editor.getByRole('combobox',{name:'Reserva de emergência',exact:true}).inputValue(),'unknown');
    for(const label of ['Objetivo de curto prazo','Prazo de curto prazo (meses)','Objetivo de longo prazo','Prazo de longo prazo (meses)','Quando você pode precisar do dinheiro?','Quanto pretende investir por mês (R$)','Experiência com investimentos'])
      assert.equal(await editor.getByLabel(new RegExp('^'+escapeRegExp(label))).inputValue(),'');
    assert.equal(await editor.locator('details.extra-details').evaluate(element=>element.open),false);
    assert.equal(await editor.getByRole('button',{name:'Salvar perfil',exact:true}).isDisabled(),true);
    assert.equal((await calls(page,'save_investment_profile')).length,0);await noAnalysis(page,'opening empty profile');
    check('profile-empty-form-preserves-unknown-null-and-optional-fields-without-default-financial-assumptions');

    const months=editor.getByLabel('Prazo de curto prazo (meses)',{exact:true});
    for(const [value,flag] of [['0','rangeUnderflow'],['1.5','stepMismatch'],['1201','rangeOverflow']]){
      await months.fill(value);await editor.getByRole('button',{name:'Salvar perfil',exact:true}).click();
      assert.equal(await months.evaluate((element,flag)=>element.validity[flag],flag),true);
      assert.equal((await calls(page,'save_investment_profile')).length,0,'Invalid month bounds must be rejected before sending IPC');
    }
    await months.fill('');await openExtra(editor);
    const contribution=editor.getByLabel('Quanto pretende investir por mês (R$)',{exact:true});await contribution.fill('-1');
    await page.evaluate(()=>{globalThis.__uiTest.profileSaveError='O aporte mensal deve ser um valor não negativo.';});
    await editor.getByRole('button',{name:'Salvar perfil',exact:true}).click();
    await editor.getByRole('alert').filter({hasText:'valor não negativo'}).waitFor();
    assert.equal(await contribution.inputValue(),'-1');assert.equal(await page.evaluate(()=>globalThis.__uiTest.profiles['wallet-original']),null);
    await noAnalysis(page,'failed validation');
    await contribution.fill('');await page.evaluate(()=>{globalThis.__uiTest.profileSaveError=null;});
    check('profile-form-validates-month-range-and-step-and-keeps-draft-after-native-validation-error');

    const partialNote='Nota privada parcial sintética A.';
    await editor.getByRole('textbox',{name:'Outras informações',exact:true}).fill(partialNote);
    await editor.getByRole('button',{name:'Salvar perfil',exact:true}).click();await dialog.waitFor({state:'detached'});
    const partialSave=(await calls(page,'save_investment_profile')).at(-1);
    assert.deepEqual(partialSave.args,{walletId:'wallet-original',input:{...blankProfile,notes:partialNote}});
    await analysis(page).getByText('Perfil salvo · risco não informado',{exact:true}).waitFor();
    await noAnalysis(page,'partial profile save');
    check('profile-partial-save-keeps-unanswered-risk-reserve-and-null-amount-horizons-and-knowledge');

    await navigate(page,'Configurações');const settingsForm=()=>form(profileSection(page));
    await settingsForm().getByRole('button',{name:'Salvar perfil',exact:true}).waitFor();
    assert.equal(await settingsForm().getByRole('textbox',{name:'Outras informações',exact:true}).inputValue(),partialNote);
    assert.equal(await settingsForm().getByRole('combobox',{name:'Perfil de risco',exact:true}).inputValue(),'unknown');
    const privateGoal='Objetivo privado de longo prazo da carteira A.';
    await settingsForm().getByRole('combobox',{name:'Perfil de risco',exact:true}).selectOption('moderate');
    await settingsForm().getByRole('combobox',{name:'Reserva de emergência',exact:true}).selectOption('partial');
    await settingsForm().getByLabel('Quando você pode precisar do dinheiro?',{exact:true}).fill('Liquidez privada sintética A.');
    await settingsForm().getByLabel('Objetivo de longo prazo',{exact:true}).fill(privateGoal);
    await settingsForm().getByLabel('Prazo de longo prazo (meses)',{exact:true}).fill('120');
    await settingsForm().getByLabel('Quanto pretende investir por mês (R$)',{exact:true}).fill('0');
    await settingsForm().getByRole('combobox',{name:'Experiência com investimentos',exact:true}).selectOption('beginner');
    await page.evaluate(()=>{globalThis.__uiTest.deferProfileSaves=true;});
    const beforeBusy=(await calls(page,'save_investment_profile')).length;
    await settingsForm().getByRole('button',{name:'Salvar perfil',exact:true}).evaluate(button=>{button.click();button.click();});
    await page.waitForFunction(()=>globalThis.__uiTest.profileSaveWaiters.length===1);
    assert.equal((await calls(page,'save_investment_profile')).length,beforeBusy+1);
    assert.equal(await settingsForm().getAttribute('aria-busy'),'true');
    assert.equal(await settingsForm().getByRole('combobox',{name:'Perfil de risco',exact:true}).isDisabled(),true);
    assert.equal(await settingsForm().getByRole('button',{name:'Salvando…',exact:true}).isDisabled(),true);
    const busyInput=(await calls(page,'save_investment_profile')).at(-1).args;
    assert.equal(busyInput.walletId,'wallet-original');assert.equal(busyInput.input.monthlyContribution,'0');assert.equal(busyInput.input.shortTermMonths,null);assert.equal(busyInput.input.longTermMonths,120);
    await page.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-other');
    await settingsForm().getByRole('combobox',{name:'Perfil de risco',exact:true}).waitFor();
    assert.equal(await settingsForm().getByRole('combobox',{name:'Perfil de risco',exact:true}).inputValue(),'conservative');
    await page.evaluate(()=>{const test=globalThis.__uiTest;test.deferProfileSaves=false;test.profileSaveWaiters.splice(0).forEach(item=>item.resolve());});
    await page.waitForFunction(()=>globalThis.__uiTest.profiles['wallet-original']?.monthlyContribution==='0');
    assert.equal(await settingsForm().getByLabel('Objetivo de longo prazo',{exact:true}).inputValue(),otherProfile.longTermObjective);
    assert.equal(await settingsForm().getByRole('combobox',{name:'Perfil de risco',exact:true}).inputValue(),'conservative');
    await noAnalysis(page,'save and wallet switch');
    check('profile-save-is-serialized-explicit-zero-is-known-and-late-save-cannot-overwrite-other-wallet');

    await page.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-original');
    await settingsForm().getByRole('combobox',{name:'Perfil de risco',exact:true}).waitFor();
    assert.equal(await settingsForm().getByRole('combobox',{name:'Perfil de risco',exact:true}).inputValue(),'moderate');
    assert.equal(await settingsForm().getByLabel('Objetivo de longo prazo',{exact:true}).inputValue(),privateGoal);
    assert.equal(await settingsForm().getByLabel('Quanto pretende investir por mês (R$)',{exact:true}).inputValue(),'0');
    assert.equal((await calls(page,'get_investment_profile')).filter(call=>call.args.walletId==='wallet-original').length,1,'StrictMode and Settings/Assistant share one profile read per wallet');
    await noOverflow(page,'investment-profile-desktop-settings-no-overflow');await sectionShot(page,'desktop-investment-profile',profileSection(page));
    await openProfileAssistant(page);await analysis(page).getByText('Perfil salvo · moderado',{exact:true}).waitFor();
    await analysis(page).getByRole('button',{name:'Editar perfil',exact:true}).click();await dialog.waitFor();
    assert.equal(await form(dialog).getByRole('combobox',{name:'Perfil de risco',exact:true}).inputValue(),'moderate');
    await openExtra(form(dialog));const assistantNote='Nota privada alterada no assistente A.';
    await form(dialog).getByRole('textbox',{name:'Outras informações',exact:true}).fill(assistantNote);
    await form(dialog).getByRole('button',{name:'Salvar perfil',exact:true}).click();await dialog.waitFor({state:'detached'});
    await navigate(page,'Configurações');assert.equal(await settingsForm().getByRole('textbox',{name:'Outras informações',exact:true}).inputValue(),assistantNote);
    await noAnalysis(page,'Settings and Assistant profile edits');
    const storage=await page.evaluate(()=>JSON.stringify({...localStorage}));
    for(const value of [privateGoal,partialNote,assistantNote,otherProfile.notes,'Liquidez privada sintética A.'])assert.equal(storage.includes(value),false,'Profile fields must not be copied into browser storage');
    check('profile-settings-and-assistant-edit-the-same-wallet-state-without-browser-storage-or-automatic-AI');

    await openProfileAssistant(page);const keptDraft='Rascunho privado que não deve virar a análise automática.';
    await draft(page).fill(keptDraft);await page.getByRole('combobox',{name:'Provedor de IA',exact:true}).selectOption('claude');
    await noAnalysis(page,'draft and provider selection');
    const previousMessages=await page.evaluate(()=>structuredClone(globalThis.__uiTest.chat.messages));
    const analyze=analysis(page).getByRole('button',{name:'Analisar carteira',exact:true});
    await analyze.evaluate(button=>{button.click();button.click();});
    await page.getByRole('button',{name:'Interromper',exact:true}).waitFor();
    const started=(await calls(page,'start_chat_turn'));assert.equal(started.length,1);
    const first=started[0].args;
    assert.equal(first.walletId,'wallet-original');assert.equal(first.provider,'claude');assert.equal(first.conversationId,'conversation-a');
    const humanRequest='Analise a carteira selecionada usando meu perfil e os dados mais recentes disponíveis. Mostre melhorias para meus objetivos de curto e longo prazo, com motivos, riscos e o que ainda preciso informar.';
    assert.equal(first.content,humanRequest,'The canonical user message is readable Portuguese; native provider policy handles the MCP tool contract');
    assert.doesNotMatch(first.content,/wallet-original|walletId|refreshPublicDataIfDue|preparar_analise|investorProfile|MCP|JSON|\{|\}/,'Internal tool arguments and IDs must stay out of the visible user message');
    for(const value of [privateGoal,assistantNote,keptDraft,'Liquidez privada sintética A.'])assert.equal(first.content.includes(value),false,'The canonical request must obtain profile details from MCP without copying form data or consuming the pending draft');
    assert.equal(await draft(page).inputValue(),keptDraft);assert.equal(await analyze.isDisabled(),true);
    const persisted=await page.evaluate(content=>globalThis.__uiTest.chat.messages.filter(message=>message.role==='user'&&message.content===content),first.content);assert.equal(persisted.length,1);
    assert.deepEqual(await page.evaluate(count=>globalThis.__uiTest.chat.messages.slice(0,count),previousMessages.length),previousMessages);
    check('profile-explicit-analysis-starts-one-human-user-turn-with-current-wallet-and-keeps-existing-draft');
    await page.evaluate(()=>globalThis.__uiTest.emitChatRun({status:'completed',text:'Análise sintética solicitada da carteira A.'}));
    await page.getByText('Análise sintética solicitada da carteira A.',{exact:true}).waitFor();
    await page.getByRole('combobox',{name:'Provedor de IA',exact:true}).selectOption('antigravity');
    assert.equal((await calls(page,'start_chat_turn')).length,1,'Provider changes never start another analysis');
    await page.getByText('Resposta sintética preservada entre provedores.',{exact:true}).waitFor();
    assert.equal(await draft(page).inputValue(),keptDraft);
    await noOverflow(page,'profile-aware-assistant-desktop-no-overflow');await shot(page,'desktop-profile-aware-assistant',{fullPage:false});

    await page.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-other');
    await analysis(page).getByText('Perfil salvo · conservador',{exact:true}).waitFor();
    assert.equal(await draft(page).inputValue(),keptDraft);assert.equal((await calls(page,'start_chat_turn')).length,1);
    await analysis(page).getByRole('button',{name:'Analisar carteira',exact:true}).click();await page.getByRole('button',{name:'Interromper',exact:true}).waitFor();
    const second=(await calls(page,'start_chat_turn')).at(-1).args;
    assert.equal(second.walletId,'wallet-other');assert.equal(second.provider,'antigravity');assert.equal(second.conversationId,first.conversationId);
    assert.equal(second.content,humanRequest);assert.equal((await calls(page,'start_chat_turn')).length,2);
    assert.equal(await page.evaluate(()=>globalThis.__uiTest.chat.messages.filter(message=>message.role==='user').length),3,'The prior question and one canonical question per requested analysis must remain in the same history');
    await page.evaluate(()=>globalThis.__uiTest.emitChatRun({status:'completed',text:'Análise sintética solicitada da carteira B.'}));
    await page.getByText('Análise sintética solicitada da carteira B.',{exact:true}).waitFor();
    await noUnexpectedCommands(page);await fixture.context.close();
    check('profile-wallet-and-provider-switch-keep-one-chat-history-and-use-the-new-wallet-only-on-explicit-analysis');

    // A delayed read belongs to the wallet that requested it, even when the
    // settings screen and chat start observing a different wallet meanwhile.
    const stale=await openPage('profile-stale',{width:1440,height:1000}),stalePage=stale.page;
    await sidebar(stalePage).getByRole('button',{name:'Configurações',exact:true}).click();await title(stalePage,'Configurações');
    await stalePage.waitForFunction(()=>globalThis.__uiTest.profileReadWaiters.length===1);
    await stalePage.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-other');
    const staleForm=()=>form(profileSection(stalePage));await staleForm().getByRole('combobox',{name:'Perfil de risco',exact:true}).waitFor();
    assert.equal(await staleForm().getByRole('combobox',{name:'Perfil de risco',exact:true}).inputValue(),'conservative');
    await stalePage.evaluate(()=>{const test=globalThis.__uiTest;test.deferProfileWallets=[];test.profileReadWaiters.splice(0).forEach(item=>item.resolve());});
    await navigate(stalePage,'Assistente');await analysis(stalePage).getByText('Perfil salvo · conservador',{exact:true}).waitFor();
    await stalePage.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-original');
    await analysis(stalePage).getByText('Perfil salvo · arrojado',{exact:true}).waitFor();
    assert.equal((await calls(stalePage,'get_investment_profile')).length,2);await noAnalysis(stalePage,'late profile reads');
    await noUnexpectedCommands(stalePage);await stale.context.close();
    check('profile-late-wallet-read-is-isolated-and-returning-to-each-wallet-reuses-its-own-profile');

    const failed=await openPage('profile-error',{width:1440,height:1000}),failedPage=failed.page;
    await navigate(failedPage,'Assistente');await analysis(failedPage).getByText('Perfil indisponível.',{exact:true}).waitFor();
    assert.equal(await analysis(failedPage).getByRole('button',{name:'Analisar carteira',exact:true}).isDisabled(),true);
    await noAnalysis(failedPage,'unavailable profile');
    await failedPage.evaluate(()=>{globalThis.__uiTest.profileReadError=null;});
    await analysis(failedPage).getByRole('button',{name:'Tentar ler perfil',exact:true}).click();
    await analysis(failedPage).getByText('Perfil não preenchido.',{exact:true}).waitFor();
    assert.equal(await analysis(failedPage).getByRole('button',{name:'Analisar carteira',exact:true}).isEnabled(),true);
    assert.equal((await calls(failedPage,'get_investment_profile')).length,2);await noAnalysis(failedPage,'profile read retry');
    await noUnexpectedCommands(failedPage);await failed.context.close();
    check('profile-read-error-blocks-analysis-until-an-explicit-read-retry-recovers-without-using-AI');

    const conflict=await openPage('profile-conflict',{width:1440,height:1000}),conflictPage=conflict.page;
    await navigate(conflictPage,'Configurações');const inline=()=>form(profileSection(conflictPage));
    const inlineNote=()=>inline().getByRole('textbox',{name:'Outras informações',exact:true});
    const openRemoteEditor=async()=>{
      await conflictPage.getByRole('button',{name:'Abrir assistente',exact:true}).click();
      const drawer=conflictPage.getByRole('dialog',{name:'Assistente lateral',exact:true});await drawer.waitFor();
      await drawer.locator('.chat-analysis-bar').getByRole('button',{name:/^(Completar|Editar) perfil$/}).click();
      const modal=conflictPage.getByRole('dialog',{name:'Perfil de investimento',exact:true});await modal.waitFor();await openExtra(form(modal));return {drawer,modal};
    };
    await inlineNote().fill('Rascunho local sintético em Configurações.');
    let remote=await openRemoteEditor();
    await form(remote.modal).getByRole('textbox',{name:'Outras informações',exact:true}).fill('Perfil remoto salvo pelo editor do assistente.');
    await form(remote.modal).getByRole('button',{name:'Salvar perfil',exact:true}).click();await remote.modal.waitFor({state:'detached'});
    await remote.drawer.getByRole('button',{name:'Fechar assistente',exact:true}).click();await remote.drawer.waitFor({state:'hidden'});
    await inline().locator('.profile-external-change').waitFor();
    await textMatches(inline().locator('.profile-external-change'),/perfil foi alterado em outra tela.*rascunho foi mantido.*Salvar meu rascunho/);
    assert.equal(await inlineNote().inputValue(),'Rascunho local sintético em Configurações.');
    await inline().getByRole('button',{name:'Salvar meu rascunho',exact:true}).waitFor();
    await sectionShot(conflictPage,'desktop-investment-profile-conflict',profileSection(conflictPage));
    await inline().getByRole('button',{name:'Carregar perfil salvo',exact:true}).click();
    assert.equal(await inlineNote().inputValue(),'Perfil remoto salvo pelo editor do assistente.');
    assert.equal(await inline().getByRole('button',{name:'Salvar perfil',exact:true}).isDisabled(),true);
    assert.equal(await inline().locator('.profile-external-change').count(),0);
    check('profile-dirty-inline-editor-keeps-its-draft-and-explicitly-reloads-a-profile-saved-in-the-drawer');

    remote=await openRemoteEditor();
    await form(remote.modal).getByRole('textbox',{name:'Outras informações',exact:true}).fill('Perfil remoto novo com formulário inline limpo.');
    await form(remote.modal).getByRole('button',{name:'Salvar perfil',exact:true}).click();await remote.modal.waitFor({state:'detached'});
    await remote.drawer.getByRole('button',{name:'Fechar assistente',exact:true}).click();await remote.drawer.waitFor({state:'hidden'});
    assert.equal(await inlineNote().inputValue(),'Perfil remoto novo com formulário inline limpo.');
    assert.equal(await inline().locator('.profile-external-change').count(),0);
    assert.equal(await inline().getByRole('button',{name:'Salvar perfil',exact:true}).isDisabled(),true);
    check('profile-clean-inline-editor-follows-a-save-in-another-editor-without-creating-unsaved-changes');

    await inlineNote().fill('Rascunho escolhido explicitamente para substituir o salvo.');
    remote=await openRemoteEditor();
    await form(remote.modal).getByRole('textbox',{name:'Outras informações',exact:true}).fill('Outra edição remota antes da substituição explícita.');
    await form(remote.modal).getByRole('button',{name:'Salvar perfil',exact:true}).click();await remote.modal.waitFor({state:'detached'});
    await remote.drawer.getByRole('button',{name:'Fechar assistente',exact:true}).click();await remote.drawer.waitFor({state:'hidden'});
    await inline().getByRole('button',{name:'Salvar meu rascunho',exact:true}).click();
    await inline().getByRole('status').filter({hasText:'Perfil salvo nesta carteira.'}).waitFor();
    assert.equal(await conflictPage.evaluate(()=>globalThis.__uiTest.profiles['wallet-original'].notes),'Rascunho escolhido explicitamente para substituir o salvo.');
    assert.equal(await inline().locator('.profile-external-change').count(),0);await noAnalysis(conflictPage,'concurrent profile editors');
    await noUnexpectedCommands(conflictPage);await conflict.context.close();
    check('profile-conflict-can-be-overwritten-only-by-explicit-save-my-draft-without-starting-AI');

    for(const viewport of [{width:375,height:812},{width:720,height:500}]){
      const mobile=await openPage('profile-mobile'+(viewport.width===720?'-zoom-200':''),viewport,{expandedSidebar:false}),mobilePage=mobile.page;
      await mobilePage.getByRole('button',{name:'Abrir assistente',exact:true}).click();
      const drawer=mobilePage.getByRole('dialog',{name:'Assistente lateral',exact:true});await drawer.waitFor();
      await drawer.locator('.chat-analysis-bar').getByText('Perfil não preenchido.',{exact:true}).waitFor();
      const stage=drawer.locator('.chat-stage');
      const metrics=await stage.evaluate(element=>({height:element.getBoundingClientRect().height,clientHeight:element.clientHeight,scrollHeight:element.scrollHeight}));
      assert.ok(metrics.height>=159,`The profile bar must leave a readable transcript at ${viewport.width}px; observed ${JSON.stringify(metrics)}`);
      await revealInViewport(drawer.getByRole('button',{name:'Analisar carteira',exact:true}));
      await targetVisibleAndClickable(drawer.getByRole('button',{name:'Analisar carteira',exact:true}),`analysis action ${viewport.width}`);
      await revealInViewport(drawer.getByRole('textbox',{name:'Sua pergunta',exact:true}));
      await targetVisibleAndClickable(drawer.getByRole('textbox',{name:'Sua pergunta',exact:true}),`profile-aware drawer draft ${viewport.width}`);
      await revealInViewport(drawer.getByRole('button',{name:'Enviar',exact:true}));
      await targetVisibleAndClickable(drawer.getByRole('button',{name:'Enviar',exact:true}),`profile-aware drawer send ${viewport.width}`);
      await noOverflow(mobilePage,`profile-aware-assistant-${viewport.width}-no-overflow`);await shot(mobilePage,`profile-aware-assistant-compose-${viewport.width}`,{fullPage:false});
      await stage.scrollIntoViewIfNeeded();await shot(mobilePage,`profile-aware-assistant-${viewport.width}`,{fullPage:false});
      check(`profile-aware-drawer-keeps-readable-transcript-and-scroll-access-to-analysis-editor-and-send-${viewport.width}`);
      await drawer.getByRole('button',{name:'Completar perfil',exact:true}).click();
      const modal=mobilePage.getByRole('dialog',{name:'Perfil de investimento',exact:true});await modal.waitFor();
      await form(modal).getByLabel('Objetivo de curto prazo',{exact:true}).fill('Objetivo parcial no celular sintético.');
      await noOverflow(mobilePage,`profile-editor-${viewport.width}-no-overflow`);await shot(mobilePage,`investment-profile-editor-${viewport.width}`,{fullPage:false});
      await form(modal).getByLabel('Objetivo de curto prazo',{exact:true}).focus();await mobilePage.mouse.move(2,2);await mobilePage.getByRole('tooltip').waitFor({state:'hidden'});
      await mobilePage.keyboard.press('Escape');await modal.waitFor({state:'detached'});assert.equal(await drawer.isVisible(),true,'Escape closes the profile editor before its containing assistant');
      assert.equal((await calls(mobilePage,'save_investment_profile')).length,0);await noAnalysis(mobilePage,'mobile form editing and cancellation');
      await noUnexpectedCommands(mobilePage);await mobile.context.close();
      check(`profile-editor-and-assistant-actions-remain-accessible-without-overlap-and-Escape-is-scoped-${viewport.width}`);
    }
  }
  async function verifyAutomaticPublicData(){
    const fixture=await openPage('auto-start',{width:1440,height:1000}),page=fixture.page;
    const result=()=>region(page,'Resultado da carteira');
    const summary=()=>region(page,'Mercado em resumo');
    await automaticCount(page,1);
    await page.waitForFunction(()=>globalThis.__uiTest.autoWaiters.length===1);
    await waitFigure(readout(result(),'Total aplicado'),'R$ 2.240,00');
    await figureEquals(readout(result(),'Rendimentos acumulados'),'+R$ 45,56');
    await figureEquals(readout(result(),'Resultado da carteira'),'−R$ 854,54');
    assert.equal(await summary().locator('.public-data-state').getAttribute('aria-busy'),'true');
    await textMatches(summary().locator('[data-indicator=cdi] .public-indicator-value'),/^8,20 % a\.a\.$/);
    const savedSummaryRow=summary().getByRole('row',{name:/^TEST11 /});
    await textMatches(savedSummaryRow,/^TEST11 R\$ 8,99/);
    assert.equal(await savedSummaryRow.locator('time').first().getAttribute('datetime'),'2020-02-03T18:00:00Z');
    const missingSummaryRow=summary().getByRole('row',{name:/^MOCK11 /});
    assert.deepEqual((await missingSummaryRow.locator('td').allInnerTexts()).map(normalize),['—','Não informada','—','—'],'Unknown quote/report values must not be displayed as zero');
    assert.equal(await summary().locator('.public-news li').count(),1);
    const original=await page.evaluate(()=>structuredClone(globalThis.__uiTest.snapshot));
    const initialPerformanceReads=(await calls(page,'get_fii_performance')).length;
    await navigate(page,'Mercado');
    const quotes=region(page,'Cotações dos fundos');
    await textMatches(quotes.locator('.market-quotes-table tbody tr').first(),/^TEST11.*R\$ 8,99/);
    await navigate(page,'Visão geral');
    assert.equal((await calls(page,'refresh_public_data_if_due')).length,1,'Navigation must share the still-running startup request');
    await noIndividualAutomaticRefresh(page);
    check('public-data-StrictMode-startup-single-call-cache-visible-during-refresh-and-navigation');

    await releaseAutomatic(page);
    await waitFigure(readout(result(),'Resultado da carteira'),'−R$ 851,94');
    await figureEquals(readout(result(),'Total aplicado'),'R$ 2.240,00');
    await figureEquals(readout(result(),'Rendimentos acumulados'),'+R$ 45,56');
    await textEquals(result().locator('.performance-table').getByRole('row',{name:/^= Resultado total/}).locator('td'),'−R$ 886,50');
    assert.ok((await calls(page,'get_fii_performance')).length>initialPerformanceReads,'Refreshed sources trigger a cached recalculation read');
    assert.deepEqual(await page.evaluate(()=>globalThis.__uiTest.snapshot),original,'Public data must not rewrite purchases, imported balances or cash reconciliation');
    await noIndividualAutomaticRefresh(page);
    check('public-data-refresh-updates-cached-FII-result-without-rewriting-financial-records');
    assert.equal(await summary().locator('.public-data-state').getAttribute('aria-busy'),null);
    assert.equal(await summary().locator('.public-data-state time').getAttribute('datetime'),'2026-10-20T12:00:00Z');
    await textMatches(summary().locator('[data-indicator=cdi] .public-indicator-value'),/^14,90 % a\.a\.$/);
    assert.deepEqual(await summary().locator('[data-indicator=cdi] time').evaluateAll(elements=>elements.map(element=>element.dateTime)),['2026-10-19','2026-10-20T12:00:00Z']);
    const summaryRow=summary().getByRole('row',{name:/^TEST11 /});
    await textMatches(summaryRow,/^TEST11 R\$ 9,25 .* R\$ 10,25 /);
    assert.deepEqual(await summaryRow.locator('time').evaluateAll(elements=>elements.map(element=>element.dateTime)),['2026-10-16T18:00:00Z','2026-09-30']);
    assert.equal(await summary().locator('.public-news li').count(),3,'The summary must stay limited to three headlines even when the cache has more');
    assert.deepEqual(await summary().locator('.public-news li time').evaluateAll(elements=>elements.map(element=>element.dateTime)),Array(3).fill('2026-10-19T12:00:00Z'));
    assert.equal(await summary().locator('.public-source-checks').evaluate(element=>element.open),false,'Source diagnostics start collapsed after the automatic response');
    for(const source of ['economics','news','quotes','cvm','cdiHistory'])assert.equal(await summary().locator(`[data-source=${source}]`).getAttribute('data-source-status'),'updated');
    assert.equal(await summary().locator('[data-indicator=cdi] a').getAttribute('href'),cdi.sourceUrl);
    const headline=summary().locator('.public-news li a').first();
    const href=await headline.getAttribute('href');await headline.click();
    assert.equal((await calls(page,'open_public_source')).at(-1).args.url,href,'A headline opens the exact source through the native safe opener');
    check('public-data-summary-separates-reference-trading-report-publication-and-collection-dates-and-limits-headlines');
    await noOverflow(page,'automatic-summary-desktop-no-overflow');await shot(page,'desktop-automatic-public-data');
    await sectionShot(page,'desktop-automatic-market-summary',summary());
    await page.setViewportSize({width:375,height:812});await noOverflow(page,'automatic-summary-375-no-overflow');await shot(page,'mobile-automatic-public-data375');
    await sectionShot(page,'mobile-automatic-market-summary375',summary());
    await page.setViewportSize({width:1440,height:1000});

    await navigate(page,'Mercado');
    const updatedQuote=region(page,'Cotações dos fundos').locator('.market-quotes-table tbody tr').first();
    await textMatches(updatedQuote,/^TEST11.*R\$ 9,25/);
    assert.equal(await updatedQuote.locator('time').getAttribute('datetime'),'2026-10-16T18:00:00Z','Market observation date must stay distinct from collection date');
    await navigate(page,'Visão geral');
    assert.equal((await calls(page,'refresh_public_data_if_due')).length,1,'Reopening views must reuse refreshed public data');
    await noIndividualAutomaticRefresh(page);
    check('public-data-navigation-reuses-refreshed-cache-without-new-external-query');

    // Only a new input set starts another coordinated attempt; rendering a new
    // snapshot object or opening a screen is insufficient.
    await navigate(page,'FIIs e posições');
    await banner(page).getByRole('button',{name:'Novo fundo',exact:true}).click();
    const newFund=page.getByRole('dialog',{name:'Novo fundo',exact:true});
    await newFund.getByLabel('Ticker').fill('NEW11');
    await newFund.getByLabel('Nome do fundo').fill('Fundo sintético de nova consulta');
    await newFund.getByRole('button',{name:'Salvar fundo',exact:true}).click();await newFund.waitFor({state:'detached'});
    await automaticCount(page,2);
    await navigate(page,'Visão geral');assert.equal((await calls(page,'refresh_public_data_if_due')).length,2);
    check('public-data-new-asset-input-triggers-one-additional-attempt');
    await page.evaluate(()=>window.dispatchEvent(new Event('focus')));
    await navigate(page,'Mercado');assert.equal((await calls(page,'refresh_public_data_if_due')).length,2,'A brief focus change must not query the sources again');
    await page.evaluate(()=>{const now=Date.now();Date.now=()=>now+31*60*1000;window.dispatchEvent(new Event('focus'));});
    await automaticCount(page,3);
    await navigate(page,'Visão geral');assert.equal((await calls(page,'refresh_public_data_if_due')).length,3);
    check('public-data-return-after-thirty-minutes-rechecks-due-sources-once-without-focus-loop');
    await navigate(page,'Configurações');
    await page.getByRole('button',{name:'Remover chave local',exact:true}).click();
    await page.getByText('Sem chave configurada',{exact:true}).waitFor();
    await automaticCount(page,4);
    await navigate(page,'Visão geral');
    await textMatches(summary().locator('.public-data-guidance'),/Cotações sem chave configurada.*brapi.*Configurações/);
    assert.equal(await summary().locator('[data-source=quotes]').getAttribute('data-source-status'),'not_configured');
    await textMatches(summary().getByRole('row',{name:/^TEST11 /}),/^TEST11 R\$ 9,25/);
    await waitFigure(readout(result(),'Resultado da carteira'),'−R$ 851,94');
    assert.equal((await calls(page,'refresh_public_data_if_due')).length,4,'Changing API configuration triggers one new coordinated attempt while preserving cached prices');
    await noIndividualAutomaticRefresh(page);
    check('public-data-configuration-change-rechecks-once-and-key-removal-preserves-cached-quotes-and-result');
    await noUnexpectedCommands(page);await fixture.context.close();

    const stale=await openPage('auto-stale',{width:1440,height:1000}),stalePage=stale.page;
    await automaticCount(stalePage,1);await stalePage.waitForFunction(()=>globalThis.__uiTest.autoWaiters.length===1);
    await stalePage.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-other');
    await automaticCount(stalePage,2);
    const otherResult=region(stalePage,'Resultado da carteira');
    await waitFigure(readout(otherResult,'Resultado da carteira'),'+R$ 60,00');
    await figureEquals(readout(otherResult,'Total aplicado'),'R$ 480,00');
    await figureEquals(readout(otherResult,'Rendimentos acumulados'),'+R$ 20,00');
    await releaseAutomatic(stalePage);
    // A subsequent view transition forces all pending React work through the
    // stale-response guards before checking the active wallet again.
    await navigate(stalePage,'Mercado');
    assert.deepEqual(await region(stalePage,'Cotações dos fundos').locator('.market-quotes-table tbody th').allInnerTexts(),['BANK11']);
    await navigate(stalePage,'Visão geral');
    await figureEquals(readout(otherResult,'Resultado da carteira'),'+R$ 60,00');
    await stalePage.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-original');
    await waitFigure(readout(otherResult,'Resultado da carteira'),'−R$ 851,94');
    assert.equal((await calls(stalePage,'refresh_public_data_if_due')).length,2,'Returning to the same wallet/input set reuses native caches');
    await noIndividualAutomaticRefresh(stalePage);await noUnexpectedCommands(stalePage);await stale.context.close();
    check('public-data-late-wallet-response-cannot-overwrite-current-wallet-and-return-reuses-cache');

    for(const mode of ['auto-offline','auto-rejected','auto-no-key']){
      const offline=await openPage(mode,{width:1440,height:1000}),offlinePage=offline.page;
      await automaticCount(offlinePage,1);await releaseAutomatic(offlinePage);
      const savedResult=region(offlinePage,'Resultado da carteira');
      await waitFigure(readout(savedResult,'Resultado da carteira'),'−R$ 854,54');
      await figureEquals(readout(savedResult,'Total aplicado'),'R$ 2.240,00');
      const sourceSummary=region(offlinePage,'Mercado em resumo');
      await offlinePage.waitForFunction(()=>!document.querySelector('.public-data-state')?.hasAttribute('aria-busy'));
      if(mode==='auto-rejected')await textMatches(sourceSummary.locator('.public-data-guidance'),/Não foi possível consultar.*dados salvos continuam disponíveis/);
      else{
        await openExtra(sourceSummary);
        const status=mode==='auto-no-key'?'not_configured':'unavailable';
        assert.equal(await sourceSummary.locator('[data-source=quotes]').getAttribute('data-source-status'),status);
        if(mode==='auto-offline')await textMatches(sourceSummary.locator('[data-source=quotes]'),/Sem conexão sintética; mantendo a observação salva\./);
        else await textMatches(sourceSummary.locator('.public-data-guidance'),/Cotações sem chave configurada.*brapi.*Configurações/);
      }
      await navigate(offlinePage,'Mercado');
      const savedQuote=region(offlinePage,'Cotações dos fundos').locator('.market-quotes-table tbody tr').first();
      await textMatches(savedQuote,/^TEST11.*R\$ 8,99/);
      assert.equal(await savedQuote.locator('time').getAttribute('datetime'),'2020-02-03T18:00:00Z');
      await navigate(offlinePage,'Visão geral');
      assert.equal((await calls(offlinePage,'refresh_public_data_if_due')).length,1,'Unavailable sources must not create a retry loop when navigating');
      await noIndividualAutomaticRefresh(offlinePage);await noUnexpectedCommands(offlinePage);await offline.context.close();
      check(`${mode}-retains-known-financial-values-and-observation-dates-without-navigation-retry`);
    }
    const zoom=await openPage('auto-zoom-200',{width:720,height:500},{expandedSidebar:false});
    await automaticCount(zoom.page,1);await releaseAutomatic(zoom.page);
    await waitFigure(readout(region(zoom.page,'Resultado da carteira'),'Resultado da carteira'),'−R$ 851,94');
    await noOverflow(zoom.page,'automatic-summary-zoom-200-no-overflow');await shot(zoom.page,'zoom-200-equivalent-automatic-public-data');
    await noUnexpectedCommands(zoom.page);await zoom.context.close();
    const web=await openPage('web',{width:375,height:812});
    await navigate(web.page,'Mercado');await navigate(web.page,'Visão geral');
    assert.equal(await web.page.evaluate(()=>'__TAURI_INTERNALS__' in window),false);
    assert.deepEqual(web.externalRequests,[],'Web preview must not issue public API or news requests');
    await web.context.close();check('public-data-web-preview-never-invokes-native-or-external-HTTP');
  }
  async function verifyCompactLayout(){
    for(const viewport of [{width:1920,height:1080},{width:1440,height:1000},{width:1000,height:760},{width:375,height:812},{width:720,height:500}]){
      const label=viewport.width===720?'compact-zoom-200':'compact-'+viewport.width;
      const fixture=await openPage(label,viewport,{expandedSidebar:false,sidebarPreference:'false'}),page=fixture.page;
      const compact=page.locator('.app-shell');assert.match(await compact.getAttribute('class'),/sidebar-collapsed/);
      if(viewport.width>760){
        const metrics=await sidebarGeometry(page,label);assert.ok(Math.abs(metrics.width-64)<=1,`${label}: the compact navigation uses its 64px rail`);
      }else{
        await banner(page).getByRole('button',{name:'Abrir menu',exact:true}).click();await sidebarGeometry(page,label+'-menu');
        await page.locator('.nav-backdrop').click({position:{x:viewport.width-12,y:Math.min(200,viewport.height/2)}});
      }
      await contentGeometry(page,label);await noOverflow(page,label+'-no-overflow');
      const financial=region(page,'Resultado da carteira');
      await figureEquals(readout(financial,'Total aplicado'),'R$ 2.240,00');await figureEquals(readout(financial,'Rendimentos acumulados'),'+R$ 45,56');await figureEquals(readout(financial,'Resultado da carteira'),'−R$ 854,54');
      assert.deepEqual(await financial.locator('.performance-readouts .figure-value').evaluateAll(cells=>cells.filter(cell=>cell.scrollWidth>cell.clientWidth+1).map(cell=>cell.textContent)),[],`${label}: financial totals remain fully readable`);
      await shot(page,label+'-overview',{fullPage:viewport.width<761});
      if(viewport.width>760){
        const compactWidth=(await sidebar(page).boundingBox()).width;
        await page.getByRole('button',{name:'Expandir barra lateral',exact:true}).focus();await page.keyboard.press('Enter');
        await page.waitForFunction(()=>!document.querySelector('.app-shell')?.classList.contains('sidebar-collapsed'));
        assert.ok((await sidebar(page).boundingBox()).width>compactWidth+50,`${label}: expanding works in the current session`);
        await sidebarGeometry(page,label+'-expanded');await contentGeometry(page,label+'-expanded');
        await page.evaluate(()=>localStorage.setItem('ivestfii.sidebarCollapsed','false'));
        await page.reload({waitUntil:'networkidle'});await title(page,'Visão geral');
        assert.match(await compact.getAttribute('class'),/sidebar-collapsed/,`${label}: reopening ignores the prior expanded preference`);
        await sidebarGeometry(page,label+'-reopened');check(label+'-always-reopens-compact-after-expansion-and-legacy-false-preference');
      }
      await noUnexpectedCommands(page);await fixture.context.close();
    }
  }
  async function verifyShell(){
    const fixture=await openPage('shell',{width:1440,height:1000},{expandedSidebar:false,sidebarPreference:'false'});const page=fixture.page;
    const shell=page.locator('.app-shell'),fab=()=>page.getByRole('button',{name:'Abrir assistente',exact:true});
    const assistant=()=>page.locator('.assistant-page'),drawer=()=>page.getByRole('dialog',{name:'Assistente lateral',exact:true});
    const draft=()=>page.locator('#chat-draft');
    assert.equal(await draft().count(),0,'The conversation is initialized only on its first opening');
    const compactWidth=(await sidebar(page).boundingBox()).width;
    for(const name of pages)assert.equal(await sidebar(page).getByRole('button',{name,exact:true}).locator('svg[aria-hidden="true"]').count(),1);
    assert.match(await shell.getAttribute('class'),/sidebar-collapsed/);
    await page.getByRole('button',{name:'Expandir barra lateral',exact:true}).focus();await page.keyboard.press('Space');
    await page.waitForFunction(()=>!document.querySelector('.app-shell')?.classList.contains('sidebar-collapsed'));
    const expandedWidth=(await sidebar(page).boundingBox()).width;assert.ok(expandedWidth>compactWidth+50);
    await page.getByRole('button',{name:'Recolher barra lateral',exact:true}).focus();await page.keyboard.press('Space');
    await page.waitForFunction(()=>document.querySelector('.app-shell')?.classList.contains('sidebar-collapsed'));
    assert.ok((await sidebar(page).boundingBox()).width<expandedWidth-50);
    assert.equal(await page.evaluate(()=>localStorage.getItem('ivestfii.sidebarCollapsed')),'false','The legacy preference is ignored rather than defining the next opening');
    for(const name of pages)assert.equal(await sidebar(page).getByRole('button',{name,exact:true}).getAttribute('title'),name);
    await noOverflow(page,'desktop-collapsed-sidebar-no-overflow');await shot(page,'desktop-sidebar-collapsed');
    await page.reload({waitUntil:'networkidle'});await title(page,'Visão geral');
    assert.match(await shell.getAttribute('class'),/sidebar-collapsed/);
    const market=sidebar(page).getByRole('button',{name:'Mercado',exact:true});await market.focus();await page.keyboard.press('Enter');await title(page,'Mercado');
    assert.equal(await market.getAttribute('aria-current'),'page');
    await page.getByRole('button',{name:/^Carteira: Carteira de teste\. Expandir para trocar carteira$/}).click();
    await page.waitForFunction(()=>!document.querySelector('.app-shell')?.classList.contains('sidebar-collapsed'));
    await focusReturnsTo(page,page.getByRole('combobox',{name:'Carteira',exact:true}));
    assert.equal(await page.evaluate(()=>localStorage.getItem('ivestfii.sidebarCollapsed')),'false');
    check('sidebar-icons-accessible-names-session-expansion-default-compact-and-wallet-shortcut');

    await notificationCount(page,1);await notificationBell(page).click();await notificationPanel(page).waitFor();
    const center=()=>notificationPanel(page),item=code=>center().locator(`[data-notification-code="${code}"]`);
    assert.equal(await item('import-warning').count(),1);await notificationCount(page,1);
    await page.keyboard.press('Escape');await center().waitFor({state:'detached'});await focusReturnsTo(page,notificationBell(page));await notificationCount(page,1);
    await navigate(page,'FIIs e posições');
    await banner(page).getByRole('button',{name:'Exportar carteira completa',exact:true}).click();await dismissToast(page,'Cópia da carteira exportada com segurança.');
    await notificationCount(page,2);
    await banner(page).getByRole('button',{name:'Exportar carteira completa',exact:true}).click();await dismissToast(page,'Cópia da carteira exportada com segurança.');
    await notificationBell(page).click();assert.equal(await center().locator('.notification-item').count(),2);await notificationCount(page,2);
    await item('export-completed').getByRole('button',{name:'Abrir página',exact:true}).click();await center().waitFor({state:'detached'});await title(page,'FIIs e posições');await notificationCount(page,1);
    await notificationBell(page).click();assert.equal(await item('import-warning').locator('.notification-item-actions').getByRole('button',{name:/^Marcar como lida:/}).count(),1);
    assert.equal(await item('export-completed').getAttribute('class').then(value=>value.includes('is-unread')),false);
    await item('import-warning').getByRole('button',{name:/^Marcar como lida:/}).click();await notificationCount(page,0);
    await shot(page,'desktop-notification-center',{fullPage:false});
    await page.keyboard.press('Escape');await center().waitFor({state:'detached'});
    await page.reload({waitUntil:'networkidle'});await title(page,'Visão geral');await notificationCount(page,0);
    await notificationBell(page).click();assert.equal(await center().locator('.notification-item').count(),2);assert.equal(await center().locator('.is-unread').count(),0);
    await item('import-warning').getByRole('button',{name:/^Dispensar notificação:/}).click();assert.equal(await item('import-warning').count(),0);
    await page.keyboard.press('Escape');await center().waitFor({state:'detached'});
    await page.reload({waitUntil:'networkidle'});await title(page,'Visão geral');await notificationCount(page,0);
    await notificationBell(page).click();assert.equal(await center().locator('.notification-item').count(),1);assert.equal(await item('import-warning').count(),0);
    await page.keyboard.press('Escape');await center().waitFor({state:'detached'});
    const storage=await page.evaluate(()=>localStorage.getItem('ivestfii.notifications.v1'));
    assert.doesNotMatch(storage,/sourceHash|sourcePath|walletId|conversationId|Corretora|TEST11|synthetic-position|1234\.56|title|message/);
    assert.deepEqual(await page.evaluate(()=>globalThis.__uiTest.snapshot.brokerPosition.warnings),position.warnings);
    check('notifications-deduplicate-opening-keeps-unread-actions-read-one-and-dismissal-persists-without-financial-text');

    const subscriptionsBefore=await chatSubscriptions(page);assert.equal((await calls(page,'get_providers')).length,0);
    await targetVisibleAndClickable(fab(),'desktop assistant shortcut');await fab().focus();await page.keyboard.press('Enter');
    await drawer().waitFor();await page.getByText('Resposta sintética preservada entre provedores.',{exact:true}).waitFor();
    assert.equal(await drawer().getAttribute('aria-modal'),'false');assert.equal(await draft().count(),1);assert.equal(await assistant().count(),1);
    await page.waitForFunction(expected=>globalThis.__uiTest.eventSubscriptions.filter(item=>item.event==='chat-run'&&item.active).length===expected,subscriptionsBefore+1);
    const subscriptions=await chatSubscriptions(page),listenCount=await chatListenCalls(page),providerReads=(await calls(page,'get_providers')).length;
    await page.evaluate(()=>{globalThis.__chatDraftNode=document.getElementById('chat-draft');});
    await drawer().getByRole('combobox',{name:'Provedor de IA',exact:true}).selectOption('claude');
    await drawer().locator('.chat-connection > summary').click();await drawer().getByRole('textbox',{name:'Modelo do provedor',exact:true}).fill('claude-fixture');
    const keptDraft='Pergunta sintética preservada entre o painel e a página.';await draft().fill(keptDraft);
    await drawer().getByRole('button',{name:'Ver contexto',exact:true}).click();await drawer().locator('.context-preview pre').filter({hasText:'Contexto sintético'}).waitFor();
    await drawer().getByRole('button',{name:/^Conversas \(/}).click();await drawer().getByRole('complementary',{name:'Conversas',exact:true}).waitFor();
    assert.equal(await drawer().getByRole('button',{name:/^Conversa sintética preservada/}).getAttribute('aria-current'),'true');
    await drawer().getByRole('button',{name:/^Conversa sintética preservada/}).click();
    await targetVisibleAndClickable(drawer().getByRole('button',{name:'Fechar assistente',exact:true}),'desktop assistant close');
    await noOverflow(page,'desktop-shared-assistant-drawer-no-overflow');await shot(page,'desktop-assistant-drawer',{fullPage:false});
    await drawer().getByRole('button',{name:'Abrir Assistente na página',exact:true}).click();await title(page,'Assistente');
    assert.equal(await assistant().getAttribute('data-assistant-view'),'page');assert.equal(await draft().inputValue(),keptDraft);
    assert.equal(await page.evaluate(()=>document.getElementById('chat-draft')===globalThis.__chatDraftNode),true);
    assert.equal(await page.getByRole('combobox',{name:'Provedor de IA',exact:true}).inputValue(),'claude');assert.equal(await page.locator('input[aria-label="Modelo do provedor"]').inputValue(),'claude-fixture');
    await page.locator('.context-preview pre').filter({hasText:'Contexto sintético'}).waitFor();
    await navigate(page,'Mercado');assert.equal(await assistant().getAttribute('data-assistant-view'),'hidden');assert.equal(await draft().inputValue(),keptDraft);
    await fab().click();await drawer().waitFor();assert.equal(await draft().inputValue(),keptDraft);
    assert.equal(await chatSubscriptions(page),subscriptions);assert.equal(await chatListenCalls(page),listenCount);assert.equal((await calls(page,'get_providers')).length,providerReads);
    check('one-assistant-instance-preserves-draft-provider-model-context-and-listener-across-drawer-page-hidden');

    await page.evaluate(()=>{globalThis.__uiTest.deferChatStart=true;});
    await drawer().getByRole('button',{name:'Enviar',exact:true}).evaluate(button=>{button.click();button.click();});
    await page.waitForFunction(()=>globalThis.__uiTest.chatStartWaiters.length===1);
    assert.equal((await calls(page,'start_chat_turn')).length,1);
    assert.deepEqual((await calls(page,'start_chat_turn'))[0].args,{conversationId:'conversation-a',walletId:'wallet-original',provider:'claude',model:'claude-fixture',content:keptDraft});
    await drawer().getByRole('button',{name:'Fechar assistente',exact:true}).click();await drawer().waitFor({state:'hidden'});
    await page.evaluate(()=>{const test=globalThis.__uiTest;test.deferChatStart=false;test.chatStartWaiters.splice(0).forEach(resolve=>resolve());});
    await page.waitForFunction(()=>document.getElementById('chat-draft').value==='');
    await page.evaluate(()=>globalThis.__uiTest.emitChatRun({status:'running',text:'Análise sintética da carteira original em andamento.'}));
    await fab().click();await drawer().waitFor();await drawer().getByText('Análise sintética da carteira original em andamento.',{exact:true}).waitFor();
    assert.equal(await drawer().getByRole('combobox',{name:'Provedor de IA',exact:true}).isDisabled(),true);
    if(!await page.getByRole('combobox',{name:'Carteira',exact:true}).isVisible())await page.getByRole('button',{name:/^Carteira:.*Expandir para trocar carteira$/}).click();
    await page.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-other');
    await drawer().locator('.chat-wallet-notice').filter({hasText:'Carteira de teste'}).waitFor();
    assert.equal(await drawer().getByText('Análise sintética da carteira original em andamento.',{exact:true}).count(),1);
    const nextDraft='Pergunta sintética da outra carteira.';await draft().fill(nextDraft);
    await drawer().getByRole('button',{name:'Fechar assistente',exact:true}).click();await drawer().waitFor({state:'hidden'});
    await page.evaluate(()=>globalThis.__uiTest.emitChatRun({status:'completed',text:'Resposta final sintética da carteira original.'}));
    await notificationCount(page,1);await fab().click();await drawer().waitFor();
    await drawer().getByText('Resposta final sintética da carteira original.',{exact:true}).waitFor();
    assert.equal(await drawer().getByText('Resposta final sintética da carteira original.',{exact:true}).count(),1);
    assert.equal(await draft().inputValue(),nextDraft);assert.equal(await chatSubscriptions(page),subscriptions);assert.equal(await chatListenCalls(page),listenCount);
    check('chat-double-submit-starts-once-and-hidden-stream-completion-preserves-next-draft-and-original-wallet');
    await notificationBell(page).click();await center().waitFor();await page.keyboard.press('Escape');await center().waitFor({state:'detached'});await focusReturnsTo(page,notificationBell(page));assert.equal(await drawer().isVisible(),true);
    await draft().focus();assert.equal(await isFocused(draft()),true);await page.keyboard.press('Escape');await drawer().waitFor({state:'hidden'});await focusReturnsTo(page,fab());
    await notificationBell(page).click();await item('chat-completed').waitFor();await center().getByRole('button',{name:'Marcar todas como lidas',exact:true}).click();await notificationCount(page,0);
    assert.equal(await center().locator('.is-unread').count(),0);
    await item('chat-completed').getByRole('button',{name:'Abrir conversa',exact:true}).click();await title(page,'Assistente');await center().waitFor({state:'detached'});
    assert.equal(await draft().inputValue(),nextDraft);assert.equal(await assistant().count(),1);assert.equal(await draft().count(),1);
    const beforeNext=(await calls(page,'start_chat_turn')).length;await draft().focus();await page.keyboard.press('Control+Enter');
    await page.getByRole('button',{name:'Interromper',exact:true}).waitFor();assert.equal((await calls(page,'start_chat_turn')).length,beforeNext+1);
    assert.equal((await calls(page,'start_chat_turn')).at(-1).args.walletId,'wallet-other');assert.equal((await calls(page,'start_chat_turn')).at(-1).args.content,nextDraft);
    await page.getByRole('button',{name:'Interromper',exact:true}).click();await page.locator('.chat-run-notice').filter({hasText:'Resposta interrompida'}).waitFor();
    assert.equal((await calls(page,'cancel_chat_turn')).length,1);
    assert.equal(await chatSubscriptions(page),subscriptions);assert.equal(await chatListenCalls(page),listenCount);
    assert.doesNotMatch(await page.evaluate(()=>localStorage.getItem('ivestfii.notifications.v1')),/conversation-a|wallet-original|wallet-other|Pergunta sintética|Resposta final sintética/);
    check('notification-Escape-keeps-chat-open-mark-all-and-conversation-action-share-state-and-next-turn-uses-new-wallet');
    await draft().fill('Pergunta que termina enquanto a nova conversa está vazia.');await page.getByRole('button',{name:'Enviar',exact:true}).click();
    await page.getByRole('button',{name:'Interromper',exact:true}).waitFor();await banner(page).getByRole('button',{name:'Nova conversa',exact:true}).click();
    await page.getByRole('heading',{name:'Nenhuma mensagem ainda',exact:true}).waitFor();assert.equal(await draft().inputValue(),'');
    assert.equal(await chatSubscriptions(page),subscriptions);assert.equal(await chatListenCalls(page),listenCount);
    await navigate(page,'Mercado');
    await page.evaluate(()=>globalThis.__uiTest.emitChatRun({conversationId:'conversation-a',status:'completed',text:'Resposta em segundo plano preservada.'}));
    await notificationCount(page,1);await fab().click();await drawer().waitFor();
    await drawer().getByRole('heading',{name:'Nenhuma mensagem ainda',exact:true}).waitFor();assert.equal(await drawer().locator('.message-text').count(),0);
    for(const command of ['get_messages','get_chat_run','get_tool_events'])assert.ok((await calls(page,command)).every(call=>!!call.args.conversationId),'A blank new conversation must not poll with an empty ID');
    await drawer().getByRole('button',{name:'Fechar assistente',exact:true}).click();await drawer().waitFor({state:'hidden'});
    await notificationBell(page).click();await center().locator('[data-notification-code="chat-completed"].is-unread').getByRole('button',{name:'Abrir conversa',exact:true}).click();
    await title(page,'Assistente');await page.getByText('Resposta em segundo plano preservada.',{exact:true}).waitFor();assert.equal(await page.getByText('Resposta em segundo plano preservada.',{exact:true}).count(),1);await notificationCount(page,0);
    await page.evaluate(()=>globalThis.__uiTest.emitChatRun({conversationId:'conversation-a',status:'completed',text:'Resposta em segundo plano preservada.'}));await notificationCount(page,0);
    assert.equal(await chatSubscriptions(page),subscriptions);assert.equal(await chatListenCalls(page),listenCount);
    check('new-empty-conversation-keeps-one-listener-and-terminal-background-notification-without-mixed-timeline-or-empty-polling');
    await noUnexpectedCommands(page);await fixture.context.close();

    for(const viewport of [{width:375,height:812},{width:720,height:500}]){
      const mobile=await openPage('shell-'+viewport.width,viewport);const page=mobile.page;
      const bell=notificationBell(page);await bell.click();const notifications=notificationPanel(page);await notifications.waitFor();
      await targetVisibleAndClickable(notifications.getByRole('button',{name:'Fechar notificações',exact:true}),`notifications close ${viewport.width}`);
      await noOverflow(page,`notification-center-${viewport.width}-no-overflow`);await shot(page,`notifications-${viewport.width}`,{fullPage:false});
      await page.keyboard.press('Escape');await notifications.waitFor({state:'detached'});await focusReturnsTo(page,bell);
      const fab=page.getByRole('button',{name:'Abrir assistente',exact:true});await targetVisibleAndClickable(fab,`assistant FAB ${viewport.width}`);await fab.click();
      const panel=page.getByRole('dialog',{name:'Assistente lateral',exact:true});await panel.waitFor();await panel.getByText('Resposta sintética preservada entre provedores.',{exact:true}).waitFor();
      assert.equal(await panel.getAttribute('aria-modal'),'true');assert.equal(await fab.count(),0);
      assert.equal(await page.locator('.page-content').getAttribute('inert'),'');assert.equal(await page.locator('.topbar').getAttribute('inert'),'');
      const input=panel.getByRole('textbox',{name:'Sua pergunta',exact:true});await input.fill('Rascunho móvel que precisa continuar guardado.');
      await revealInViewport(input);await targetVisibleAndClickable(input,`assistant draft ${viewport.width}`);
      await revealInViewport(panel.getByRole('button',{name:'Enviar',exact:true}));await targetVisibleAndClickable(panel.getByRole('button',{name:'Enviar',exact:true}),`assistant send ${viewport.width}`);
      await panel.getByRole('button',{name:'Nova conversa',exact:true}).focus();await page.keyboard.press('Shift+Tab');assert.equal(await panel.evaluate(element=>element.contains(document.activeElement)),true);
      for(let index=0;index<14;index++){await page.keyboard.press('Tab');assert.equal(await panel.evaluate(element=>element.contains(document.activeElement)),true,`Focus left the modal assistant at ${viewport.width}px`);}
      await revealInViewport(panel.locator('.chat-toolbar .def').first());await panel.locator('.chat-toolbar .def').first().focus();await page.getByRole('tooltip').waitFor();
      await page.keyboard.press('Escape');assert.equal(await panel.isVisible(),true,'Escape closes the explanation before the assistant');
      await revealInViewport(input);await input.focus();await page.mouse.move(2,2);await page.getByRole('tooltip').waitFor({state:'hidden'});
      await noOverflow(page,`shared-assistant-${viewport.width}-no-overflow`);await shot(page,`assistant-drawer-${viewport.width}`,{fullPage:false});
      await input.focus();
      await page.keyboard.press('Escape');await panel.waitFor({state:'hidden'});await focusReturnsTo(page,fab);
      await fab.click();await panel.waitFor();assert.equal(await input.inputValue(),'Rascunho móvel que precisa continuar guardado.');
      await panel.getByRole('button',{name:'Fechar assistente',exact:true}).click();await panel.waitFor({state:'hidden'});
      for(const name of pages.slice(1)){
        await navigate(page,name);await noOverflow(page,`mobile-assistant-action-${viewport.width}-${name}`);
        await targetVisibleAndClickable(fab,`assistant header action ${viewport.width} ${name}`);
      }
      await noUnexpectedCommands(page);await mobile.context.close();
      check(`mobile-shell-notifications-chat-modal-focus-and-draft-preserved-${viewport.width}`);
    }
  }

  async function verifyMonthlyIncome(){
    const currency=value=>new Intl.NumberFormat('pt-BR',{style:'currency',currency:'BRL'}).format(Number(value));
    const readOnly=async fixture=>{
      const mutations=await fixture.page.evaluate(()=>globalThis.__uiTest.calls.filter(({command})=>/^(?:save_|upsert_|delete_|remove_|import_|export_|start_chat_turn$|create_conversation$)/.test(command)));
      assert.deepEqual(mutations,[],'Viewing the chart must not call save_record/upsert_fund, import financial data, or start AI');
      assert.deepEqual(fixture.externalRequests,[],'The synthetic income scenarios must not make external HTTP requests');
      await noUnexpectedCommands(fixture.page);
    };
    const chartOf=page=>page.locator('.ov-income');
    const monthButton=(page,month)=>chartOf(page).locator(`.ov-income-month[data-month="${month}"] .def`);
    const monthRows=page=>chartOf(page).getByRole('table',{name:'Proventos por mês',exact:true}).locator('tbody > tr');
    async function prepare(page){
      await openExtra(region(page,'Histórico de FIIs'));
      await chartOf(page).locator('.ov-chart').waitFor({state:'visible'});
      await revealInViewport(chartOf(page).getByRole('heading',{name:'Proventos por mês',exact:true}));
      assert.deepEqual(await chartOf(page).locator('.ov-income-month').evaluateAll(items=>items.map(item=>item.dataset.month)),incomeExpected.map(([month])=>month));
      assert.equal(await chartOf(page).locator('.ov-income-month [role="button"][tabindex="0"]').count(),12);
    }
    async function dismiss(page){await page.keyboard.press('Escape');await page.getByRole('tooltip').waitFor({state:'hidden'});await page.mouse.move(1,1);}
    async function explain(page,month,method='hover'){
      await dismiss(page);
      const trigger=monthButton(page,month);await revealInViewport(trigger);
      if(method==='tap')await trigger.tap();else if(method==='focus')await trigger.focus();else await trigger.hover();
      const bubble=page.getByRole('tooltip');await bubble.waitFor({state:'visible'});
      assert.equal(await trigger.getAttribute('aria-expanded'),'true');assert.equal(await trigger.getAttribute('aria-describedby'),await bubble.getAttribute('id'));
      assert.equal(await trigger.getAttribute('title'),null,'Month detail uses the accessible shared tooltip, rather than a native title');
      const boxes=await bubble.evaluate(element=>{const b=element.getBoundingClientRect();return {x:b.x,y:b.y,right:b.right,bottom:b.bottom,width:innerWidth,height:innerHeight};});
      assert.ok(boxes.x>=11.5&&boxes.y>=11.5&&boxes.right<=boxes.width-11.5&&boxes.bottom<=boxes.height-11.5,`Monthly explanation stays inside the viewport: ${JSON.stringify(boxes)}`);
      assert.equal(await bubble.getByRole('rowheader',{name:'FUTR11',exact:true}).count(),0,'A fund first bought after the visible period is not treated as missing in earlier months');
      assert.deepEqual(await bubble.locator('thead th').allTextContents(),['Fundo','Cotas no histórico','Rendimento']);
      return {trigger,bubble};
    }
    async function fundEquals(bubble,ticker,held,income,{computed=false}={}){
      const row=bubble.getByRole('row',{name:new RegExp('^'+ticker+' ')}),cells=row.locator('td');
      assert.equal(normalize(await cells.nth(0).innerText()),held);
      assert.equal(compact(await cells.nth(1).innerText()),income===null?'Semregistro':compact(currency(income))+(computed?'calculado':''));
    }
    async function captureOpen(page,name){const filename=name+'.png';await page.screenshot({path:path.join(output,filename),fullPage:false,animations:'disabled'});screenshots.push(filename);}
    const fixture=await openPage('income',{width:1440,height:1000},{expandedSidebar:false}),page=fixture.page;
    await prepare(page);
    const chart=chartOf(page),rows=monthRows(page);
    assert.equal(await rows.count(),12);
    for(let index=0;index<incomeExpected.length;index++){
      const [month,value]=incomeExpected[index],cells=rows.nth(index).locator('td');
      const monthLabel=['jan','fev','mar','abr','mai','jun','jul','ago','set','out','nov','dez'][Number(month.slice(5))-1]+'/'+month.slice(2,4);
      assert.equal(normalize(await cells.nth(0).textContent()),monthLabel+(month==='2026-10'?' (parcial, até 20/10)':''));
      assert.equal(normalize(await cells.nth(1).textContent()),value===null?(month==='2026-03'?'Sem rendimento registrado':'Sem registros'):normalize(currency(value)));
    }
    await figureEquals(chart.locator('.ov-head-figure > .num'),'R$ 28,95');
    assert.equal(await page.evaluate(()=>globalThis.__uiTest.snapshot.totals.dividendsReceived),'28.95');
    assert.equal(await chart.locator('.ov-income-average, svg .avg').count(),0,'A mean must not turn gaps or partial fund coverage into zero months');
    assert.equal(await chart.locator('svg .cap-zero').count(),1,'Only the explicit February zero is a numeric zero cap');
    assert.equal(await chart.locator('svg .cap-missing').count(),5,'December, March and June–August are gaps, not recorded zero payouts');
    assert.equal(await chart.locator('svg .bar-partial').count(),1,'Current October remains partial and hatched');
    assert.ok((await rows.nth(2).locator('td').nth(3).textContent()).includes('BETA11'));
    assert.equal(normalize(await rows.nth(5).locator('td').nth(4).textContent()),'ALFA11: 12; BETA11: 5','The screen-reader table exposes the same historical holdings as the visual explanation');
    check('monthly-income-12-months-preserve-recorded-sum-distinguish-gaps-zero-and-partial-coverage');
    await noOverflow(page,'monthly-income-desktop-no-outer-overflow');await sectionShot(page,'desktop-monthly-income',chart);

    let opened=await explain(page,'2025-11');
    await fundEquals(opened.bubble,'ALFA11','10','5',{computed:true});await fundEquals(opened.bubble,'BETA11','5','1',{computed:true});
    assert.match(await opened.bubble.innerText(),/Sem novas compras: as cotas anteriores continuam no saldo/);
    assert.match(await opened.bubble.innerText(),/2 de 2 fundos com rendimento registrado/);
    await opened.bubble.hover();assert.equal(await opened.bubble.isVisible(),true,'Moving from the month to its explanation keeps it open');
    check('monthly-income-carries-existing-holdings-without-requiring-a-new-purchase');

    opened=await explain(page,'2025-12');
    await fundEquals(opened.bubble,'ALFA11','10',null);await fundEquals(opened.bubble,'BETA11','5',null);
    assert.match(await opened.bubble.innerText(),/Sem registros/);assert.match(await opened.bubble.innerText(),/Isso não confirma rendimento zero/);
    assert.match(await opened.bubble.innerText(),/Não repetimos automaticamente o pagamento do mês anterior/);
    await captureOpen(page,'desktop-monthly-income-gap');check('monthly-income-missing-month-keeps-cotas-and-does-not-repeat-a-prior-payment');

    opened=await explain(page,'2026-01','focus');
    await fundEquals(opened.bubble,'ALFA11','10','6',{computed:true});await fundEquals(opened.bubble,'BETA11','5',null);
    assert.match(await opened.bubble.innerText(),/1 de 2 fundos com rendimento registrado/);
    await captureOpen(page,'desktop-monthly-income-partial-funds');await dismiss(page);assert.equal(await opened.trigger.evaluate(element=>element===document.activeElement),true,'Escape dismisses only the explanation and retains keyboard focus');
    check('monthly-income-keyboard-focus-explains-partial-fund-coverage-and-Escape-preserves-focus');

    opened=await explain(page,'2026-02');
    await fundEquals(opened.bubble,'ALFA11','10','0');await fundEquals(opened.bubble,'BETA11','5','0');
    assert.match(await opened.bubble.innerText(),/R\$ 0 informado no lançamento é diferente de falta de registro/);
    assert.doesNotMatch(await opened.bubble.innerText(),/Falta o rendimento/);
    await captureOpen(page,'desktop-monthly-income-explicit-zero');check('monthly-income-explicit-zero-overrides-a-positive-per-share-value-without-becoming-missing');

    opened=await explain(page,'2026-03');
    await fundEquals(opened.bubble,'ALFA11','12',null);await fundEquals(opened.bubble,'BETA11','5',null);
    assert.match(await opened.bubble.innerText(),/Sem rendimento registrado/);assert.match(await opened.bubble.innerText(),/Há lançamentos, mas nenhum rendimento informado/);
    check('monthly-income-purchase-only-month-updates-held-shares-without-inventing-income');
    opened=await explain(page,'2026-04');
    await fundEquals(opened.bubble,'ALFA11','12','4.5',{computed:true});await fundEquals(opened.bubble,'BETA11','5','1.25',{computed:true});
    assert.equal(await opened.bubble.locator('tbody > tr').count(),2,'Repeated month records aggregate once per fund');
    assert.match(compact(await opened.bubble.locator('.ov-income-tooltip-title').innerText()),/R\$5,75/);
    check('monthly-income-same-month-records-sum-without-counting-the-held-shares-twice');
    opened=await explain(page,'2026-10');await fundEquals(opened.bubble,'ALFA11','12','2');await fundEquals(opened.bubble,'BETA11','5',null);
    assert.match(await opened.bubble.innerText(),/Parcial até 20\/10/);await dismiss(page);
    await readOnly(fixture);await fixture.context.close();

    for(const [width,height,touch] of [[720,900,false],[375,812,true]]){
      const responsive=await openPage('income-'+width,{width,height},{expandedSidebar:false,touch}),small=responsive.page;
      await prepare(small);const scroll=chartOf(small).locator('.ov-income-scroll');
      const geometry=await scroll.evaluate(element=>({scrollWidth:element.scrollWidth,clientWidth:element.clientWidth,chartWidth:element.querySelector('.ov-chart').getBoundingClientRect().width}));
      assert.ok(geometry.chartWidth>=720&&geometry.scrollWidth>geometry.clientWidth,'Narrow screens retain legible month slots and local chart scrolling');
      await noOverflow(small,'monthly-income-'+width+'-no-outer-overflow');await sectionShot(small,'monthly-income-'+width,chartOf(small));
      const {trigger,bubble}=await explain(small,'2026-04',touch?'tap':'focus');
      await fundEquals(bubble,'ALFA11','12','4.5',{computed:true});await fundEquals(bubble,'BETA11','5','1.25',{computed:true});
      await captureOpen(small,'monthly-income-'+width+'-explanation');
      if(touch){await trigger.tap();await bubble.waitFor({state:'hidden'});await trigger.tap();await bubble.waitFor({state:'visible'});await chartOf(small).getByRole('heading',{name:'Proventos por mês',exact:true}).tap();await bubble.waitFor({state:'hidden'});}
      else{await dismiss(small);assert.equal(await trigger.evaluate(element=>element===document.activeElement),true);}
      check('monthly-income-'+width+(touch?'-touch-toggle-and-outside-dismiss':'-keyboard-focus-and-Escape')+'-with-local-scroll-and-viewport-clamped-explanation');
      await readOnly(responsive);await responsive.context.close();
    }
    const short=await openPage('income-short',{width:720,height:320},{expandedSidebar:false});await prepare(short.page);
    const shortTip=await explain(short.page,'2026-01','focus');
    const before=await shortTip.bubble.evaluate(element=>({scroll:element.scrollTop,height:element.clientHeight,content:element.scrollHeight,pageY:scrollY}));
    assert.ok(before.content>before.height,'The short viewport genuinely exercises an overflowing explanation');
    await short.page.keyboard.press('PageDown');
    const after=await shortTip.bubble.evaluate(element=>({scroll:element.scrollTop,pageY:scrollY}));
    assert.ok(after.scroll>before.scroll);assert.equal(after.pageY,before.pageY,'PageDown scrolls the explanation without scrolling the underlying page');
    await short.page.keyboard.press('End');assert.equal(await shortTip.bubble.evaluate(element=>Math.abs(element.scrollTop+element.clientHeight-element.scrollHeight)<=1),true);
    await short.page.keyboard.press('Home');assert.equal(await shortTip.bubble.evaluate(element=>element.scrollTop),0);
    assert.equal(await shortTip.trigger.evaluate(element=>element===document.activeElement),true);
    await captureOpen(short.page,'monthly-income-720-short-keyboard-explanation');await dismiss(short.page);
    check('monthly-income-short-viewport-PageDown-Home-End-scroll-the-explanation-and-Escape-retains-anchor-focus');
    await readOnly(short);await short.context.close();
    for(const mode of ['income-unknown','income-zero']){
      const evidence=await openPage(mode,{width:1440,height:1000},{expandedSidebar:false});await prepare(evidence.page);
      await figureEquals(chartOf(evidence.page).locator('.ov-head-figure > .num'),mode==='income-zero'?'R$ 0,00':'—');
      assert.equal(await chartOf(evidence.page).locator('svg .avg,.ov-income-average').count(),0);
      const feb=monthRows(evidence.page).nth(3).locator('td').nth(1);
      assert.equal(normalize(await feb.textContent()),mode==='income-zero'?normalize(currency(0)):'Sem registros');
      check(mode==='income-zero'?'monthly-income-zero-recorded-evidence-shows-a-known-zero-total':'monthly-income-no-income-evidence-remains-unknown-instead-of-showing-a-false-zero-total');
      await readOnly(evidence);await evidence.context.close();
    }
    const complete=await openPage('income-complete',{width:1440,height:1000},{expandedSidebar:false});await prepare(complete.page);
    const completeChart=chartOf(complete.page);await figureEquals(completeChart.locator('.ov-head-figure > .num'),'R$ 38,95');
    await textMatches(completeChart.locator('.ov-income-average'),/^Média dos valores registrados em 11 meses fechados: R\$\s*3,36\.$/);
    assert.equal(await completeChart.locator('svg .avg').count(),1);
    const completeRows=monthRows(complete.page);assert.equal(normalize(await completeRows.nth(4).locator('td').nth(1).textContent()),normalize(currency(0)));
    await sectionShot(complete.page,'desktop-monthly-income-complete-average',completeChart);
    check('monthly-income-mean-includes-known-zero-months-and-excludes-current-partial-month-only-when-closed-coverage-is-complete');
    await readOnly(complete);await complete.context.close();
  }

  async function verifyAutomaticMonthlyIncome(){
    const fixture=await openPage('income-api',{width:1440,height:1000},{expandedSidebar:false}),page=fixture.page;
    const chart=page.locator('.ov-income');
    const original=await page.evaluate(()=>structuredClone(globalThis.__uiTest.snapshot));
    const sourceCalls=await calls(page,'get_monthly_income');
    assert.ok(sourceCalls.length>0);
    assert.deepEqual(sourceCalls.at(-1).args,{walletId:'wallet-original',startMonth:'2025-10',endMonth:'2026-10'});
    await openExtra(region(page,'Histórico de FIIs'));await chart.locator('.ov-chart').waitFor();
    async function explain(month,method='hover'){
      await page.keyboard.press('Escape');await page.mouse.move(1,1);
      const trigger=chart.locator(`.ov-income-month[data-month="${month}"] .def`);await revealInViewport(trigger);
      if(method==='focus')await trigger.focus();else await trigger.hover();
      const bubble=page.getByRole('tooltip');await bubble.waitFor();return {trigger,bubble};
    }
    async function captureExplanation(currentPage,name){
      const bubble=currentPage.getByRole('tooltip');await bubble.waitFor({state:'visible'});
      assert.equal(await currentPage.locator('.ov-income svg').isVisible(),true);
      const activeMonth=currentPage.locator('.ov-income-month .def[aria-expanded="true"]');
      const geometry=await activeMonth.evaluate(element=>{const box=element.getBoundingClientRect();return {top:box.top,bottom:box.bottom,height:innerHeight};});
      assert.ok(geometry.top>=0&&geometry.bottom<=geometry.height,'A tooltip capture must include its visible chart month');
      const filename=name+'.png';await currentPage.screenshot({path:path.join(output,filename),fullPage:false,animations:'disabled'});screenshots.push(filename);
    }
    const fundCell=(bubble,symbol)=>bubble.getByRole('row',{name:new RegExp('^'+symbol+' ')}).locator('td').nth(1);
    let opened=await explain('2026-07');
    assert.match(compact(await opened.bubble.locator('.ov-income-tooltip-title').innerText()),/R\$7,00/);
    await textEquals(fundCell(opened.bubble,'ALFA11'),'R$ 6,00 B3 · automático');
    await textEquals(fundCell(opened.bubble,'BETA11'),'R$ 1,00 B3 · automático');
    assert.match(await opened.bubble.innerText(),/2 de 2 fundos com rendimento disponível/);
    assert.match(await opened.bubble.innerText(),/Preenchido automaticamente.*Um valor informado por você tem prioridade/);
    assert.equal(await page.evaluate(()=>globalThis.__uiTest.snapshot.funds[1].entries.some(entry=>entry.month==='2026-07')),false,'The public payout fills a true ledger gap');
    check('monthly-income-B3-fills-an-unrecorded-fund-month-with-the-current-published-rate');
    const alphaRow=opened.bubble.getByRole('row',{name:/^ALFA11 /});
    await textEquals(alphaRow.locator('td').nth(0),'15');
    const alphaEvent=opened.bubble.locator('.ov-income-events').filter({hasText:'ALFA11 · cálculo B3'});
    assert.match(compact(await alphaEvent.innerText()),/12cotascomdireito×0,5R\$\/cota=R\$6,00/);
    assert.match(await alphaEvent.innerText(),/Data-com 10\/07\/2026 · pagamento 20\/07\/2026/);
    assert.equal(await alphaEvent.getByRole('link',{name:/^Ver na B3/}).getAttribute('href'),'https://sistemaswebb3-listados.b3.com.br/fundsListedPage/events-main/ALFA');
    await captureExplanation(page,'desktop-monthly-income-B3-cutoff');
    check('monthly-income-B3-shows-entitled-shares-separately-from-final-holdings-dates-formula-and-official-source');
    opened=await explain('2026-08','focus');
    await textEquals(fundCell(opened.bubble,'ALFA11'),'R$ 10,50 B3 · automático');
    await textEquals(fundCell(opened.bubble,'BETA11'),'R$ 1,50 B3 · automático');
    assert.match(await opened.bubble.innerText(),/Sem novas compras: as cotas anteriores continuam no saldo/);
    assert.match(compact(await opened.bubble.innerText()),/15cotascomdireito×0,7R\$\/cota=R\$10,50/);
    check('monthly-income-B3-carries-purchases-forward-and-uses-each-months-own-public-distribution');
    opened=await explain('2026-06');
    await textEquals(fundCell(opened.bubble,'BETA11'),'R$ 0,00');
    assert.match(await opened.bubble.innerText(),/R\$ 0 informado no lançamento é diferente de falta de registro/);
    assert.equal(await opened.bubble.locator('.ov-income-events').filter({hasText:'BETA11 · cálculo B3'}).count(),0);
    assert.equal(await page.evaluate(()=>globalThis.__uiTest.dividendPanel.items[1].observation.events.some(event=>event.paymentDate==='2026-06-20'&&event.amount==='4')),true,'A positive public payout exists but the canonical manual zero wins');
    check('monthly-income-manual-zero-takes-priority-over-an-available-positive-B3-distribution');
    await page.keyboard.press('Escape');await page.mouse.move(1,1);
    const writes=await page.evaluate(()=>globalThis.__uiTest.calls.filter(({command})=>/^(?:save_|upsert_|delete_|remove_|import_|export_|start_chat_turn$|create_conversation$)/.test(command)));
    assert.deepEqual(writes,[]);assert.deepEqual(await page.evaluate(()=>globalThis.__uiTest.snapshot),original,'Automatic chart calculation must not modify records, recorded totals, cash, or position');
    assert.deepEqual(fixture.externalRequests,[]);
    assert.equal((await calls(page,'save_record')).length,0);
    check('monthly-income-automatic-payouts-never-create-ledger-records-or-change-recorded-wallet-totals');
    await page.setViewportSize({width:375,height:812});
    opened=await explain('2026-07','focus');
    const geometry=await opened.bubble.evaluate(element=>{const box=element.getBoundingClientRect();return {left:box.left,right:box.right,top:box.top,bottom:box.bottom,width:innerWidth,height:innerHeight};});
    assert.ok(geometry.left>=11.5&&geometry.right<=geometry.width-11.5&&geometry.top>=11.5&&geometry.bottom<=geometry.height-11.5);
    await noOverflow(page,'monthly-income-B3-375-no-outer-overflow');await captureExplanation(page,'mobile-monthly-income-B3');
    check('monthly-income-B3-explanation-with-formula-and-dates-remains-inside-a-mobile-viewport');
    await page.keyboard.press('Escape');await page.mouse.move(1,1);await page.setViewportSize({width:1440,height:1000});
    await navigate(page,'FIIs e posições');await openTab(page,'Histórico');
    await banner(page).getByRole('button',{name:'Novo lançamento',exact:true}).click();
    const form=page.getByRole('dialog',{name:'Novo lançamento · ALFA11',exact:true});await form.waitFor();
    await form.getByLabel('Mês de referência',{exact:true}).fill('2026-10');
    await form.getByLabel('Cotas adicionadas',{exact:true}).fill('3');
    await form.getByLabel('Preço por cota (R$)',{exact:true}).fill('12,50');
    const date=form.getByLabel('Data da compra',{exact:true});await date.waitFor();
    assert.equal(await date.getAttribute('type'),'date');assert.equal(await date.getAttribute('min'),'2026-10-01');assert.equal(await date.getAttribute('max'),'2026-10-31');
    assert.equal(await date.evaluate(element=>element.required),true);
    await form.getByRole('button',{name:'Salvar lançamento',exact:true}).click();
    assert.equal((await calls(page,'save_record')).length,0);assert.equal(await date.evaluate(element=>element.validity.valueMissing),true);
    await date.fill('2026-09-30');await form.getByRole('button',{name:'Salvar lançamento',exact:true}).click();
    assert.equal((await calls(page,'save_record')).length,0);assert.equal(await date.evaluate(element=>element.validity.rangeUnderflow),true);
    check('new-purchase-requires-an-exact-date-within-the-reference-month-before-saving');
    await date.fill('2026-10-11');await form.getByRole('button',{name:'Salvar lançamento',exact:true}).click();await form.waitFor({state:'detached'});
    const save=(await calls(page,'save_record')).at(-1).args;
    assert.equal(save.walletId,'wallet-original');
    assert.deepEqual({...save.input,id:undefined},{id:undefined,fundId:'income-alpha',month:'2026-10',purchaseDate:'2026-10-11',cotasAdded:'3',pricePerCota:'12.50',dividendPerCota:'0',dividendTotal:null,notes:''});
    await dismissToast(page,'Lançamento salvo. Indicadores atualizados.');
    await navigate(page,'FIIs e posições');await openTab(page,'Histórico');
    const purchase=panel(page,'Histórico').locator('.purchases-table').getByRole('row').filter({has:page.locator('time[datetime="2026-10-11"]')});
    await purchase.waitFor();await textEquals(purchase.locator('time[datetime="2026-10-11"]'),'11/10/2026');
    check('new-purchase-sends-the-exact-ISO-date-and-displays-it-in-the-purchase-history');
    await noUnexpectedCommands(page);assert.deepEqual(fixture.externalRequests,[]);await fixture.context.close();

    const unknown=await openPage('income-api-date-required',{width:1440,height:1000},{expandedSidebar:false});
    await openExtra(region(unknown.page,'Histórico de FIIs'));
    const unknownMonth=unknown.page.locator('.ov-income-month[data-month="2026-07"] .def');await revealInViewport(unknownMonth);await unknownMonth.focus();
    const unknownTip=unknown.page.getByRole('tooltip');await unknownTip.waitFor();
    await textEquals(fundCell(unknownTip,'ALFA11'),'Informe a data da compra');
    assert.match(await unknownTip.innerText(),/Informe o dia da compra no histórico.*direito ao provento.*meses seguintes continuam usando essas cotas/);
    await textEquals(fundCell(unknownTip,'BETA11'),'R$ 1,00 B3 · automático');
    assert.equal(await unknownTip.locator('.ov-income-events').filter({hasText:'ALFA11 · cálculo B3'}).count(),0,'Unknown eligibility cannot be displayed as zero entitled shares');
    await captureExplanation(unknown.page,'desktop-monthly-income-date-required');
    assert.equal((await calls(unknown.page,'save_record')).length,0);assert.deepEqual(unknown.externalRequests,[]);
    check('monthly-income-purchase-without-a-day-requests-the-date-without-fabricating-an-API-payment');
    await unknown.page.keyboard.press('Escape');await unknown.page.mouse.move(1,1);
    await navigate(unknown.page,'FIIs e posições');await openTab(unknown.page,'Histórico');
    const legacy=panel(unknown.page,'Histórico').locator('.purchases-table').locator('tbody > tr').filter({has:unknown.page.locator('time[datetime="2026-07"]')});
    await legacy.getByRole('button',{name:/^Editar compra/}).click();
    const legacyForm=unknown.page.getByRole('dialog',{name:'Editar lançamento · ALFA11',exact:true});await legacyForm.waitFor();
    assert.equal(await legacyForm.getByLabel('Data da compra',{exact:true}).inputValue(),'');
    assert.equal(await legacyForm.getByLabel('Data da compra',{exact:true}).evaluate(element=>element.required),false,'Unchanged legacy entries remain editable without inventing a purchase day');
    await legacyForm.getByRole('button',{name:'Salvar lançamento',exact:true}).click();await legacyForm.waitFor({state:'detached'});
    const legacySave=(await calls(unknown.page,'save_record')).at(-1).args.input;
    assert.equal(legacySave.id,'alpha-july-purchase');assert.equal('purchaseDate' in legacySave,false);
    check('legacy-purchase-edit-preserves-an-unknown-day-until-the-user-provides-it');
    await noUnexpectedCommands(unknown.page);await unknown.context.close();
  }

  async function verifyAutomaticIncomePerformance(){
    const resultOf=page=>region(page,'Resultado da carteira');
    const compositionOf=page=>resultOf(page).getByRole('table',{name:'Composição do resultado da carteira: FIIs, caixinhas e pós-fixado',exact:true});
    const breakdownOf=page=>resultOf(page).getByRole('table',{name:'Composição do resultado total dos FIIs',exact:true});
    const fundProceeds=(page,symbol)=>page.locator('.ov-funds-table tbody tr').filter({has:page.locator('.ticker',{hasText:new RegExp('^'+symbol)})}).locator('td').last();
    async function expectedTotals(page,{income='+R$ 64,56',result='−R$ 835,54',fiiIncome='+R$ 30,00',fiiResult='−R$ 870,10',recorded='R$ 11,00',api='R$ 19,00',difference='−R$ 950,10',alpha='R$ 25,00'}={}){
      const section=resultOf(page),composition=compositionOf(page),breakdown=breakdownOf(page);
      await figureEquals(readout(section,'Total aplicado'),'R$ 2.240,00');
      await figureEquals(readout(section,'Rendimentos acumulados'),income);
      await figureEquals(readout(section,'Resultado da carteira'),result);
      for(const [name,values] of [['FIIs',['R$ 1.040,00',fiiIncome,fiiResult]],['Caixinhas',['R$ 1.200,00','+R$ 34,56','+R$ 34,56']],['Total',['R$ 2.240,00',income,result]]])
        assert.deepEqual((await composition.getByRole('row',{name:new RegExp('^'+name+' ')}).locator('td').allInnerTexts()).map(normalize),values);
      await figureEquals(breakdown.getByRole('row',{name:/^\+ Rendimentos registrados/}).locator('td'),recorded);
      await figureEquals(breakdown.getByRole('row',{name:/^\+ Proventos B3/}).locator('td'),api);
      await figureEquals(breakdown.getByRole('row',{name:/^= Resultado total/}).locator('td'),fiiResult);
      await textMatches(section.locator('.performance-difference'),new RegExp(escapeRegExp(difference)));
      await figureEquals(fundProceeds(page,'TEST11'),alpha);await figureEquals(fundProceeds(page,'MOCK11'),'R$ 5,00');
    }
    async function unchangedLedger(fixture,original){
      const {page}=fixture;
      assert.deepEqual(await page.evaluate(()=>globalThis.__uiTest.snapshot),original,'Reconciled public income must not write imported position, cash, purchases, or manual dividends');
      const writes=await page.evaluate(()=>globalThis.__uiTest.calls.filter(({command})=>/^(?:save_|upsert_|delete_|remove_|import_|export_|start_chat_turn$|create_conversation$)/.test(command)));
      assert.deepEqual(writes,[]);assert.deepEqual(fixture.externalRequests,[]);await noUnexpectedCommands(page);
    }

    const fixture=await openPage('income-performance',{width:1440,height:1000}),page=fixture.page;
    const original=await page.evaluate(()=>structuredClone(globalThis.__uiTest.snapshot));
    await expectedTotals(page);
    assert.equal((await calls(page,'refresh_fii_performance')).length,0,'Opening the view reads the cached native calculation');
    assert.equal(await page.locator('.ov-funds-table thead').getByText('Proventos acumulados',{exact:true}).count(),1);
    check('automatic-income-performance-joins-reconciled-B3-and-manual-income-with-cash-income-once');
    const section=resultOf(page),graph=section.locator('.performance-bars svg');
    assert.match(await graph.locator('desc').textContent(),/FIIs: −R\$\s*870,10\. CDI: \+R\$\s*80,00/);
    await graph.focus();await page.keyboard.press('Home');await textMatches(section.locator('.performance-point'),/^FIIs: −R\$ 870,10/);
    await page.keyboard.press('End');await textMatches(section.locator('.performance-point'),/^100% do CDI: \+R\$ 80,00/);
    await section.locator('.performance-chart-table > summary').click();
    const comparison=section.getByRole('table',{name:'Resultados acumulados comparados com as mesmas compras',exact:true});
    await figureEquals(comparison.getByRole('row',{name:/^FIIs /}).locator('td').first(),'−R$ 870,10');
    await figureEquals(comparison.getByRole('row',{name:/^100% do CDI /}).locator('td').first(),'+R$ 80,00');
    assert.match(await tip(section,'Rendimentos acumulados'),/B3/);
    check('automatic-income-performance-CDI-chart-keyboard-table-and-difference-use-the-combined-FII-result');
    await section.getByRole('button',{name:'Atualizar resultado',exact:true}).click();
    await section.getByText('Consulta concluída. Confira as datas dos dados.',{exact:true}).waitFor();
    await expectedTotals(page);
    assert.equal((await calls(page,'refresh_fii_performance')).length,1);
    await unchangedLedger(fixture,original);
    check('automatic-income-performance-reloading-public-income-does-not-compound-it-or-create-records');
    await page.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-other');
    await figureEquals(readout(resultOf(page),'Rendimentos acumulados'),'+R$ 20,00');
    await figureEquals(fundProceeds(page,'BANK11'),'R$ 0,00');
    assert.equal(await page.locator('.ov-funds-table thead').getByText('Proventos acumulados',{exact:true}).count(),0);
    assert.equal(await resultOf(page).locator('.performance-api-row').count(),0);
    await page.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-original');
    await expectedTotals(page);
    check('automatic-income-performance-switching-wallets-never-reuses-another-wallets-B3-fund-totals');
    await noOverflow(page,'automatic-income-performance-desktop-no-overflow');await sectionShot(page,'desktop-automatic-income-performance',section);
    await page.setViewportSize({width:375,height:812});await expectedTotals(page);
    await noOverflow(page,'automatic-income-performance-mobile375-no-overflow');
    assert.deepEqual(await breakdownOf(page).locator('td.num').evaluateAll(cells=>cells.filter(cell=>cell.scrollWidth>cell.clientWidth+1).map(cell=>cell.textContent)),[]);
    const note=await section.locator('.performance-api-income').evaluate(element=>{
      const range=document.createRange();range.selectNodeContents(element.firstChild);
      return {lineHeight:parseFloat(getComputedStyle(element).lineHeight),prefixHeight:range.getBoundingClientRect().height,dateHeight:element.querySelector('time').getBoundingClientRect().height};
    });
    assert.ok(note.prefixHeight<=note.lineHeight*1.5&&note.dateHeight<=note.lineHeight*1.5,`Mobile provenance must flow as a sentence, without splitting short words or dates into columns ${JSON.stringify(note)}`);
    await sectionShot(page,'mobile-automatic-income-performance375',section);
    check('automatic-income-performance-provenance-and-combined-totals-remain-readable-on-mobile');
    await unchangedLedger(fixture,original);await fixture.context.close();

    for(const mode of ['income-performance-overridden','income-performance-zero']){
      const current=await openPage(mode,{width:1440,height:1000}),currentPage=current.page;
      const saved=await currentPage.evaluate(()=>structuredClone(globalThis.__uiTest.snapshot));
      if(mode==='income-performance-overridden'){
        await expectedTotals(currentPage,{recorded:'R$ 26,00',api:'R$ 4,00'});
        check('automatic-income-performance-confirmed-manual-payment-replaces-the-same-B3-month-without-changing-total');
      }else{
        await expectedTotals(currentPage,{income:'+R$ 49,56',result:'−R$ 850,54',fiiIncome:'+R$ 15,00',fiiResult:'−R$ 885,10',api:'R$ 4,00',difference:'−R$ 965,10',alpha:'R$ 10,00'});
        const zero=await currentPage.evaluate(()=>globalThis.__uiTest.snapshot.funds[0].entries.find(entry=>entry.id==='entry-zero'));
        assert.equal(zero.dividendTotal,'0');
        assert.equal(await currentPage.evaluate(()=>globalThis.__uiTest.dividendPanel.items[0].observation.events.find(event=>event.paymentDate==='2026-03-20').amount),'15');
        check('automatic-income-performance-explicit-manual-zero-suppresses-only-that-funds-public-payment');
      }
      await unchangedLedger(current,saved);await current.context.close();
    }
  }

  async function verifyFixedIncomeObservation(){
    async function combinedTotals(page,{income='+R$ 105,56',result='−R$ 794,54',fiiIncome='+R$ 11,00',fiiResult='−R$ 889,10'}={}){
      const scope=region(page,'Resultado da carteira');
      await figureEquals(readout(scope,'Total aplicado'),'R$ 2.540,00');
      await figureEquals(readout(scope,'Rendimentos acumulados'),income);
      await figureEquals(readout(scope,'Resultado da carteira'),result);
      const table=scope.locator('.performance-portfolio-table');
      for(const [name,values] of [
        ['FIIs',['R$ 1.040,00',fiiIncome,fiiResult]],['Caixinhas',['R$ 1.200,00','+R$ 34,56','+R$ 34,56']],
        ['Pós-fixado',['R$ 300,00','+R$ 60,00','+R$ 60,00']],['Total',['R$ 2.540,00',income,result]],
      ])assert.deepEqual((await table.getByRole('row',{name:new RegExp('^'+name+' ')}).locator('td').allInnerTexts()).map(normalize),values);
      assert.equal(await table.locator('.performance-fixed-income-row').count(),1);
      await textMatches(scope.locator('.performance-fixed-income-scope'),/Pós-fixado sintético com aportes.*20\/10\/2026.*12 meses.*não.*somado/i);
    }
    const fixture=await openPage('fixed-income',{width:1440,height:1000}),page=fixture.page;
    const original=await page.evaluate(()=>structuredClone(globalThis.__uiTest.snapshot));
    await combinedTotals(page);
    check('fixed-income-native-portfolio-totals-include-FIIs-cash-and-current-fixed-income-once');
    const broker=region(page,'Posição na corretora');
    await figureEquals(broker.locator('.figure-hero'),'R$ 201,00');
    await textMatches(broker.locator('.cp-source'),/03\/02\/2020/);
    const update=broker.locator('.cp-fixed-update-note');
    await textMatches(update,/20\/10\/2026/);
    await update.getByRole('button',{name:'Ver pós-fixado',exact:true}).click();
    await title(page,'FIIs e posições');
    assert.equal(await tab(page,'Posição').getAttribute('aria-selected'),'true');
    check('fixed-income-new-observation-is-reachable-without-changing-the-older-overview');

    const current=page.locator('.cp-fixed-observation');
    await textMatches(current,/Pós-fixado sintético com aportes/);
    await textMatches(current,/20\/10\/2026/);
    const totals=current.locator('.cp-fixed-current-readouts');
    await figureEquals(readout(totals,'Saldo'),'R$ 360,00');
    await figureEquals(readout(totals,'Valor aplicado'),'R$ 300,00');
    await figureEquals(readout(totals,'Rendimento no saldo'),'+R$ 60,00');
    const period=current.locator('.cp-fixed-period');
    await textMatches(period,/15\/10\/2026/);
    await textMatches(period,/Rendimento em 12 meses.*R\$\s*30,00/);
    await textMatches(period,/Rentabilidade em 12 meses.*8,50%/);
    assert.doesNotMatch(await period.innerText(),/20\/10\/2026/,'The 12-month report date must remain distinct from the newer balance date');
    assert.match(await tip(current,'Rendimento no saldo'),/saldo.*aplicado|aplicado.*saldo/i);
    check('fixed-income-balance-gain-and-twelve-month-gain-have-distinct-values-and-reference-dates');

    const contributions=current.getByRole('table',{name:'Aportes do pós-fixado',exact:true});
    await contributions.waitFor({state:'visible'});
    const contributionRows=contributions.locator('tbody tr');
    assert.equal(await contributionRows.count(),3,'All three dated contributions remain visible without expanding details');
    for(const [date,applied,value,earned,percent] of [
      ['01/02/2023','R$ 100,00','R$ 130,00','+R$ 30,00','30,00%'],
      ['01/06/2024','R$ 110,00','R$ 132,00','+R$ 22,00','20,00%'],
      ['10/01/2025','R$ 90,00','R$ 98,00','+R$ 8,00','8,89%'],
    ]){
      const row=contributions.getByRole('row',{name:new RegExp(escapeRegExp(date))});
      for(const amount of [applied,value,earned,percent])assert.ok(compact(await row.innerText()).includes(compact(amount)),`Contribution ${date} contains ${amount}`);
    }
    const older=page.locator('.cp-fixed-previous');
    await textMatches(older.locator(':scope > summary'),/Registro anterior de 03\/02\/2020/);
    if(!await older.evaluate(element=>element.open))await older.locator(':scope > summary').click();
    await textMatches(older,/R\$\s*50,00/);
    await textMatches(older,/R\$\s*4,00/);
    assert.doesNotMatch(await older.innerText(),/R\$\s*360,00/);
    check('fixed-income-three-contributions-are-visible-and-previous-broker-values-remain-separate');
    await noOverflow(page,'fixed-income-desktop-no-overflow');
    await sectionShot(page,'desktop-fixed-income-observation',current);

    await page.setViewportSize({width:375,height:812});
    await noOverflow(page,'fixed-income-mobile375-no-overflow');
    await contributions.waitFor({state:'visible'});
    await figureEquals(readout(totals,'Rendimento no saldo'),'+R$ 60,00');
    await textMatches(period,/15\/10\/2026/);
    const mobileTable=await contributions.evaluate(table=>{
      const wrap=table.closest('.table-wrap');
      wrap.scrollLeft=wrap.scrollWidth;
      const last=table.querySelector('tbody tr:last-child td:last-child').getBoundingClientRect(),box=wrap.getBoundingClientRect();
      const result={viewport:innerWidth,wrapLeft:box.left,wrapRight:box.right,scrollWidth:wrap.scrollWidth,clientWidth:wrap.clientWidth,lastRight:last.right};
      wrap.scrollLeft=0;return result;
    });
    assert.ok(mobileTable.wrapLeft>=0&&mobileTable.wrapRight<=mobileTable.viewport+1&&mobileTable.lastRight<=mobileTable.wrapRight+1,`The contribution table may scroll internally, but its return column must be reachable ${JSON.stringify(mobileTable)}`);
    await sectionShot(page,'mobile-fixed-income-observation375',current);
    check('fixed-income-mobile-shows-current-balances-period-and-contributions-without-outer-overflow');

    await navigate(page,'Visão geral');
    await figureEquals(region(page,'Posição na corretora').locator('.figure-hero'),'R$ 201,00');
    await combinedTotals(page);
    const result=region(page,'Resultado da carteira');
    await noOverflow(page,'fixed-income-combined-result-mobile375-no-overflow');
    await sectionShot(page,'mobile-fixed-income-result375',result);
    await page.setViewportSize({width:1440,height:1000});
    await noOverflow(page,'fixed-income-combined-result-desktop-no-overflow');
    await sectionShot(page,'desktop-fixed-income-result',result);
    await page.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-other');
    await figureEquals(readout(region(page,'Resultado da carteira'),'Total aplicado'),'R$ 480,00');
    await figureEquals(readout(region(page,'Resultado da carteira'),'Rendimentos acumulados'),'+R$ 20,00');
    assert.equal(await page.locator('.performance-fixed-income-row, .performance-fixed-income-scope').count(),0,'A different wallet must not inherit the fixed-income observation');
    await page.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-original');
    await combinedTotals(page);
    await page.locator('.performance-fixed-income-row').getByRole('button',{name:'Pós-fixado',exact:true}).click();
    await title(page,'FIIs e posições');
    assert.equal(await tab(page,'Posição').getAttribute('aria-selected'),'true');
    assert.deepEqual(await page.evaluate(()=>globalThis.__uiTest.snapshot),original,'A supplemental observation must not rewrite historical position, cash, or financial entries');
    const writes=await page.evaluate(()=>globalThis.__uiTest.calls.filter(({command})=>/^(?:save_|upsert_|delete_|remove_|import_|export_|start_chat_turn$|create_conversation$)/.test(command)));
    assert.deepEqual(writes,[]);await noUnexpectedCommands(page);
    check('fixed-income-result-includes-current-gain-without-duplicating-12-month-gain-contributions-old-balance-or-ledger');
    check('fixed-income-result-row-opens-position-and-wallet-switch-does-not-leak-fixed-income');
    await fixture.context.close();

    const api=await openPage('fixed-income-api',{width:1440,height:1000}),apiPage=api.page;
    const apiOriginal=await apiPage.evaluate(()=>structuredClone(globalThis.__uiTest.snapshot));
    const apiExpected={income:'+R$ 124,56',result:'−R$ 775,54',fiiIncome:'+R$ 30,00',fiiResult:'−R$ 870,10'};
    await combinedTotals(apiPage,apiExpected);
    await textMatches(region(apiPage,'Resultado da carteira').locator('.performance-difference'),/−R\$ 950,10/);
    assert.match(await apiPage.locator('.performance-bars svg desc').textContent(),/FIIs: −R\$\s*870,10\. CDI: \+R\$\s*80,00/,'The FII/CDI comparison retains only reconciled FII income, while wallet totals include fixed income');
    await region(apiPage,'Resultado da carteira').getByRole('button',{name:'Atualizar resultado',exact:true}).click();
    await apiPage.getByText('Consulta concluída. Confira as datas dos dados.',{exact:true}).waitFor();
    await combinedTotals(apiPage,apiExpected);
    assert.deepEqual(await apiPage.evaluate(()=>globalThis.__uiTest.snapshot),apiOriginal);
    assert.equal((await calls(apiPage,'save_record')).length,0);
    check('fixed-income-and-reconciled-B3-payments-join-native-wallet-totals-without-compounding-on-refresh');
    await noUnexpectedCommands(apiPage);await api.context.close();

    for(const mode of ['fixed-income-legacy','fixed-income-none']){
      const scenario=await openPage(mode,{width:1440,height:1000}),scope=region(scenario.page,'Resultado da carteira');
      if(mode==='fixed-income-legacy'){
        for(const label of ['Total aplicado','Rendimentos acumulados','Resultado da carteira'])await figureEquals(readout(scope,label),'—');
        assert.deepEqual((await scope.locator('.performance-fixed-income-row td').allInnerTexts()).map(normalize),['—','—','—']);
        await textMatches(scope.locator('.performance-fixed-income-scope'),/Faltam dados/);
        assert.equal(await scope.locator('.performance-bars svg').count(),1,'Unknown fixed-income basis must not hide the available FII comparison');
        check('fixed-income-legacy-positive-balance-without-basis-keeps-total-cost-income-and-result-unknown');
      }else{
        await figureEquals(readout(scope,'Total aplicado'),'R$ 2.240,00');
        await figureEquals(readout(scope,'Rendimentos acumulados'),'+R$ 45,56');
        await figureEquals(readout(scope,'Resultado da carteira'),'−R$ 854,54');
        assert.equal(await scope.locator('.performance-fixed-income-row, .performance-fixed-income-scope').count(),0);
        check('fixed-income-empty-native-component-adds-neither-a-row-nor-a-duplicate-broker-balance');
      }
      await noUnexpectedCommands(scenario.page);await scenario.context.close();
    }
  }

  async function verifyRecentMovements(){
    const fixture=await openPage('ledger-recent',{width:1440,height:1000}),page=fixture.page;
    const original=await page.evaluate(()=>structuredClone(globalThis.__uiTest.snapshot));
    const scope=page.locator('.ov-recent'),purchases=scope.locator('.ov-recent-purchases'),income=scope.locator('.ov-recent-income');
    await purchases.getByRole('heading',{name:'Últimas compras',exact:true}).waitFor();
    await income.getByRole('heading',{name:'Últimos rendimentos',exact:true}).waitFor();
    assert.equal(await purchases.locator('tbody tr').count(),4);assert.equal(await income.locator('tbody tr').count(),4);
    for(const part of [purchases,income])assert.equal(await part.evaluate(element=>!!element.closest('details')),false);
    const byMonth=(part,month)=>part.locator('tbody tr').filter({hasText:month});
    const january=purchases.locator('tbody tr').filter({has:page.locator('time[datetime="2020-01-21"]')});
    await textMatches(january,/21\/01\/2020 TEST11 R\$ 100,00 10 cotas × R\$ 10,00/);
    const march=purchases.locator('tbody tr').filter({has:page.locator('time[datetime="2020-03-11"]')});
    await textMatches(march,/11\/03\/2020 TEST11 R\$ 25,00 2 cotas × R\$ 12,50/);
    await textMatches(byMonth(purchases,'jun/20'),/dia não informado.*R\$ 30,00 3 cotas/);
    assert.equal(await byMonth(purchases,'jun/20').locator('time').count(),0,'A legacy purchase month must not acquire an invented day');
    await textMatches(byMonth(income,'jun/20'),/R\$ 1,50/);await textMatches(byMonth(income,'mai/20'),/R\$ 3,75/);
    assert.equal(await byMonth(purchases,'mai/20').count(),0,'A recent payment must never create a purchase');
    assert.equal(await byMonth(purchases,'fev/20').count(),0);
    assert.equal(await byMonth(income,'jul/20').count(),0,'An informational price is not a payment');
    check('overview-recent-purchases-retain-older-real-buys-even-when-the-latest-records-are-income-only');
    assert.equal(await march.count(),1);assert.equal(await byMonth(income,'mar/20').count(),1);
    await textEquals(byMonth(income,'mar/20').locator('.ov-received'),'R$ 6,00');
    await textMatches(byMonth(purchases,'ago/20'),/Preço a conferir.*1 cota.*preço registrado R\$ 0,00/);
    assert.equal(await byMonth(income,'ago/20').count(),0);
    assert.notEqual(await purchases.locator('.ov-purchase').first().evaluate(element=>getComputedStyle(element).color),await income.locator('.ov-received').first().evaluate(element=>getComputedStyle(element).color));
    assert.match(await tip(purchases,'Data da compra'),/dia exato.*apenas o mês.*nenhum dia.*presumido/i);
    check('overview-recent-mixed-record-shows-once-per-list-and-zero-price-purchase-needs-confirmation');
    await noOverflow(page,'overview-recent-movements-desktop-no-overflow');await sectionShot(page,'desktop-overview-recent-movements',scope);
    await page.setViewportSize({width:375,height:812});
    await noOverflow(page,'overview-recent-movements-mobile375-no-overflow');await sectionShot(page,'mobile-overview-recent-movements375',scope);
    await purchases.getByRole('button',{name:'Ver todas',exact:true}).click();await title(page,'FIIs e posições');
    assert.equal(await tab(page,'Histórico').getAttribute('aria-selected'),'true');
    await navigate(page,'Visão geral');
    await page.locator('.ov-recent-income').getByRole('button',{name:'Ver todos',exact:true}).click();await title(page,'FIIs e posições');
    assert.equal(await tab(page,'Histórico').getAttribute('aria-selected'),'true');
    assert.deepEqual(await page.evaluate(()=>globalThis.__uiTest.snapshot),original);
    assert.equal((await calls(page,'save_record')).length,0);await noUnexpectedCommands(page);
    check('overview-recent-purchase-and-income-actions-reach-the-preserved-history-without-writing-records');
    await fixture.context.close();
  }

  if(focusedFixedIncome){
    await verifyFixedIncomeObservation();await verifyRecentMovements();assert.deepEqual(browserErrors,[]);
    await fs.writeFile(path.join(output,reportName),JSON.stringify({status:'PASS',uiVersion,syntheticDataOnly:true,checks,browserErrors,screenshots},null,2));
    console.log(JSON.stringify({status:'PASS',mode:'fixed-income',checks:checks.length,screenshots:screenshots.length,browserErrors:browserErrors.length}));return;
  }

  if(focusedIncomePerformance){
    await verifyAutomaticIncomePerformance();assert.deepEqual(browserErrors,[]);
    await fs.writeFile(path.join(output,reportName),JSON.stringify({status:'PASS',uiVersion,syntheticDataOnly:true,checks,browserErrors,screenshots},null,2));
    console.log(JSON.stringify({status:'PASS',mode:'income-performance',checks:checks.length,screenshots:screenshots.length,browserErrors:browserErrors.length}));return;
  }

  if(focusedIncomeApi){
    await verifyAutomaticMonthlyIncome();assert.deepEqual(browserErrors,[]);
    await fs.writeFile(path.join(output,reportName),JSON.stringify({status:'PASS',uiVersion,syntheticDataOnly:true,checks,browserErrors,screenshots},null,2));
    console.log(JSON.stringify({status:'PASS',mode:'income-api',checks:checks.length,screenshots:screenshots.length,browserErrors:browserErrors.length}));return;
  }
  if(focusedIncome){
    await verifyMonthlyIncome();assert.deepEqual(browserErrors,[]);
    await fs.writeFile(path.join(output,reportName),JSON.stringify({status:'PASS',uiVersion,syntheticDataOnly:true,checks,browserErrors,screenshots},null,2));
    console.log(JSON.stringify({status:'PASS',mode:'income',checks:checks.length,screenshots:screenshots.length,browserErrors:browserErrors.length}));return;
  }
  if(focusedRowMotion){
    const fixture=await openPage('cash-motion',{width:1440,height:1000},{reducedMotion:'no-preference'});
    assert.equal(await fixture.page.evaluate(()=>matchMedia('(prefers-reduced-motion: reduce)').matches),false);
    const motion=[];
    motion.push({table:'portfolio',samples:await rowBackgroundMotion(fixture.page,fixture.page.locator('.performance-portfolio-table'),'portfolio-table')});
    motion.push({table:'FII-breakdown',samples:await rowBackgroundMotion(fixture.page,fixture.page.locator('.performance-table'),'FII-breakdown-table')});
    await noUnexpectedCommands(fixture.page);assert.deepEqual(browserErrors,[]);await fixture.context.close();
    const report={status:'PASS',uiVersion,syntheticDataOnly:true,reducedMotion:'no-preference',checks,browserErrors,motion};
    await fs.writeFile(path.join(output,reportName),JSON.stringify(report,null,2));
    console.log(JSON.stringify({status:'PASS',mode:'rows-motion',checks:checks.length,rows:motion.reduce((sum,table)=>sum+new Set(table.samples.map(row=>row.row)).size,0),transitions:motion.reduce((sum,table)=>sum+table.samples.length,0),frames:motion.flatMap(table=>table.samples).reduce((sum,row)=>sum+row.frames.length,0),browserErrors:browserErrors.length}));
    return;
  }
  if(!focusedCompact&&!focusedShell&&!focusedPublicData)await verifyInvestmentProfiles();
  if(focusedProfile){
    assert.deepEqual(browserErrors,[]);
    await fs.writeFile(path.join(output,reportName),JSON.stringify({status:'PASS',uiVersion,syntheticDataOnly:true,checks,browserErrors,screenshots},null,2));
    console.log(JSON.stringify({status:'PASS',mode:'profile',checks:checks.length,screenshots:screenshots.length,browserErrors:browserErrors.length}));return;
  }
  if(!focusedCompact&&!focusedShell)await verifyAutomaticPublicData();
  if(focusedPublicData){
    assert.deepEqual(browserErrors,[]);
    await fs.writeFile(path.join(output,reportName),JSON.stringify({status:'PASS',uiVersion,syntheticDataOnly:true,checks,browserErrors,screenshots},null,2));
    console.log(JSON.stringify({status:'PASS',mode:'public-data',checks:checks.length,screenshots:screenshots.length,browserErrors:browserErrors.length}));return;
  }
  await verifyCompactLayout();
  if(focusedCompact){
    assert.deepEqual(browserErrors,[]);
    await fs.writeFile(path.join(output,reportName),JSON.stringify({status:'PASS',uiVersion,syntheticDataOnly:true,checks,browserErrors,screenshots},null,2));
    console.log(JSON.stringify({status:'PASS',mode:'compact',checks:checks.length,screenshots:screenshots.length,browserErrors:browserErrors.length}));return;
  }
  await verifyShell();
  if(focusedShell){
    assert.deepEqual(browserErrors,[]);
    await fs.writeFile(path.join(output,reportName),JSON.stringify({status:'PASS',uiVersion,syntheticDataOnly:true,checks,browserErrors,screenshots},null,2));
    console.log(JSON.stringify({status:'PASS',mode:'shell',checks:checks.length,screenshots:screenshots.length,browserErrors:browserErrors.length}));return;
  }
  await verifyMonthlyIncome();
  await verifyAutomaticMonthlyIncome();
  await verifyAutomaticIncomePerformance();
  await verifyFixedIncomeObservation();
  await verifyRecentMovements();

  // ---------- Recorded cash income joins wallet totals; CDI simulations never do ----------
  {
    const combined=await openPage('cash-combined',{width:1440,height:1000});
    const page=combined.page;
    const result=()=>region(page,'Resultado da carteira');
    const globalTotals=async(applied,income,total)=>{
      await figureEquals(readout(result(),'Total aplicado'),applied);
      await figureEquals(readout(result(),'Rendimentos acumulados'),income);
      await figureEquals(readout(result(),'Resultado da carteira'),total);
    };
    const composition=result().getByRole('table',{name:'Composição do resultado da carteira: FIIs, caixinhas e pós-fixado',exact:true});
    await globalTotals('R$ 300,00','+R$ 35,00','+R$ 25,00');
    for(const [name,values] of [['FIIs',['R$ 100,00','+R$ 5,00','−R$ 5,00']],['Caixinhas',['R$ 200,00','+R$ 30,00','+R$ 30,00']],['Total',['R$ 300,00','+R$ 35,00','+R$ 25,00']]])
      assert.deepEqual((await composition.getByRole('row',{name:new RegExp('^'+name+' ')}).locator('td').allInnerTexts()).map(normalize),values);
    await textEquals(result().locator('.performance-chart-scope'),'Apenas FIIs. Caixinhas e pós-fixado entram nos totais acima, sem participar deste gráfico.');
    const graph=result().locator('.performance-bars svg');
    assert.match(await graph.locator('desc').textContent(),/FIIs: −R\$\s*5,00\. CDI: \+R\$\s*8,00/);
    assert.equal(Number(await graph.locator('.performance-bar.fii').getAttribute('y')),Number(await graph.locator('.performance-zero').getAttribute('y1')));
    assert.equal((await calls(page,'refresh_fii_performance')).length,0);
    assert.equal((await calls(page,'simulate_cash_pocket')).length,0);
    await noOverflow(page,'combined-cash-and-FII-desktop-no-overflow');
    await rowPaintContinuity(page,composition,'desktop-portfolio-table',{capture:true});
    await rowPaintContinuity(page,result().locator('.performance-table'),'desktop-FII-breakdown-table',{capture:true});
    await shot(page,'desktop-overview-with-cash');
    check('wallet-totals-combine-FII-loss-and-recorded-cash-gain-without-changing-FII-CDI-bars');
    await page.setViewportSize({width:375,height:812});
    await globalTotals('R$ 300,00','+R$ 35,00','+R$ 25,00');
    await noOverflow(page,'combined-cash-and-FII-mobile375-no-overflow');
    await rowPaintContinuity(page,composition,'mobile375-portfolio-table');
    await rowPaintContinuity(page,result().locator('.performance-table'),'mobile375-FII-breakdown-table');
    assert.deepEqual(await composition.locator('td.num').evaluateAll(cells=>cells.filter(cell=>cell.scrollWidth>cell.clientWidth+1).map(cell=>cell.textContent)),[]);
    await shot(page,'mobile-overview-with-cash375');
    await page.setViewportSize({width:1440,height:1000});

    await navigate(page,'Caixinhas CDI');
    const cashReadouts=()=>page.locator('.cash-actual-readouts');
    await figureEquals(readout(cashReadouts(),'Total nas caixinhas'),'R$ 220,00');
    await figureEquals(readout(cashReadouts(),'Total aplicado'),'R$ 200,00');
    await figureEquals(readout(cashReadouts(),'Rendimento acumulado'),'+R$ 30,00');
    assert.equal((await calls(page,'get_cash_performance')).at(-1).args.walletId,'wallet-original');
    const edit=page.getByRole('button',{name:'Editar caixinha Reserva sintética',exact:true});
    const dialog=page.getByRole('dialog',{name:'Editar caixinha',exact:true});
    await edit.click();await dialog.waitFor();
    assert.deepEqual(await Promise.all(['Total aplicado (R$)','Total resgatado (R$)','Data do saldo'].map(label=>dialog.getByLabel(label,{exact:true}).inputValue())),['200','10','2020-02-03']);
    assert.equal(await dialog.getByLabel('Data do saldo',{exact:true}).getAttribute('required'),null,'Balance date is optional unless reconciliation is being supplied');
    await noOverflow(page,'cash-reconciliation-form-desktop-no-overflow');await shot(page,'desktop-cash-reconciliation-form',{fullPage:false});
    await page.setViewportSize({width:375,height:812});await noOverflow(page,'cash-reconciliation-form-mobile375-no-overflow');await shot(page,'mobile-cash-reconciliation-form375',{fullPage:false});
    await page.setViewportSize({width:1440,height:1000});
    const beforePartial=(await calls(page,'save_cash_pocket')).length;
    await dialog.getByLabel('Data do saldo',{exact:true}).fill('');
    await dialog.getByRole('button',{name:'Salvar caixinha',exact:true}).click();
    await textMatches(dialog.getByRole('alert'),/preencha total aplicado, total resgatado e data do saldo juntos/);
    assert.equal((await calls(page,'save_cash_pocket')).length,beforePartial);
    await dialog.getByLabel('Total aplicado (R$)',{exact:true}).fill('');
    await dialog.getByLabel('Total resgatado (R$)',{exact:true}).fill('');
    await dialog.getByRole('button',{name:'Salvar caixinha',exact:true}).click();
    await textMatches(dialog.getByRole('alert'),/use “Remover dados de rendimento”/);
    assert.equal((await calls(page,'save_cash_pocket')).length,beforePartial);
    check('cash-reconciliation-requires-all-three-fields-and-cannot-be-cleared-by-blank-input');
    await dialog.getByLabel('Total aplicado (R$)',{exact:true}).fill('200');
    await dialog.getByLabel('Total resgatado (R$)',{exact:true}).fill('10');
    await dialog.getByLabel('Data do saldo',{exact:true}).fill('2020-02-03');
    await dialog.getByLabel('Saldo atual (R$)',{exact:true}).fill('230');
    const savedReads=(await calls(page,'get_portfolio')).length;
    await page.evaluate(()=>{globalThis.__uiTest.deferSaves=true;});
    await dialog.getByRole('button',{name:'Salvar caixinha',exact:true}).evaluate(button=>{button.click();button.click();});
    await page.waitForFunction(()=>globalThis.__uiTest.saveWaiters.length===1);
    assert.equal((await calls(page,'save_cash_pocket')).length,beforePartial+1);
    assert.equal(await dialog.getAttribute('aria-busy'),'true');
    assert.equal(await dialog.getByRole('button',{name:'Cancelar',exact:true}).isDisabled(),true);
    assert.equal(await dialog.getByLabel('Total aplicado (R$)',{exact:true}).isDisabled(),true);
    assert.equal(await page.getByRole('combobox',{name:'Carteira',exact:true}).evaluate(element=>{element.focus();return document.activeElement===element;}),false,'The native modal must keep the wallet selector outside the focusable surface');
    await page.keyboard.press('Escape');assert.equal(await dialog.evaluate(element=>element.open),true);
    await page.evaluate(()=>{const test=globalThis.__uiTest;test.deferSaves=false;test.saveWaiters.splice(0).forEach(resolve=>resolve());});
    await dialog.waitFor({state:'detached'});await dismissToast(page,'Caixinha salva na carteira selecionada.');
    await figureEquals(readout(cashReadouts(),'Rendimento acumulado'),'+R$ 40,00');
    assert.ok((await calls(page,'get_portfolio')).length>savedReads,'Saving a pocket must refresh the wallet snapshot');
    await navigate(page,'Visão geral');
    await globalTotals('R$ 300,00','+R$ 45,00','+R$ 35,00');
    check('cash-save-is-serialized-keeps-wallet-locked-and-refreshes-global-income-and-result');

    // A new pocket can omit the reconciliation entirely; an explicit zero is a known value.
    await navigate(page,'Caixinhas CDI');
    await page.getByRole('button',{name:'Nova caixinha',exact:true}).click();
    const newPocket=page.getByRole('dialog',{name:'Nova caixinha',exact:true});await newPocket.waitFor();
    for(const label of ['Total aplicado (R$)','Total resgatado (R$)','Data do saldo'])assert.equal(await newPocket.getByLabel(label,{exact:true}).inputValue(),'');
    await newPocket.getByLabel('Nome da caixinha',{exact:true}).fill('Reserva zero');
    await newPocket.getByLabel('Instituição',{exact:true}).fill('Banco de teste');
    await newPocket.getByLabel('Saldo atual (R$)',{exact:true}).fill('0');
    await newPocket.getByLabel('Total aplicado (R$)',{exact:true}).fill('0');
    await newPocket.getByLabel('Total resgatado (R$)',{exact:true}).fill('0');
    await newPocket.getByRole('button',{name:'Salvar caixinha',exact:true}).click();
    await textMatches(newPocket.getByRole('alert'),/preencha total aplicado, total resgatado e data do saldo juntos/);
    await newPocket.getByLabel('Data do saldo',{exact:true}).fill('2020-02-03');
    await newPocket.getByRole('button',{name:'Salvar caixinha',exact:true}).click();await newPocket.waitFor({state:'detached'});await dismissToast(page,'Caixinha salva na carteira selecionada.');
    const zeroSave=(await calls(page,'save_cash_pocket')).at(-1).args.input;
    assert.deepEqual([zeroSave.balance,zeroSave.totalDeposits,zeroSave.totalWithdrawals,zeroSave.balanceDate],['0','0','0','2020-02-03']);
    const zeroRow=page.locator('.cash-table tbody tr').filter({has:page.locator('.cash-name',{hasText:'Reserva zero'})});
    await textEquals(zeroRow.locator('.cash-applied'),'R$ 0,00');await textEquals(zeroRow.locator('td').nth(3),'R$ 0,00');
    assert.doesNotMatch(await zeroRow.innerText(),/Complete|—/);
    await navigate(page,'Visão geral');await globalTotals('R$ 300,00','+R$ 45,00','+R$ 35,00');
    check('new-cash-pocket-explicit-zero-is-known-and-does-not-change-existing-wallet-totals');
    await noUnexpectedCommands(page);await combined.context.close();

    const legacy=await openPage('cash-legacy',{width:1440,height:1000});
    const legacyPage=legacy.page;
    const legacyResult=()=>region(legacyPage,'Resultado da carteira');
    for(const label of ['Total aplicado','Rendimentos acumulados','Resultado da carteira'])await figureEquals(readout(legacyResult(),label),'—');
    assert.equal(await legacyResult().locator('.performance-bars svg').count(),1,'Unknown cash basis must not hide an otherwise complete FII comparison');
    assert.match(await legacyResult().locator('.performance-bars svg desc').textContent(),/FIIs: −R\$\s*5,00/);
    assert.deepEqual((await legacyResult().locator('.performance-portfolio-table tbody tr').filter({has:legacyPage.getByRole('button',{name:'Caixinhas',exact:true})}).locator('td').allInnerTexts()).map(normalize),['—','—','—']);
    await legacyResult().getByRole('button',{name:'Completar caixinhas',exact:true}).click();await title(legacyPage,'Caixinhas CDI');
    await figureEquals(readout(legacyPage.locator('.cash-actual-readouts'),'Total nas caixinhas'),'R$ 220,00');
    await figureEquals(readout(legacyPage.locator('.cash-actual-readouts'),'Total aplicado'),'—');
    await figureEquals(readout(legacyPage.locator('.cash-actual-readouts'),'Rendimento acumulado'),'—');
    await textMatches(legacyPage.locator('.cash-table tbody tr').first().locator('td').nth(1),/^R\$ 220,00 Data não informada$/);
    const legacyDialog=legacyPage.getByRole('dialog',{name:'Editar caixinha',exact:true});
    await legacyPage.getByRole('button',{name:'Editar caixinha Reserva sintética',exact:true}).click();await legacyDialog.waitFor();
    for(const label of ['Total aplicado (R$)','Total resgatado (R$)','Data do saldo'])assert.equal(await legacyDialog.getByLabel(label,{exact:true}).inputValue(),'');
    await legacyDialog.getByLabel(/^Observações/).fill('Observação alterada sem criar aplicações ou resgates.');
    await legacyDialog.getByRole('button',{name:'Salvar caixinha',exact:true}).click();await legacyDialog.waitFor({state:'detached'});await dismissToast(legacyPage,'Caixinha salva na carteira selecionada.');
    const omitted=(await calls(legacyPage,'save_cash_pocket')).at(-1).args.input;
    for(const key of ['totalDeposits','totalWithdrawals','balanceDate','clearReconciliation'])assert.equal(Object.hasOwn(omitted,key),false);
    check('legacy-cash-unknown-is-not-zero-and-ordinary-save-omits-reconciliation-fields');
    await legacyPage.getByRole('button',{name:'Editar caixinha Reserva sintética',exact:true}).click();await legacyDialog.waitFor();
    await legacyDialog.getByLabel('Total aplicado (R$)',{exact:true}).fill('200');await legacyDialog.getByLabel('Total resgatado (R$)',{exact:true}).fill('10');await legacyDialog.getByLabel('Data do saldo',{exact:true}).fill('2020-02-03');
    await legacyDialog.getByRole('button',{name:'Salvar caixinha',exact:true}).click();await legacyDialog.waitFor({state:'detached'});await dismissToast(legacyPage,'Caixinha salva na carteira selecionada.');
    await navigate(legacyPage,'Visão geral');
    await figureEquals(readout(legacyResult(),'Total aplicado'),'R$ 300,00');await figureEquals(readout(legacyResult(),'Rendimentos acumulados'),'+R$ 35,00');await figureEquals(readout(legacyResult(),'Resultado da carteira'),'+R$ 25,00');
    assert.equal(await legacyResult().getByRole('button',{name:'Completar caixinhas',exact:true}).count(),0);
    await navigate(legacyPage,'Caixinhas CDI');
    await legacyPage.getByRole('button',{name:'Editar caixinha Reserva sintética',exact:true}).click();await legacyDialog.waitFor();
    await legacyDialog.getByRole('button',{name:'Remover dados de rendimento',exact:true}).click();
    await legacyDialog.getByRole('status').filter({hasText:'serão removidos'}).waitFor();
    await legacyDialog.getByRole('button',{name:'Salvar caixinha',exact:true}).click();await legacyDialog.waitFor({state:'detached'});await dismissToast(legacyPage,'Caixinha salva na carteira selecionada.');
    const cleared=(await calls(legacyPage,'save_cash_pocket')).at(-1).args.input;
    assert.equal(cleared.clearReconciliation,true);for(const key of ['totalDeposits','totalWithdrawals','balanceDate'])assert.equal(Object.hasOwn(cleared,key),false);
    assert.equal(cleared.balance,'220');assert.equal(cleared.cdiPercent,'120');
    await navigate(legacyPage,'Visão geral');for(const label of ['Total aplicado','Rendimentos acumulados','Resultado da carteira'])await figureEquals(readout(legacyResult(),label),'—');
    check('completing-legacy-cash-updates-global-totals-and-explicit-removal-restores-unknown');
    await noUnexpectedCommands(legacyPage);await legacy.context.close();

    for(const [mode,applied,income,total] of [['cash-only','R$ 200,00','+R$ 30,00','+R$ 30,00'],['cash-negative','R$ 200,00','−R$ 15,00','−R$ 25,00'],['cash-partial','—','—','—']]){
      const fixture=await openPage(mode,{width:1440,height:1000});const page=fixture.page;const result=region(page,'Resultado da carteira');
      await figureEquals(readout(result,'Total aplicado'),applied);await figureEquals(readout(result,'Rendimentos acumulados'),income);await figureEquals(readout(result,'Resultado da carteira'),total);
      if(mode==='cash-only'){
        assert.equal(await result.locator('.performance-bars svg').count(),0);
        await result.getByText('Sem compras de FIIs registradas para comparar com o CDI.',{exact:true}).waitFor();
        assert.equal((await calls(page,'refresh_fii_performance')).length,0);
      }
      if(mode==='cash-negative'){
        const row=result.locator('.performance-portfolio-table tbody tr').filter({has:page.getByRole('button',{name:'Caixinhas',exact:true})});
        await textEquals(row.locator('td').nth(1),'−R$ 20,00');assert.match(await row.locator('td').nth(1).getAttribute('class'),/value-negative/);
      }
      if(mode==='cash-partial'){
        await result.getByRole('button',{name:'Completar caixinhas',exact:true}).waitFor();
        await navigate(page,'Caixinhas CDI');await figureEquals(readout(page.locator('.cash-actual-readouts'),'Total nas caixinhas'),'R$ 270,00');await figureEquals(readout(page.locator('.cash-actual-readouts'),'Rendimento acumulado'),'—');
        await textEquals(page.locator('.cash-table tbody tr').first().locator('td').nth(3),'+R$ 30,00');
      }
      check(mode==='cash-only'?'cash-only-wallet-does-not-require-quotes-or-CDI-benchmark':mode==='cash-negative'?'negative-recorded-cash-income-reduces-wallet-income-and-result':'mixed-known-and-unknown-cash-never-promotes-partial-income-to-complete-total');
      await noUnexpectedCommands(page);await fixture.context.close();
    }
    const zoom=await openPage('cash-zoom-200',{width:720,height:500});
    await noOverflow(zoom.page,'combined-cash-and-FII-zoom200-no-overflow');
    await rowPaintContinuity(zoom.page,zoom.page.locator('.performance-portfolio-table'),'zoom200-portfolio-table');
    await rowPaintContinuity(zoom.page,zoom.page.locator('.performance-table'),'zoom200-FII-breakdown-table');
    await shot(zoom.page,'zoom-200-equivalent-overview-with-cash');await zoom.context.close();
  }

  // ---------- Desktop: every screen with the default wallet ----------
  const desktop=await openPage('desktop',{width:1440,height:1000});
  let page=desktop.page;
  await updateBlocked(page,false);
  await page.keyboard.press('Tab');const skip=page.getByRole('link',{name:'Pular para o conteúdo',exact:true});assert.equal(await isFocused(skip),true);
  await page.keyboard.press('Enter');await focusReturnsTo(page,page.locator('#main-content'));check('skip-link-moves-focus-to-main-content');
  await verifyOverview(page);check('overview-position-and-cost-history-stay-separated');
  await verifyWalletTotals(page);check('overview-wallet-table-keeps-funds-and-cash-per-wallet');
  await verifyImportNotice(page,region(page,'Posição na corretora'),position.warnings,{capture:true});check('position-import-notice-preserves-source-and-links-to-affected-values');
  assert.equal(await banner(page).getByRole('button',{name:'Importar posição',exact:true}).isEnabled(),true);
  assert.equal(await banner(page).getByRole('button',{name:'Novo lançamento',exact:true}).isEnabled(),true);
  await textEquals(banner(page).locator('.topbar-context'),'Carteira de teste · Corretora de teste');
  await noOverflow(page,'desktop-overview-no-overflow');await shot(page,'desktop-overview');await shot(page,'desktop-overview-top',{fullPage:false});
  await shot(page,'desktop-overview-expanded',{collapsed:false});

  // FIIs e posições: two tabs. With an imported position it opens on Posição, which no longer repeats the overview summary.
  await navigate(page,'FIIs e posições');
  assert.deepEqual(await page.getByRole('tablist',{name:'Seções de FIIs e posições',exact:true}).getByRole('tab').allInnerTexts(),['Posição','Histórico']);
  assert.equal(await tab(page,'Posição').getAttribute('aria-selected'),'true');assert.equal(await panel(page,'Histórico').count(),0);
  const detail=panel(page,'Posição');
  assert.equal(await detail.locator('.cp-holdings thead th').count(),4);
  await textEquals(detail.locator('.group-head p'),'Corretora de teste · ref. 03/02/2020, atualizada às 02:00');
  assert.equal(await detail.locator('.figure-hero, .cp-composition').count(),0);
  await figureEquals(readout(detail,'Valor de mercado'),'R$ 140,00');
  assert.match(await tip(detail,'Valor de mercado'),/Não é uma cotação atualizada automaticamente\.$/);
  assert.deepEqual(await detail.locator('.cp-value-note .num').allInnerTexts(),['R$\u00a0139,00','R$\u00a0140,00']);
  await figureEquals(readout(detail,'Custo inferido'),'R$ 1.040,00');
  await figureEquals(readout(detail,'Resultado sobre o custo'),'−R$ 900,00');
  const holdings=detail.getByRole('table',{name:/^Posição de FIIs e fundos listados em 03\/02\/2020/});
  assert.equal(await holdings.locator('tbody tr').count(),2);
  await textMatches(holdings.locator('tbody tr').first(),/^TEST11 10 R\$ 90,00 −R\$ 910,00$/);
  await textMatches(detail.locator('.cp-holding-costs tbody tr').first(),/^TEST11 R\$ 100,00 R\$ 9,00 R\$ 1\.000,00$/);
  await textEquals(detail.locator('.cp-product'),'Fundo sintético pós-fixado');
  await textEquals(fact(detail,'Saldo bruto'),'R$ 50,00');
  await textEquals(fact(detail,'Saldo em conta'),'R$ 10,00');
  assert.equal(await detail.locator('.block-foot, .cp-callout').count(),0,'Position tab should keep explanations in tooltips');
  check('portfolio-position-tab-detail-without-repeated-summary-or-footnotes');
  await noOverflow(page,'desktop-portfolio-position-no-overflow');await shot(page,'desktop-portfolio-position');
  await shot(page,'desktop-portfolio-position-expanded',{collapsed:false});

  // Position import: cancel, Escape and Fechar return focus; an error keeps the data; busy blocks closing; success refreshes.
  const updatePosition=detail.getByRole('button',{name:'Atualizar posição',exact:true});
  const positionDialog=importDialog(page);
  const positionPath=positionDialog.getByLabel('Arquivo JSON da posição');
  for(const close of [()=>positionDialog.getByRole('button',{name:'Cancelar',exact:true}).click(),()=>page.keyboard.press('Escape'),()=>positionDialog.getByRole('button',{name:'Fechar',exact:true}).click()]){
    await updatePosition.click();await positionDialog.waitFor();
    await updateBlocked(page,true);
    await page.waitForFunction(element=>element===document.activeElement,await positionPath.elementHandle());
    assert.match(await positionPath.inputValue(),/synthetic-position\.json$/);
    await close();await positionDialog.waitFor({state:'detached'});await focusReturnsTo(page,updatePosition);
    await updateBlocked(page,false);
  }
  assert.equal((await calls(page,'import_broker_position')).length,0);check('position-modal-cancel-Escape-and-close-restore-focus');
  await updatePosition.click();await positionDialog.waitFor();
  await textMatches(positionDialog.locator('.modal-header p'),/^Destino: Carteira de teste\./);
  await positionPath.fill('');assert.equal(await positionDialog.getByRole('button',{name:'Salvar posição',exact:true}).isDisabled(),true);
  await positionDialog.getByRole('button',{name:'Procurar arquivo',exact:true}).click();assert.match(await positionPath.inputValue(),/synthetic-position\.json$/);
  await positionDialog.getByRole('button',{name:'Salvar posição',exact:true}).click();
  await textEquals(positionDialog.getByRole('alert'),'Arquivo sintético inválido.');
  await textEquals(fact(detail,'Saldo em conta'),'R$ 10,00');
  await shot(page,'desktop-import-error',{fullPage:false});check('position-import-error-keeps-current-data');
  const readsBefore=(await calls(page,'get_portfolio')).length;
  await page.evaluate(()=>{globalThis.__uiTest.deferImports=true;});
  await positionDialog.getByRole('button',{name:'Salvar posição',exact:true}).click();
  await page.waitForFunction(()=>globalThis.__uiTest.importWaiters.length===1);
  assert.equal(await positionDialog.getAttribute('aria-busy'),'true');
  for(const name of ['Fechar','Cancelar','Salvar posição','Procurar arquivo'])assert.equal(await positionDialog.getByRole('button',{name,exact:true}).isDisabled(),true,`${name} should be disabled while importing`);
  assert.equal(await positionPath.isDisabled(),true);assert.equal(await page.getByRole('combobox',{name:'Carteira',exact:true}).isDisabled(),true);
  assert.equal(await positionDialog.getByRole('alert').count(),0);
  await page.keyboard.press('Escape');assert.equal(await positionDialog.evaluate(element=>element.open),true);
  await page.mouse.click(8,8);assert.equal(await positionDialog.evaluate(element=>element.open),true);
  await shot(page,'desktop-modal-busy',{fullPage:false});
  await page.evaluate(()=>{const test=globalThis.__uiTest;test.deferImports=false;test.importWaiters.splice(0).forEach(resolve=>resolve());});
  await positionDialog.waitFor({state:'detached'});
  await textEquals(fact(detail,'Saldo em conta'),'R$ 15,00');
  await toast(page,'Posição completa salva').waitFor();
  assert.ok((await calls(page,'get_portfolio')).length>readsBefore,'Successful position import did not refresh the portfolio');
  assert.deepEqual((await calls(page,'import_broker_position')).map(call=>call.args.walletId),['wallet-original','wallet-original']);
  await focusReturnsTo(page,updatePosition);
  check('position-import-busy-blocks-closing-and-success-refreshes');
  await shot(page,'desktop-import-success');await dismissToast(page,'Posição completa salva');

  // Tabs: arrows, Home and End switch and move focus; only the active tab is in the Tab order.
  await tab(page,'Posição').focus();await page.keyboard.press('ArrowRight');
  assert.equal(await tab(page,'Histórico').getAttribute('aria-selected'),'true');assert.equal(await isFocused(tab(page,'Histórico')),true);
  assert.equal(await tab(page,'Posição').getAttribute('tabindex'),'-1');assert.equal(await panel(page,'Posição').count(),0);
  await page.keyboard.press('Home');assert.equal(await tab(page,'Posição').getAttribute('aria-selected'),'true');
  await page.keyboard.press('End');assert.equal(await tab(page,'Histórico').getAttribute('aria-selected'),'true');
  const historyPanel=panel(page,'Histórico');await historyPanel.waitFor();
  await openExtra(historyPanel);
  check('portfolio-tabs-keyboard-arrows-home-end');
  // The successful import above did not touch the purchase history.
  await textEquals(historyPanel.locator('.group-head p'),'Não somado ao patrimônio');
  const ledger=()=>page.getByRole('table',{name:/^Fundos do histórico com cotas/});
  assert.equal(await ledger().locator('tbody tr').count(),2);
  const search=page.getByRole('textbox',{name:'Buscar fundo no histórico'});
  await search.fill('TEST11');assert.equal(await ledger().locator('tbody tr').count(),1);assert.equal(await ledger().locator('tfoot').count(),0);
  await search.fill('ZZZZ');await page.getByRole('heading',{name:'Nenhum fundo encontrado',exact:true}).waitFor();
  await search.fill('');assert.equal(await ledger().locator('tbody tr').count(),2);await textMatches(ledger().locator('tfoot tr'),/^Total do histórico 12 R\$ 11,00 R\$ 11,00$/);
  await openExtra(historyPanel);
  await textEquals(historyPanel.locator('.fund-costs-table tfoot tr'),'Total das compras R$ 1.040,00');
  await page.getByRole('button',{name:'Ver lançamentos de MOCK11',exact:true}).click();
  await page.getByRole('heading',{name:'Lançamentos · MOCK11',exact:true}).waitFor();
  assert.equal(await page.getByRole('button',{name:'Ver lançamentos de MOCK11',exact:true}).getAttribute('aria-current'),'true');
  await page.getByRole('button',{name:'Ver lançamentos de TEST11',exact:true}).click();await page.getByRole('heading',{name:'Lançamentos · TEST11',exact:true}).waitFor();
  assert.equal(await historyPanel.locator('.block-foot').count(),0);
  check('portfolio-history-tab-search-and-fund-selection');
  await noOverflow(page,'desktop-portfolio-history-no-overflow');await shot(page,'desktop-portfolio-history');
  await shot(page,'desktop-portfolio-history-expanded',{collapsed:false});

  // Fund and entry forms: initial focus, Escape, busy submit and focus return.
  const newFund=banner(page).getByRole('button',{name:'Novo fundo',exact:true});
  const fundDialog=page.getByRole('dialog',{name:'Novo fundo',exact:true});
  await newFund.click();await fundDialog.waitFor();
  await page.waitForFunction(element=>element===document.activeElement,await fundDialog.getByLabel('Ticker').elementHandle());
  assert.equal(normalize(await page.locator(`[id="${await fundDialog.getAttribute('aria-describedby')}"]`).innerText()),'Os lançamentos são registrados depois, no histórico.');
  await fundDialog.getByLabel('Ticker').fill('new11');assert.equal(await fundDialog.getByLabel('Ticker').inputValue(),'NEW11');await fundDialog.getByLabel('Segmento').fill('Teste');
  await page.keyboard.press('Escape');await fundDialog.waitFor({state:'detached'});await focusReturnsTo(page,newFund);
  assert.equal((await calls(page,'upsert_fund')).length,0);
  await newFund.click();await fundDialog.waitFor();await fundDialog.getByLabel('Ticker').fill('NEW11');await fundDialog.getByLabel('Nome do fundo').fill('Fundo criado no teste');
  await page.evaluate(()=>{globalThis.__uiTest.deferSaves=true;});
  await fundDialog.getByRole('button',{name:'Salvar fundo',exact:true}).click();
  await page.waitForFunction(()=>globalThis.__uiTest.saveWaiters.length===1);
  assert.equal(await fundDialog.getByRole('button',{name:'Salvando…',exact:true}).isDisabled(),true);
  await shot(page,'desktop-fund-busy',{fullPage:false});
  await page.evaluate(()=>{const test=globalThis.__uiTest;test.deferSaves=false;test.saveWaiters.splice(0).forEach(resolve=>resolve());});
  await fundDialog.waitFor({state:'detached'});await focusReturnsTo(page,newFund);await dismissToast(page,'Fundo salvo na carteira selecionada.');
  const fundSaves=await calls(page,'upsert_fund');assert.equal(fundSaves.length,1);assert.equal(fundSaves[0].args.walletId,'wallet-original');assert.equal(fundSaves[0].args.input.ticker,'NEW11');assert.equal(fundSaves[0].args.input.name,'Fundo criado no teste');
  await page.getByRole('button',{name:'Ver lançamentos de NEW11',exact:true}).waitFor();
  check('fund-form-focus-Escape-busy-submit-and-focus-return');
  const newEntry=banner(page).getByRole('button',{name:'Novo lançamento',exact:true});
  const entryDialog=page.getByRole('dialog',{name:'Novo lançamento · TEST11',exact:true});
  await newEntry.click();await entryDialog.waitFor();
  await page.waitForFunction(element=>element===document.activeElement,await entryDialog.getByLabel('Mês de referência').elementHandle());
  assert.equal(await entryDialog.getByLabel('Preço por cota (R$)').inputValue(),'100');
  assert.match(await tip(entryDialog,'Total recebido (R$)'),/prevalece, inclusive 0.*Vazio: calculado.*fim do mês.*rendimento por cota/i);
  await entryDialog.getByRole('button',{name:'Fechar',exact:true}).click();await entryDialog.waitFor({state:'detached'});await focusReturnsTo(page,newEntry);
  check('entry-form-defaults-hint-and-close');

  // Cash pockets: the manual CDI typed during a Banco Central lookup wins; balances stay per wallet.
  await navigate(page,'Caixinhas CDI');
  const pockets=()=>page.getByRole('table',{name:/^Caixinhas da carteira Carteira de teste:/});
  const pocketRow=()=>pockets().getByRole('row',{name:/^Reserva sintética/});
  await textMatches(pocketRow().locator('td').nth(1),/^R\$ 1\.234,56 03\/02\/2020$/);
  await textEquals(pocketRow().locator('.cash-applied'),'R$ 1.200,00');
  await textEquals(pocketRow().locator('td').nth(3),'+R$ 34,56');
  await textEquals(pocketRow().locator('td').nth(4),'120%');
  await textMatches(page.locator('.cash-details'),/10% a\.a\. · Manual · 03\/02\/2020/);
  await figureEquals(readout(page.locator('.cash-actual-readouts'),'Total nas caixinhas'),'R$ 1.234,56');
  const editPocket=page.getByRole('button',{name:'Editar caixinha Reserva sintética',exact:true});
  const pocketDialog=page.getByRole('dialog',{name:'Editar caixinha',exact:true});
  await editPocket.click();await pocketDialog.waitFor();
  await page.waitForFunction(element=>element===document.activeElement,await pocketDialog.getByLabel('Nome da caixinha').elementHandle());
  await pocketDialog.getByLabel('Saldo atual (R$)').fill('1500');
  await pocketDialog.getByRole('button',{name:'Buscar CDI no Banco Central',exact:true}).click();
  await pocketDialog.getByRole('button',{name:'Consultando CDI…',exact:true}).waitFor();
  assert.equal(await pocketDialog.getByRole('button',{name:'Salvar caixinha',exact:true}).isDisabled(),true);
  await pocketDialog.getByLabel('CDI anual (% a.a.)').fill('9.5');
  await pocketDialog.getByText('A referência foi editada durante a consulta. Mantivemos os valores que você digitou.',{exact:true}).waitFor();
  assert.equal(await pocketDialog.getByLabel('CDI anual (% a.a.)').inputValue(),'9.5');
  await pocketDialog.getByRole('button',{name:'Salvar caixinha',exact:true}).click();await pocketDialog.waitFor({state:'detached'});
  await dismissToast(page,'Caixinha salva na carteira selecionada.');
  const cashSave=(await calls(page,'save_cash_pocket')).at(-1).args;assert.equal(cashSave.walletId,'wallet-original');assert.equal(cashSave.input.rateSource,'manual');assert.equal(cashSave.input.rateSourceUrl,null);assert.equal(cashSave.input.balance,'1500');assert.equal(cashSave.input.cdiPercent,'120');assert.equal(cashSave.input.annualCdiRate,'9.5');
  await textMatches(pocketRow().locator('td').nth(1),/^R\$ 1\.500,00 03\/02\/2020$/);
  await textEquals(pocketRow().locator('td').nth(4),'120%');
  await textEquals(pocketRow().locator('td').nth(3),'+R$ 300,00');
  assert.deepEqual([cashSave.input.totalDeposits,cashSave.input.totalWithdrawals,cashSave.input.balanceDate],['1200','0','2020-02-03']);
  await openExtra(page);
  await textMatches(page.locator('.cash-details'),/9,5% a\.a\. · Manual/);
  check('cash-CDI-fetch-race-keeps-manual-rate-and-remuneration');
  const simulation=region(page,'Simular rendimento');
  await simulation.getByRole('button',{name:'Calcular rendimento bruto',exact:true}).click();
  await textMatches(simulation.locator('.cash-sim-lead'),/^Saldo bruto estimado após 252 dias úteis R\$ ?1\.246,90$/);
  await openExtra(simulation);
  await textMatches(simulation.locator('.cash-sim-details'),/120% de um CDI de 10% a\.a\./);
  await simulation.getByText('Cenário bruto fictício de teste.',{exact:true}).waitFor();
  assert.deepEqual((await calls(page,'simulate_cash_pocket')).at(-1).args,{walletId:'wallet-original',input:{pocketId:'pocket-a',businessDays:252}});
  check('cash-simulation-uses-selected-wallet-and-pocket');
  await noOverflow(page,'desktop-cash-no-overflow');await shot(page,'desktop-cash');
  await shot(page,'desktop-cash-expanded',{collapsed:false});

  // Market: opening reads only the local cache; failed refreshes keep the saved indicator and headline.
  await navigate(page,'Mercado');
  await textMatches(banner(page).locator('.topbar-context'),/^Carteira de teste/);
  const headline=page.getByRole('link',{name:`${news.items[0].title} (abre no navegador)`,exact:true});
  await headline.waitFor();
  const cdiCard=page.getByRole('group',{name:'CDI',exact:true});
  await textMatches(cdiCard.locator('strong'),/^8,20 ?% a\.a\.$/);
  assert.equal((await page.evaluate(()=>globalThis.__uiTest.calls.filter(call=>['refresh_economic_indicators','refresh_news','refresh_quotes','refresh_cvm_data'].includes(call.command)))).length,0);
  assert.equal((await calls(page,'get_quote_cache')).at(-1).args.walletId,'wallet-original');
  assert.equal((await calls(page,'get_cvm_cache')).at(-1).args.walletId,'wallet-original');
  const quotesSection=region(page,'Cotações dos fundos');const cvmSection=region(page,'Informes mensais da CVM');
  await rowPaintContinuity(page,quotesSection.locator('.market-quotes-table'),'desktop-market-row-headers');
  await textMatches(quotesSection.locator('tbody tr').first(),/^TEST11 R\$ 8,99/);
  assert.equal(await quotesSection.getByRole('button',{name:'Atualizar cotações',exact:true}).isDisabled(),true);
  await quotesSection.getByText('Exibindo cotação salva',{exact:true}).waitFor();
  await cvmSection.getByText('jan/20',{exact:true}).waitFor();
  await textMatches(cvmSection.locator('.market-cvm-table tbody tr').first().locator('td').nth(1),/^R\$ 10,25/);
  check('market-opens-from-local-cache-only');
  await cvmSection.getByRole('button',{name:'Vincular CNPJ de MOCK11',exact:true}).click();
  const mappingForm=cvmSection.getByRole('form',{name:'CNPJ de MOCK11',exact:true});
  await updateBlocked(page,true);
  await mappingForm.getByLabel(/^CNPJ do fundo MOCK11/).fill('123');
  await mappingForm.getByRole('button',{name:'Salvar vínculo',exact:true}).click();
  await mappingForm.getByRole('alert').waitFor();assert.equal((await calls(page,'save_cvm_mapping')).length,0);
  await mappingForm.getByLabel(/^CNPJ do fundo MOCK11/).fill('11.222.333/0001-81');
  await mappingForm.getByRole('button',{name:'Salvar vínculo',exact:true}).click();
  await mappingForm.waitFor({state:'detached'});
  assert.deepEqual((await calls(page,'save_cvm_mapping')).at(-1).args,{symbol:'MOCK11',cnpj:'11.222.333/0001-81'});
  await cvmSection.getByText('Vínculo alterado. Atualize a CVM para consultar esse CNPJ.',{exact:true}).waitFor();
  await cvmSection.getByRole('button',{name:'Atualizar CVM',exact:true}).click();
  await cvmSection.getByText('Fundo CVM sintético B',{exact:true}).waitFor();
  await cvmSection.getByText('CVM temporariamente indisponível no teste.',{exact:true}).waitFor();
  assert.equal(await cvmSection.getByText('jan/20',{exact:true}).count(),2);
  await cvmSection.getByRole('button',{name:'Vincular CNPJ de MOCK11',exact:true}).click();
  await mappingForm.getByRole('button',{name:'Remover vínculo',exact:true}).click();
  await mappingForm.waitFor({state:'detached'});
  assert.equal(await cvmSection.getByText('Fundo CVM sintético B',{exact:true}).count(),0);
  assert.deepEqual((await calls(page,'save_cvm_mapping')).at(-1).args,{symbol:'MOCK11',cnpj:null});
  check('CVM-mapping-validation-explicit-refresh-and-cache-on-partial-failure');
  await banner(page).getByRole('button',{name:'Atualizar indicadores',exact:true}).click();
  await textEquals(cdiCard.getByRole('alert'),'Fonte temporariamente indisponível no teste. Exibindo a referência salva.');
  await page.getByRole('status').filter({hasText:'Atualização parcial.'}).waitFor();
  await textMatches(cdiCard.locator('strong'),/^8,20/);
  await region(page,'Notícias').getByRole('button',{name:'Atualizar notícias',exact:true}).click();
  await textEquals(region(page,'Notícias').getByRole('alert'),'Sem conexão sintética. As manchetes salvas continuam disponíveis.');
  assert.equal(await headline.count(),1);
  check('market-refresh-errors-keep-cached-indicator-and-news');
  await noOverflow(page,'desktop-market-no-overflow');await shot(page,'desktop-market');
  await shot(page,'desktop-market-expanded',{collapsed:false});

  // Projections: calculate, change a premise, then Recalcular from the stale banner.
  await navigate(page,'Projeções');
  const result=region(page,'Resultado estimado');
  await result.getByRole('heading',{name:'Nenhuma projeção calculada',exact:true}).waitFor();
  await page.getByRole('button',{name:'Calcular projeção',exact:true}).click();
  await openExtra(page);
  const monthly=page.getByRole('table',{name:'Projeção mês a mês',exact:true});
  await monthly.locator('tbody tr').nth(1).waitFor();
  assert.equal(await monthly.locator('tbody tr').count(),2);
  await textMatches(monthly.locator('tbody tr').nth(1),/^abr\/20 15 2 R\$ 12,00 R\$ 14,00 R\$ 22,00$/);
  await figureEquals(result.locator('.figure-hero'),'R$ 14,00');
  await textEquals(result.locator('.block-meta'),'Carteira completa · 24 meses · sem aportes · com reinvestimento');
  const projectionArgs=(await calls(page,'project_income')).at(-1).args;assert.equal(projectionArgs.walletId,'wallet-original');
  const {fundId,...projectionInput}=projectionArgs.input;assert.equal(fundId,undefined);assert.deepEqual(projectionInput,{months:24,monthlyCotas:'0',window:6,reinvest:true});
  await page.getByLabel('Horizonte').selectOption('12');
  const stale=result.getByRole('status').filter({hasText:'Premissas alteradas. Os números abaixo são do cálculo anterior.'});
  await stale.waitFor();assert.match(await page.locator('.projection-page').getAttribute('class'),/\bis-stale\b/);
  await shot(page,'desktop-projections-stale');
  await stale.getByRole('button',{name:'Recalcular',exact:true}).click();
  await stale.waitFor({state:'detached'});
  await focusReturnsTo(page,result.getByRole('heading',{name:'Resultado estimado',exact:true}));
  await textEquals(result.locator('.block-meta'),'Carteira completa · 12 meses · sem aportes · com reinvestimento');
  assert.equal((await calls(page,'project_income')).at(-1).args.input.months,12);
  check('projection-values-stale-premises-and-recalculate');
  await noOverflow(page,'desktop-projections-no-overflow');await shot(page,'desktop-projections');
  await shot(page,'desktop-projections-expanded',{collapsed:false});

  // Assistant: switching provider keeps the local conversation.
  await navigate(page,'Assistente');
  const conversation=page.getByRole('complementary',{name:'Conversas',exact:true}).getByRole('button',{name:/^Conversa sintética preservada/});
  await page.getByText('Resposta sintética preservada entre provedores.',{exact:true}).waitFor();
  assert.equal(await conversation.getAttribute('aria-current'),'true');
  await page.getByText('Preparar análise da carteira',{exact:true}).waitFor();
  await page.getByText('CLI disponível',{exact:false}).waitFor();
  await page.getByRole('combobox',{name:'Provedor de IA',exact:true}).selectOption('claude');
  await page.getByRole('textbox',{name:'Sua pergunta',exact:true}).fill('Rascunho sintético que deve impedir reinício.');
  await updateBlocked(page,true);
  await page.getByRole('textbox',{name:'Sua pergunta',exact:true}).fill('');
  await updateBlocked(page,false);
  check('updater-delays-restart-for-dialogs-mappings-and-unsent-chat');
  assert.equal(await page.getByText('Pergunta sintética sobre a carteira de teste.',{exact:true}).count(),1);
  assert.equal(await page.getByText('Resposta sintética preservada entre provedores.',{exact:true}).count(),1);
  assert.equal(await conversation.getAttribute('aria-current'),'true');
  assert.equal(await page.evaluate(()=>localStorage.getItem('ivestfii.assistant.provider')),'claude');
  assert.ok((await calls(page,'get_messages')).every(call=>call.args.conversationId==='conversation-a'));
  check('assistant-provider-switch-keeps-local-history');
  await noOverflow(page,'desktop-assistant-no-overflow');await shot(page,'desktop-assistant');

  // Settings: preferences save through IPC; the position import is reachable from here too.
  await navigate(page,'Configurações');
  const localKey=page.getByLabel(/^Chave da brapi/);
  await localKey.waitFor();assert.equal(await localKey.inputValue(),'');assert.equal(await localKey.getAttribute('type'),'password');
  await localKey.fill('synthetic-ui-key');
  await updateBlocked(page,true);
  await page.getByRole('button',{name:'Salvar chave',exact:true}).click();
  await page.getByText('Chave local configurada',{exact:true}).waitFor();assert.equal(await localKey.inputValue(),'');
  await updateBlocked(page,false);
  assert.equal(await page.locator('body').innerText().then(text=>text.includes('synthetic-ui-key')),false);
  assert.equal(await page.evaluate(()=>Object.values(localStorage).some(value=>value.includes('synthetic-ui-key'))),false);
  check('brapi-local-password-field-clears-after-save-and-never-uses-localstorage');
  assert.equal(await page.getByRole('switch',{name:'Fechar para a bandeja',exact:true}).isChecked(),true);
  assert.equal(await page.getByRole('switch',{name:'Atualizações automáticas',exact:true}).isChecked(),true);
  await page.getByRole('switch',{name:'Atualizações automáticas',exact:true}).uncheck();
  await page.getByRole('switch',{name:'Iniciar com o Windows',exact:true}).check();
  await page.getByText('Alterações não salvas.',{exact:true}).waitFor();
  await updateBlocked(page,true);
  await page.getByRole('button',{name:'Salvar preferências',exact:true}).click();
  await dismissToast(page,'Preferências do aplicativo salvas.');await page.getByText('Sem alterações.',{exact:true}).waitFor();
  await updateBlocked(page,false);
  assert.ok((await calls(page,'set_update_activity')).every(call=>Object.keys(call.args).length===1&&typeof call.args.blocked==='boolean'));
  check('updater-activity-sends-only-boolean-and-protects-unsaved-settings-and-key');
  assert.deepEqual((await calls(page,'save_settings')).at(-1).args.settings,{autostart:true,startMinimized:false,closeToTray:true,autoUpdate:false});
  await page.getByText('X:\\fixture\\portfolio.sqlite',{exact:true}).waitFor();
  await page.getByRole('button',{name:'Importar posição',exact:true}).click();await positionDialog.waitFor();await page.keyboard.press('Escape');await positionDialog.waitFor({state:'detached'});
  check('settings-save-preferences-and-reach-position-import');
  const updates=region(page,'Atualizações');
  await updates.getByText('Versão 0.1.0',{exact:true}).waitFor();
  await updates.getByRole('status').filter({hasText:'Você está usando a versão mais recente.'}).waitFor();
  await textMatches(updates.locator('time'),/04\/02\/2020/);
  await page.evaluate(()=>{globalThis.__uiTest.update={...globalThis.__uiTest.update,status:'disabled',error:'Esta instalação não possui um canal de atualização configurado.'};});
  await updates.getByText('Esta instalação não possui um canal de atualização configurado.',{exact:true}).waitFor();
  assert.equal(await updates.getByRole('button',{name:'Verificar atualizações',exact:true}).isDisabled(),true);
  assert.equal(await updates.getByText('Você está usando a versão mais recente.',{exact:true}).count(),0);
  check('updater-without-channel-does-not-claim-latest-version');
  await page.evaluate(()=>{globalThis.__uiTest.update={...globalThis.__uiTest.update,status:'error',error:'Não foi possível consultar o canal no teste.'};globalThis.__uiTest.failUpdateCheck=true;});
  await updates.getByText('Não foi possível consultar o canal no teste.',{exact:true}).waitFor();
  await updates.getByRole('button',{name:'Verificar atualizações',exact:true}).click();
  await updates.getByRole('alert').filter({hasText:'Servidor de atualização indisponível no teste.'}).waitFor();
  await page.evaluate(()=>{globalThis.__uiTest.failUpdateCheck=false;globalThis.__uiTest.update={...globalThis.__uiTest.update,status:'downloading',availableVersion:'0.2.0',downloadedBytes:2621440,totalBytes:10485760,error:null};});
  await updates.getByRole('progressbar',{name:'Download da atualização',exact:true}).waitFor();
  assert.equal(await updates.getByRole('progressbar').getAttribute('value'),'2621440');
  await updates.getByText('25% · 2,5 MB de 10 MB',{exact:true}).waitFor();
  assert.equal(await updates.getByRole('button',{name:'Verificar atualizações',exact:true}).isDisabled(),true);
  check('updater-retry-errors-and-download-progress');
  await page.evaluate(()=>{globalThis.__uiTest.update={...globalThis.__uiTest.update,status:'waiting_for_idle'};});
  await updates.getByRole('status').filter({hasText:'Aguardando as tarefas e os formulários em uso'}).waitFor();
  assert.equal(await updates.getByRole('button',{name:'Verificar atualizações',exact:true}).isEnabled(),true);
  await page.evaluate(()=>{globalThis.__uiTest.failUpdateInstall=true;});
  await updates.getByRole('button',{name:'Aplicar e reiniciar',exact:true}).click();
  await updates.getByRole('alert').filter({hasText:'Aguarde o assistente encerrar a tarefa antes de reiniciar.'}).waitFor();
  assert.equal(await updates.getByRole('button',{name:'Aplicar e reiniciar',exact:true}).isEnabled(),true);
  await page.evaluate(()=>{globalThis.__uiTest.failUpdateInstall=false;});
  await updates.getByRole('button',{name:'Aplicar e reiniciar',exact:true}).click();
  await updates.getByRole('status').filter({hasText:'Aplicando a versão 0.2.0'}).waitFor();
  check('updater-waiting-with-auto-update-off-allows-manual-retry-after-busy');
  await page.evaluate(()=>{globalThis.__uiTest.update={...globalThis.__uiTest.update,status:'ready',downloadedBytes:10485760};globalThis.__uiTest.failUpdateInstall=true;});
  await updates.getByRole('button',{name:'Aplicar e reiniciar',exact:true}).click();
  await updates.getByRole('alert').filter({hasText:'Aguarde o assistente encerrar a tarefa antes de reiniciar.'}).waitFor();
  await page.evaluate(()=>{globalThis.__uiTest.failUpdateInstall=false;});
  await updates.getByRole('button',{name:'Aplicar e reiniciar',exact:true}).click();
  await updates.getByRole('status').filter({hasText:'Aplicando a versão 0.2.0'}).waitFor();
  assert.equal((await calls(page,'install_pending_update')).length,4);
  await page.evaluate(()=>{globalThis.__uiTest.update={...globalThis.__uiTest.update,status:'up_to_date',availableVersion:null,downloadedBytes:0,totalBytes:null};});
  await updates.getByText('Você está usando a versão mais recente.',{exact:true}).waitFor();
  check('updater-ready-explicit-restart-and-active-chat-error');
  await noOverflow(page,'desktop-settings-no-overflow');await shot(page,'desktop-settings');

  // Authenticated coverage is simulated here; live validation still needs the user's own key.
  await navigate(page,'Mercado');
  const updateReads=(await calls(page,'get_update_status')).length;
  await page.waitForTimeout(2150);
  assert.equal((await calls(page,'get_update_status')).length,updateReads);
  check('updater-polling-stops-when-settings-close');
  await quotesSection.getByRole('button',{name:'Atualizar cotações',exact:true}).click();
  await textMatches(quotesSection.locator('tbody tr').first(),/^TEST11 R\$ 9,25/);
  await quotesSection.getByText('Ativo não incluído no plano sintético.',{exact:true}).waitFor();
  await textMatches(quotesSection.getByRole('status'),/^Consulta parcial/);
  assert.equal((await calls(page,'refresh_quotes')).at(-1).args.walletId,'wallet-original');
  assert.equal(await page.evaluate(()=>globalThis.__uiTest.snapshot.brokerPosition.overview.totalWithCash),'206');
  assert.equal(await page.evaluate(()=>globalThis.__uiTest.snapshot.funds[0].averagePrice),'100');
  check('quotes-refresh-distinguishes-plan-coverage-and-never-modifies-portfolio');
  await page.evaluate(()=>{globalThis.__uiTest.deferMarket=true;});
  await quotesSection.getByRole('button',{name:'Atualizar cotações',exact:true}).click();
  await page.waitForFunction(()=>globalThis.__uiTest.marketWaiters.length===1);
  await page.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-other');
  await quotesSection.locator('.market-quotes-table').getByRole('rowheader',{name:'BANK11',exact:true}).waitFor();
  await page.evaluate(()=>{globalThis.__uiTest.marketWaiters.splice(0).forEach(resolve=>resolve());});
  await page.waitForTimeout(150);
  assert.equal(await quotesSection.locator('.market-quotes-table').getByRole('rowheader',{name:'TEST11',exact:true}).count(),0);
  assert.equal((await calls(page,'get_quote_cache')).at(-1).args.walletId,'wallet-other');
  check('late-quote-refresh-cannot-overwrite-new-wallet-cache');
  await page.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-original');
  await quotesSection.locator('.market-quotes-table').getByRole('rowheader',{name:'TEST11',exact:true}).waitFor();
  await navigate(page,'Configurações');
  assert.equal(await page.getByRole('switch',{name:'Atualizações automáticas',exact:true}).isChecked(),false);
  check('automatic-update-preference-survives-settings-remount');
  assert.equal(await page.getByLabel(/^Chave da brapi/).inputValue(),'');
  await page.getByRole('button',{name:'Remover chave local',exact:true}).click();
  await page.getByText('Sem chave configurada',{exact:true}).waitFor();
  assert.equal((await calls(page,'clear_brapi_key')).length,1);
  check('brapi-key-is-not-reread-and-local-removal-is-explicit');

  // Wallets: funds, position and cash pockets follow the selected wallet.
  await navigate(page,'Visão geral');
  await page.locator('.ov-recent-income').getByRole('button',{name:'Ver todos',exact:true}).click();await title(page,'FIIs e posições');
  assert.equal(await tab(page,'Histórico').getAttribute('aria-selected'),'true','A direct history action should reach its intended list');
  await navigate(page,'Visão geral');await navigate(page,'FIIs e posições');
  assert.equal(await tab(page,'Posição').getAttribute('aria-selected'),'true','The sidebar should open the imported broker position regardless of the last history action');
  check('portfolio-position-default-and-explicit-history-navigation-stay-distinct');
  await navigate(page,'Visão geral');
  await page.getByRole('button',{name:'Abrir carteira Carteira Nubank teste',exact:true}).click();
  await title(page,'FIIs e posições');
  // A wallet without any imported position opens its available historical records.
  assert.equal(await tab(page,'Histórico').getAttribute('aria-selected'),'true');check('portfolio-without-position-opens-its-available-history');
  await openTab(page,'Histórico');
  await page.getByRole('button',{name:'Ver lançamentos de BANK11',exact:true}).waitFor();
  assert.equal(await page.getByRole('button',{name:'Ver lançamentos de TEST11',exact:true}).count(),0);
  assert.equal(await panel(page,'Histórico').locator('.group-head p').count(),0);
  await openTab(page,'Posição');
  await region(page,'Posição na corretora').getByRole('heading',{name:'Nenhuma posição importada',exact:true}).waitFor();
  assert.equal(await panel(page,'Posição').locator('.cp-listed').count(),0);
  assert.equal(await page.getByRole('combobox',{name:'Carteira',exact:true}).inputValue(),'wallet-other');
  await textEquals(banner(page).locator('.topbar-context'),'Carteira Nubank teste · Nubank teste');
  await shot(page,'desktop-wallet-other');
  await navigate(page,'Caixinhas CDI');
  await page.getByRole('table',{name:/^Caixinhas da carteira Carteira Nubank teste:/}).getByRole('row',{name:/^Reserva da outra carteira/}).waitFor();
  assert.equal(await page.getByRole('button',{name:'Editar caixinha Reserva sintética',exact:true}).count(),0);
  assert.equal((await calls(page,'get_cash_pockets')).at(-1).args.walletId,'wallet-other');
  await page.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-original');
  await textMatches(pockets().getByRole('row',{name:/^Reserva sintética/}),/R\$ 1\.500,00/);
  assert.equal(await page.getByRole('button',{name:'Editar caixinha Reserva da outra carteira',exact:true}).count(),0);
  check('wallet-selection-keeps-funds-position-and-cash-separated');
  await sidebar(page).getByRole('link',{name:'IvestFII · Visão geral',exact:true}).click();await title(page,'Visão geral');
  // The imported position (cash 10 → 15) reached the overview summary.
  await figureEquals(region(page,'Posição na corretora').locator('.figure-hero'),'R$ 206,00');
  await noUnexpectedCommands(page);check('desktop-navigation-covers-every-screen');
  await desktop.context.close();

  // Full result includes price changes and registered distributions; the CDI uses the same purchases.
  const comparison=await openPage('performance',{width:1440,height:1000});page=comparison.page;
  const performanceSection=region(page,'Resultado da carteira');
  const performanceTotal=()=>performanceSection.getByRole('table',{name:'Composição do resultado total dos FIIs',exact:true}).getByRole('row',{name:/^= Resultado total/}).locator('td');
  const refreshPerformance=()=>performanceSection.getByRole('button',{name:'Atualizar resultado',exact:true});
  await figureEquals(performanceTotal(),'−R$ 889,10');
  assert.equal((await calls(page,'refresh_fii_performance')).length,0,'Opening Overview must only read cached performance');
  const breakdown=performanceSection.getByRole('table',{name:'Composição do resultado total dos FIIs',exact:true});
  for(const [label,value] of [['Valor de mercado dos FIIs','R$ 139,90'],['− Total investido','R$ 1.040,00'],['= Variação das cotas','−R$ 900,10'],['+ Rendimentos registrados','R$ 11,00'],['= Resultado total','−R$ 889,10']])
    await textEquals(breakdown.getByRole('row',{name:new RegExp('^'+escapeRegExp(label))}).locator('td'),value);
  assert.match(await tip(breakdown,'+ Rendimentos registrados'),/calculados.*estimativas/);
  await performanceSection.getByText('Comparação estimada · compras no fim do mês; no mês da cotação, até a cotação.',{exact:true}).waitFor();
  const graph=performanceSection.locator('.performance-bars svg');
  const zero=Number(await graph.locator('.performance-zero').getAttribute('y1'));
  assert.equal(Number(await graph.locator('.performance-bar.fii').getAttribute('y')),zero,'A loss must start at zero and extend below it');
  assert.ok(Number(await graph.locator('.performance-bar.fii').getAttribute('height'))>0);
  assert.ok(Number(await graph.locator('.performance-bar.cdi').getAttribute('y'))<zero,'A positive benchmark must extend above zero');
  await graph.focus();await page.keyboard.press('End');await textMatches(performanceSection.locator('.performance-point'),/^100% do CDI: \+R\$ 80,00/);
  await page.keyboard.press('Home');await textMatches(performanceSection.locator('.performance-point'),/^FIIs: −R\$ 889,10/);
  await performanceSection.locator('.performance-chart-table > summary').click();
  const alternative=performanceSection.getByRole('table',{name:'Resultados acumulados comparados com as mesmas compras',exact:true});
  await textMatches(alternative.getByRole('row',{name:/^FIIs/}),/^FIIs −R\$ 889,10 −85,49%$/);
  assert.equal(await performanceSection.locator('.performance-source a').getAttribute('href'),performance.cdiSourceUrl);
  await performanceSection.locator('.performance-source a').click();
  assert.equal((await calls(page,'open_public_source')).at(-1).args.url,performance.cdiSourceUrl);
  check('full-performance-composition-negative-baseline-keyboard-and-public-CDI-source');

  // The total cost and distributions stay the same, but a fresh local snapshot must trigger another cache read.
  const cacheReads=(await calls(page,'get_fii_performance')).length;
  const networkReads=(await calls(page,'refresh_fii_performance')).length;
  await page.evaluate(()=>{globalThis.__uiTest.failImports=0;globalThis.__uiTest.snapshot.funds[0].totalCotas='11';globalThis.__uiTest.snapshot.totals.cotas='13';});
  await banner(page).getByRole('button',{name:'Importar posição',exact:true}).click();
  const sameCostImport=importDialog(page);await sameCostImport.getByLabel('Arquivo JSON da posição',{exact:true}).fill('X:\\fixture\\synthetic-position.json');
  await sameCostImport.getByRole('button',{name:'Salvar posição',exact:true}).click();await sameCostImport.waitFor({state:'detached'});
  await page.waitForFunction(count=>globalThis.__uiTest.calls.filter(call=>call.command==='get_fii_performance').length>count,cacheReads);
  assert.equal((await calls(page,'refresh_fii_performance')).length,networkReads);
  check('performance-reloads-local-snapshot-with-unchanged-cost-and-distributions');

  await page.evaluate(()=>{globalThis.__uiTest.performance={...globalThis.__uiTest.performance,marketValue:'1160',priceResult:'120',totalResult:'131',totalReturnPercent:'12.5961538462',difference:'51'};});
  await refreshPerformance().click();await figureEquals(performanceTotal(),'+R$ 131,00');
  await figureEquals(readout(performanceSection,'Total aplicado'),'R$ 2.240,00');
  await figureEquals(readout(performanceSection,'Rendimentos acumulados'),'+R$ 45,56');
  await figureEquals(readout(performanceSection,'Resultado da carteira'),'+R$ 165,56');
  assert.ok(Number(await graph.locator('.performance-bar.fii').getAttribute('height'))>Number(await graph.locator('.performance-bar.cdi').getAttribute('height')),'The portfolio bar must exceed CDI when its result is higher');
  await textMatches(performanceSection.locator('.performance-difference'),/\+R\$ 51,00/);
  await shot(page,'desktop-fii-performance-positive');
  check('full-performance-positive-result-exceeds-CDI-with-signed-difference');

  await page.evaluate(()=>{globalThis.__uiTest.failPerformance=true;});
  await refreshPerformance().click();await performanceSection.getByRole('alert').filter({hasText:'Fontes de resultado temporariamente indisponíveis no teste.'}).waitFor();
  await figureEquals(performanceTotal(),'+R$ 131,00');assert.equal(await graph.count(),1);
  check('performance-refresh-error-retains-complete-cached-result');
  await page.evaluate(()=>{globalThis.__uiTest.failPerformance=false;globalThis.__uiTest.performance={...globalThis.__uiTest.performance,status:'incomplete',cdi:null,difference:null,issues:['CDI do período indisponível no cenário sintético.']};});
  await refreshPerformance().click();await performanceSection.getByText('O resultado dos FIIs está disponível. Falta o CDI do mesmo período para comparar.',{exact:true}).waitFor();
  await figureEquals(performanceTotal(),'+R$ 131,00');assert.equal(await graph.count(),0);
  await figureEquals(readout(performanceSection,'Resultado da carteira'),'+R$ 165,56');
  check('missing-CDI-preserves-full-FII-result-without-income-only-comparison');
  await page.evaluate(base=>{globalThis.__uiTest.performance={...base,status:'incomplete',marketValue:null,priceResult:null,totalResult:null,totalReturnPercent:null,difference:null,issues:['Sem cotação de MOCK11 no cenário sintético.']};},performance);
  await refreshPerformance().click();await performanceSection.getByText('Sem cotação de MOCK11 no cenário sintético.',{exact:true}).waitFor();
  await figureEquals(performanceTotal(),'—');await textEquals(breakdown.getByRole('row',{name:/^= Variação das cotas/}).locator('td'),'—');assert.equal(await graph.count(),0);
  await figureEquals(readout(performanceSection,'Total aplicado'),'R$ 2.240,00');await figureEquals(readout(performanceSection,'Rendimentos acumulados'),'+R$ 45,56');await figureEquals(readout(performanceSection,'Resultado da carteira'),'—');
  check('missing-quote-is-a-gap-not-zero-or-distributions-only-result');

  await page.evaluate(base=>{globalThis.__uiTest.performance=structuredClone(base);},performance);
  await refreshPerformance().click();await figureEquals(performanceTotal(),'−R$ 889,10');
  await page.setViewportSize({width:375,height:812});await noOverflow(page,'performance-mobile375-no-overflow');
  assert.deepEqual(await breakdown.locator('td.num').evaluateAll(cells=>cells.filter(cell=>cell.scrollWidth>cell.clientWidth+1).map(cell=>cell.textContent)),[]);
  assert.deepEqual(await graph.locator('text').evaluateAll(texts=>texts.filter(text=>{const svg=text.closest('svg').getBoundingClientRect(),box=text.getBoundingClientRect();return box.left<svg.left-1||box.right>svg.right+1;}).map(text=>text.textContent)),[],'Bar values must fit the mobile SVG without clipping');
  await shot(page,'mobile-fii-performance375');

  const refreshCount=(await calls(page,'refresh_fii_performance')).length;
  await page.evaluate(()=>{globalThis.__uiTest.deferPerformance=true;});
  await refreshPerformance().evaluate(button=>{button.click();button.click();});
  await page.waitForFunction(()=>globalThis.__uiTest.performanceWaiters.length===1);
  assert.equal((await calls(page,'refresh_fii_performance')).length,refreshCount+1);
  assert.equal(await performanceSection.getByRole('button',{name:'Atualizando…',exact:true}).isDisabled(),true);
  await page.getByRole('combobox',{name:'Carteira',exact:true}).selectOption('wallet-other');
  await figureEquals(readout(region(page,'Resultado da carteira'),'Total aplicado'),'R$ 480,00');
  await figureEquals(readout(region(page,'Resultado da carteira'),'Rendimentos acumulados'),'+R$ 20,00');
  await figureEquals(readout(region(page,'Resultado da carteira'),'Resultado da carteira'),'+R$ 40,00');
  await page.evaluate(()=>globalThis.__uiTest.performanceWaiters.splice(0).forEach(resolve=>resolve()));
  await page.waitForTimeout(150);await figureEquals(readout(region(page,'Resultado da carteira'),'Resultado da carteira'),'+R$ 40,00');
  assert.equal((await calls(page,'get_fii_performance')).at(-1).args.walletId,'wallet-other');
  check('performance-double-click-is-serialized-and-old-wallet-refresh-cannot-overwrite-new-wallet');
  await noUnexpectedCommands(page);await comparison.context.close();

  const loadingPerformance=await openPage('performance-loading',{width:1440,height:1000});page=loadingPerformance.page;
  const loadingResult=region(page,'Resultado da carteira');
  await loadingResult.getByText('Lendo dados salvos…',{exact:true}).waitFor();
  assert.equal(await loadingResult.getByRole('button',{name:'Atualizar resultado',exact:true}).isDisabled(),true);
  for(const label of ['Total aplicado','Rendimentos acumulados','Resultado da carteira'])await figureEquals(readout(loadingResult,label),'—');
  await page.evaluate(()=>{globalThis.__uiTest.deferPerformanceRead=false;globalThis.__uiTest.performanceReadWaiters.splice(0).forEach(resolve=>resolve());});
  await figureEquals(readout(loadingResult,'Resultado da carteira'),'−R$ 854,54');
  assert.equal((await calls(page,'refresh_fii_performance')).length,0);
  check('performance-loading-reads-cache-only-and-does-not-render-an-invented-zero');
  await noUnexpectedCommands(page);await loadingPerformance.context.close();

  // Separate synthetic success state for a clean preview, without the deliberate failure scenarios above.
  const connected=await openPage('connected',{width:1440,height:1000});page=connected.page;
  await navigate(page,'Mercado');
  const connectedQuotes=region(page,'Cotações dos fundos');
  await connectedQuotes.getByText('R$\u00a025,00',{exact:true}).waitFor();
  assert.equal(await connectedQuotes.getByText('Cadência do gratuito: 30 min',{exact:true}).count(),3);
  assert.equal(await connectedQuotes.getByText(/Atraso informado/).count(),0);
  const connectedCvm=region(page,'Informes mensais da CVM');
  await connectedCvm.getByText('Fundo CVM sintético B',{exact:true}).waitFor();
  const fiagroRow=connectedCvm.locator('.market-cvm-table tbody tr').filter({has:page.locator('.ticker',{hasText:/^VGIA11$/})});
  const fiagroDetails=connectedCvm.locator('.market-cvm-details-table tbody tr').filter({has:page.locator('.ticker',{hasText:/^VGIA11$/})});
  await fiagroRow.getByText('FIAGRO',{exact:true}).waitFor();
  await fiagroDetails.getByText('FIAGRO sintético VGIA11',{exact:true}).waitFor();
  await textEquals(fiagroDetails.locator('td').nth(0),'R$ 1.000.000,00');
  await textEquals(fiagroRow.locator('td').nth(1),'—');
  await textEquals(fiagroDetails.locator('td').nth(1),'—');
  await textEquals(fiagroDetails.locator('td').nth(2),'—');
  assert.equal(await fiagroRow.locator(`time[datetime="${fiagroReport.referenceDate}"]`).count(),1);
  assert.equal(await fiagroDetails.locator('a').getAttribute('href'),fiagroReport.sourceUrl);
  await connectedCvm.locator('.market-cvm-table tbody tr').first().getByText('FII',{exact:true}).waitFor();
  assert.equal(await fiagroRow.getByText('FII',{exact:true}).count(),0);
  check('CVM-FIAGRO-is-explicit-and-missing-metrics-are-not-invented-with-legacy-FII-fallback');
  const recentFetchRow=connectedQuotes.locator('.market-quotes-table tbody tr').filter({has:page.getByRole('rowheader',{name:'VGIA11',exact:true})});
  const recentFetchDetails=connectedQuotes.locator('.market-quotes-details-table tbody tr').filter({has:page.getByRole('rowheader',{name:'VGIA11',exact:true})});
  assert.equal(await recentFetchRow.locator('td').nth(1).locator('time').getAttribute('datetime'),quoteObservation.quotedAt);
  assert.equal(await recentFetchDetails.locator('time').getAttribute('datetime'),recentQuoteObservation.fetchedAt);
  assert.match(await recentFetchRow.locator('td').nth(1).innerText(),/2020/);
  assert.doesNotMatch(await recentFetchRow.innerText(),/tempo real|cotação (?:atual|de hoje)/i);
  check('quotes-separate-old-trading-date-from-new-fetch-even-after-success');
  assert.equal(await page.getByRole('alert').count(),0);
  assert.equal(await page.locator('.market-cell-error').count(),0);
  assert.equal((await calls(page,'refresh_public_data_if_due')).length,1);
  await noIndividualAutomaticRefresh(page);
  await noOverflow(page,'desktop-market-connected-no-overflow');await shot(page,'desktop-market-connected');
  await noUnexpectedCommands(page);check('market-clean-preview-shares-one-automatic-attempt-keeps-synthetic-cache-and-labels-plan-cadence');
  await connected.context.close();

  // A damaged local cache must not look like a missing key or disable an explicit repair.
  const corruptQuotes=await openPage('corrupt-quotes',{width:1440,height:1000});page=corruptQuotes.page;
  await navigate(page,'Mercado');
  const corruptSection=region(page,'Cotações dos fundos');
  await corruptSection.getByText('Cache de cotações sintético inválido. Atualize para consultar a fonte.',{exact:true}).waitFor();
  assert.equal(await corruptSection.getByRole('button',{name:'Atualizar cotações',exact:true}).isEnabled(),true);
  assert.equal(await corruptSection.getByText(/Cadastre uma chave gratuita/).count(),0);
  assert.equal((await calls(page,'refresh_quotes')).length,0);
  await corruptSection.getByRole('button',{name:'Atualizar cotações',exact:true}).click();
  await textMatches(corruptSection.locator('tbody tr').first(),/^TEST11 R\$ 9,25/);
  assert.equal((await calls(page,'refresh_quotes')).at(-1).args.walletId,'wallet-original');
  await noUnexpectedCommands(page);check('invalid-quote-cache-keeps-configured-key-and-explicit-refresh-available');
  await shot(page,'desktop-market-cache-recovery');await corruptQuotes.context.close();

  // ---------- Mobile: menu, every screen and the import modal at 375 px ----------
  const mobile=await openPage('mobile',{width:375,height:812});page=mobile.page;
  const menuButton=banner(page).locator('.mobile-menu');
  const drawerOnScreen=async()=>{const box=await sidebar(page).boundingBox();return box.x+box.width>1;};
  assert.equal(await menuButton.getAttribute('aria-label'),'Abrir menu');assert.equal(await menuButton.getAttribute('aria-expanded'),'false');assert.equal(await drawerOnScreen(),false);
  await menuButton.click();
  assert.equal(await menuButton.getAttribute('aria-label'),'Fechar menu');assert.equal(await menuButton.getAttribute('aria-expanded'),'true');assert.equal(await drawerOnScreen(),true);
  await shot(page,'mobile-drawer',{fullPage:false});
  await page.locator('.nav-backdrop').click({position:{x:355,y:400}});
  assert.equal(await menuButton.getAttribute('aria-expanded'),'false');assert.equal(await drawerOnScreen(),false);
  await navigate(page,'Mercado');assert.equal(await menuButton.getAttribute('aria-expanded'),'false');assert.equal(await drawerOnScreen(),false);
  check('mobile-menu-opens-closes-by-backdrop-toggle-and-navigation');
  await navigate(page,'Visão geral');
  await verifyOverview(page);await noOverflow(page,'mobile-overview-no-overflow');await shot(page,'mobile-overview');
  for(const [label,file] of [['FIIs e posições','mobile-portfolio'],['Caixinhas CDI','mobile-cash'],['Mercado','mobile-market'],['Projeções','mobile-projections'],['Assistente','mobile-assistant'],['Configurações','mobile-settings']]){
    await navigate(page,label);
    if(label==='Projeções'){await page.getByRole('button',{name:'Calcular projeção',exact:true}).click();await openExtra(page);await page.getByRole('table',{name:'Projeção mês a mês',exact:true}).waitFor();}
    await noOverflow(page,file+'-no-overflow');await shot(page,file);
    if(label==='FIIs e posições'){await openTab(page,'Histórico');await noOverflow(page,'mobile-portfolio-history-no-overflow');await shot(page,'mobile-portfolio-history');}
  }
  await navigate(page,'FIIs e posições');await openTab(page,'Posição');await panel(page,'Posição').getByRole('button',{name:'Atualizar posição',exact:true}).click();
  await importDialog(page).waitFor();await noOverflow(page,'mobile-import-modal-no-overflow');await shot(page,'mobile-import',{fullPage:false});
  await importDialog(page).getByRole('button',{name:'Cancelar',exact:true}).click();await importDialog(page).waitFor({state:'detached'});
  await noUnexpectedCommands(page);await mobile.context.close();

  // ---------- Other narrow layouts ----------
  for(const viewport of [{width:768,height:1024},{width:1024,height:768},{width:812,height:375}]){
    const device=await openPage('viewport-'+viewport.width,viewport);
    await noOverflow(device.page,'viewport-'+viewport.width+'-overview-no-overflow');await shot(device.page,'viewport-'+viewport.width+'-overview');
    await navigate(device.page,'FIIs e posições');await noOverflow(device.page,'viewport-'+viewport.width+'-portfolio-no-overflow');await shot(device.page,'viewport-'+viewport.width+'-portfolio');
    await openTab(device.page,'Histórico');await noOverflow(device.page,'viewport-'+viewport.width+'-portfolio-history-no-overflow');await shot(device.page,'viewport-'+viewport.width+'-portfolio-history');
    await device.context.close();
  }
  // Equivalent layout viewport of a 1440 px browser window at 200% zoom; no OS settings change.
  const zoom=await openPage('zoom-200-equivalent',{width:720,height:500});
  await noOverflow(zoom.page,'zoom-200-equivalent-overview-no-overflow');await shot(zoom.page,'zoom-200-equivalent-overview');
  await navigate(zoom.page,'FIIs e posições');await noOverflow(zoom.page,'zoom-200-equivalent-portfolio-no-overflow');await shot(zoom.page,'zoom-200-equivalent-portfolio');
  await openTab(zoom.page,'Histórico');await noOverflow(zoom.page,'zoom-200-equivalent-portfolio-history-no-overflow');await shot(zoom.page,'zoom-200-equivalent-portfolio-history');
  await zoom.context.close();

  // ---------- Wallet without an imported position: the purchase history stays available ----------
  const missing=await openPage('without-position',{width:1440,height:1000});page=missing.page;
  const emptyPosition=region(page,'Posição na corretora');
  await emptyPosition.getByRole('heading',{name:'Nenhuma posição importada',exact:true}).waitFor();
  assert.equal(await page.getByRole('table',{name:/^Composição do patrimônio/}).count(),0);
  assert.equal(await region(page,'Histórico de FIIs').locator('.group-head p').count(),0);
  await openExtra(page);
  await figureEquals(readout(region(page,'Resultado da carteira'),'Total aplicado'),'R$ 2.240,00');
  await shot(page,'without-position');
  await emptyPosition.getByRole('button',{name:'Importar posição',exact:true}).click();await importDialog(page).waitFor();await page.keyboard.press('Escape');await importDialog(page).waitFor({state:'detached'});
  // Without a position, FIIs e posições opens on the history tab.
  await navigate(page,'FIIs e posições');
  assert.equal(await tab(page,'Histórico').getAttribute('aria-selected'),'true');
  assert.equal(await ledger().locator('tbody tr').count(),2);
  assert.equal(await panel(page,'Histórico').locator('.group-head p').count(),0);
  await openTab(page,'Posição');
  await region(page,'Posição na corretora').getByRole('heading',{name:'Nenhuma posição importada',exact:true}).waitFor();
  assert.equal(await panel(page,'Posição').locator('.cp-listed').count(),0);
  check('missing-position-keeps-historical-ledger');await shot(page,'without-position-portfolio');
  await noUnexpectedCommands(page);await missing.context.close();

  // Ten original paragraphs become a grouped notice, without losing any source text or the affected values.
  const notifications=await openPage('position-notes',{width:1440,height:1000});page=notifications.page;
  const notedPosition=region(page,'Posição na corretora');
  await verifyImportNotice(page,notedPosition,importWarnings,{capture:'desktop-import-notification-grouped'});
  assert.equal(await notedPosition.getByRole('button',{name:/^Ver observações/}).innerText(),'Ver observações (6)');
  assert.deepEqual(await page.evaluate(()=>globalThis.__uiTest.snapshot.brokerPosition.warnings),importWarnings);
  await noOverflow(page,'grouped-import-notice-no-overflow');await shot(page,'desktop-position-notice-10');
  await page.setViewportSize({width:375,height:812});
  await notedPosition.getByRole('button',{name:/^Ver observações/}).click();
  const noticeDialog=page.getByRole('dialog',{name:'Observações da importação',exact:true});await noticeDialog.waitFor();
  await noOverflow(page,'mobile-grouped-import-notice-no-overflow');await shot(page,'mobile-import-notification',{fullPage:false});
  await noticeDialog.getByRole('button',{name:'Concluir',exact:true}).click();await noticeDialog.waitFor({state:'detached'});
  check('position-ten-import-notes-stay-verbatim-in-grouped-notification-with-targets');
  await noUnexpectedCommands(page);await notifications.context.close();

  // ---------- Purchases and income: separate visible lists with the same original records ----------
  const journal=await openPage('ledger',{width:1440,height:1000});page=journal.page;
  await navigate(page,'FIIs e posições');await openTab(page,'Histórico');
  const journalPanel=panel(page,'Histórico');
  const purchases=journalPanel.locator('.purchases-table');
  const receipts=journalPanel.locator('.income-table');
  const otherRecords=journalPanel.locator('.other-records-table');
  const monthlyRow=(table,month)=>table.locator('tbody tr').filter({has:page.locator(`time[datetime="${month}"]`)});
  assert.equal(await purchases.isVisible(),true);assert.equal(await receipts.isVisible(),true);
  assert.equal(await purchases.evaluate(element=>!!element.closest('details')),false);
  assert.equal(await receipts.evaluate(element=>!!element.closest('details')),false);
  assert.notEqual(await purchases.locator('.ledger-paid').first().evaluate(element=>getComputedStyle(element).color),await receipts.locator('.ledger-received').first().evaluate(element=>getComputedStyle(element).color),'Purchases and received income need distinct visual accents');
  assert.equal(await purchases.locator('tbody tr').count(),4);assert.equal(await receipts.locator('tbody tr').count(),4);
  assert.equal(await otherRecords.locator('tbody tr').count(),2);
  assert.equal(await journalPanel.getByRole('combobox',{name:'Mostrar lançamentos',exact:true}).count(),0);
  await textEquals(monthlyRow(purchases,'2020-01').locator('.ledger-paid'),'R$ 100,00');
  assert.equal(await monthlyRow(receipts,'2020-01').count(),0);
  assert.equal(await monthlyRow(purchases,'2020-02').count(),0);
  await textEquals(monthlyRow(receipts,'2020-02').locator('.ledger-received'),'R$ 5,00');
  await textEquals(monthlyRow(purchases,'2020-03').locator('.ledger-paid'),'R$ 25,00');
  await textEquals(monthlyRow(receipts,'2020-03').locator('.ledger-received'),'R$ 6,00');
  assert.equal(await monthlyRow(receipts,'2020-04').count(),0,'Explicit zero cannot become an income');
  await textEquals(monthlyRow(receipts,'2020-05').locator('.ledger-received'),'R$ 3,75');
  await textEquals(monthlyRow(receipts,'2020-06').locator('.ledger-received'),'R$ 1,50');
  await textEquals(monthlyRow(purchases,'2020-06').locator('.ledger-paid'),'R$ 30,00');
  assert.equal(await monthlyRow(purchases,'2020-07').count(),0,'A price without quantity is not a purchase');
  await textEquals(monthlyRow(purchases,'2020-08').locator('.ledger-paid'),'R$ 0,00');
  const monetaryValue=text=>Number(text.replace(/[^\d,-]/g,'').replace(',','.'));
  assert.equal(Math.round((await purchases.locator('.ledger-paid').allInnerTexts()).reduce((sum,value)=>sum+monetaryValue(value),0)*100),15500);
  assert.equal(Math.round((await receipts.locator('.ledger-received').allInnerTexts()).reduce((sum,value)=>sum+monetaryValue(value),0)*100),1625);
  check('ledger-separate-lists-preserve-purchases-income-same-month-records-and-explicit-zero');
  assert.match(await tip(journalPanel,'Valor recebido'),/fim do mês.*total 0.*não é recalculado/i);
  assert.match(await tip(journalPanel,'Parte do valor comprado'),/200.*1\.000.*20%/);
  const detailChecks=[
    ['ledger-purchase',purchases,'R$ 100,00','R$ 0,00'],
    ['ledger-income',receipts,'R$ 0,00','R$ 5,00'],
    ['ledger-both',purchases,'R$ 25,00','R$ 6,00'],
    ['ledger-explicit-zero',otherRecords,'R$ 0,00','R$ 0,00'],
    ['ledger-manual-income',receipts,'R$ 0,00','R$ 3,75'],
    ['ledger-same-month-income',receipts,'R$ 0,00','R$ 1,50'],
    ['ledger-same-month-purchase',purchases,'R$ 30,00','R$ 0,00'],
    ['ledger-price-only',otherRecords,'R$ 0,00','R$ 0,00'],
    ['ledger-zero-price',purchases,'R$ 0,00','R$ 0,00'],
  ];
  for(const [id,table,paid,received] of detailChecks){
    const original=ledgerEntries.find(entry=>entry.id===id);
    const trigger=monthlyRow(table,original.month).getByRole('button',{name:/^Ver detalhes /});
    await trigger.click();
    const recordDetail=page.getByRole('dialog',{name:'Detalhes do lançamento · TEST11',exact:true});await recordDetail.waitFor();
    await textEquals(fact(recordDetail,'Cotas compradas'),original.cotasAdded.replace('.',','));
    await textEquals(fact(recordDetail,'Total pago'),paid);await textEquals(fact(recordDetail,'Valor recebido'),received);
    assert.equal(await recordDetail.locator('.record-source').evaluate(element=>element.open),false);
    if(id==='ledger-same-month-income')await shot(page,'desktop-ledger-record-details',{fullPage:false});
    await recordDetail.locator('.record-source > summary').click();
    assert.equal(await recordDetail.locator('.record-source pre').textContent(),original.notes,'The source note must remain verbatim and attached to its own record');
    await page.keyboard.press('Escape');await recordDetail.waitFor({state:'detached'});await focusReturnsTo(page,trigger);
  }
  assert.equal(await journalPanel.locator('.entry-details-table').count(),0);
  assert.equal((await calls(page,'save_record')).length,0,'Reading details must not create or rewrite records');
  assert.equal(await page.evaluate(()=>globalThis.__uiTest.snapshot.funds[0].entries.length),ledgerEntries.length);
  check('record-detail-modal-preserves-each-source-note-same-month-record-zero-and-null-values');
  await noOverflow(page,'desktop-ledger-types-no-overflow');await shot(page,'desktop-ledger-types');
  assert.equal(await monthlyRow(purchases,'2020-03').count(),1);assert.equal(await monthlyRow(receipts,'2020-03').count(),1);
  check('ledger-combined-record-provides-one-purchase-and-one-receipt-without-creating-records');
  // Saving unchanged legacy records must preserve explicit zero versus automatic/null totals.
  await monthlyRow(purchases,'2020-03').getByRole('button',{name:/^Editar compra/}).click();
  const editRecord=page.getByRole('dialog',{name:'Editar lançamento · TEST11',exact:true});await editRecord.waitFor();
  assert.equal(await editRecord.getByLabel('Cotas adicionadas',{exact:true}).inputValue(),'2');
  assert.equal(await editRecord.getByLabel('Preço por cota (R$)',{exact:true}).inputValue(),'12.50');
  assert.equal(await editRecord.getByLabel('Total recebido (R$)',{exact:true}).inputValue(),'');
  await shot(page,'desktop-ledger-edit-mixed',{fullPage:false});
  await page.evaluate(()=>{globalThis.__uiTest.deferSaves=true;});
  await editRecord.getByRole('button',{name:'Salvar lançamento',exact:true}).click();
  await page.waitForFunction(()=>globalThis.__uiTest.saveWaiters.length===1);
  assert.equal(await editRecord.getAttribute('aria-busy'),'true');
  assert.equal(await editRecord.getByRole('button',{name:'Cancelar',exact:true}).isDisabled(),true);
  assert.equal(await editRecord.getByLabel('Total recebido (R$)',{exact:true}).isDisabled(),true);
  await page.keyboard.press('Escape');assert.equal(await editRecord.evaluate(element=>element.open),true);
  await page.evaluate(()=>{const test=globalThis.__uiTest;test.deferSaves=false;test.saveWaiters.splice(0).forEach(resolve=>resolve());});
  await editRecord.waitFor({state:'detached'});
  let recordSaves=await calls(page,'save_record');
  assert.equal(recordSaves.length,1);assert.equal(recordSaves[0].args.walletId,'wallet-original');
  assert.deepEqual(recordSaves[0].args.input,{...ledgerEntries.find(item=>item.id==='ledger-both'),fundId:'fund-a'});
  await dismissToast(page,'Lançamento salvo. Indicadores atualizados.');
  await openExtra(journalPanel);
  await monthlyRow(otherRecords,'2020-04').getByRole('button',{name:/^Editar registro/}).click();await editRecord.waitFor();
  assert.equal(await editRecord.getByLabel('Total recebido (R$)',{exact:true}).inputValue(),'0');
  await editRecord.getByRole('button',{name:'Salvar lançamento',exact:true}).click();await editRecord.waitFor({state:'detached'});
  recordSaves=await calls(page,'save_record');assert.equal(recordSaves.at(-1).args.input.dividendTotal,'0');
  await openExtra(journalPanel);
  assert.equal(await monthlyRow(otherRecords,'2020-04').count(),1);assert.equal(await purchases.locator('tbody tr').count(),4);assert.equal(await receipts.locator('tbody tr').count(),4);
  await dismissToast(page,'Lançamento salvo. Indicadores atualizados.');
  check('ledger-edit-keeps-id-quantities-prices-notes-null-zero-and-blocks-close-during-save');
  await banner(page).getByRole('button',{name:'Novo lançamento',exact:true}).click();
  const createRecord=page.getByRole('dialog',{name:'Novo lançamento · TEST11',exact:true});await createRecord.waitFor();
  await createRecord.getByLabel('Mês de referência',{exact:true}).fill('2020-09');
  await createRecord.getByLabel('Total recebido (R$)',{exact:true}).fill('2,50');
  await createRecord.getByRole('button',{name:'Salvar lançamento',exact:true}).click();await createRecord.waitFor({state:'detached'});
  await textEquals(monthlyRow(receipts,'2020-09').locator('.ledger-received'),'R$ 2,50');
  assert.equal(await monthlyRow(purchases,'2020-09').count(),0);
  recordSaves=await calls(page,'save_record');assert.equal(recordSaves.at(-1).args.input.cotasAdded,'0');assert.equal(recordSaves.at(-1).args.input.dividendTotal,'2.50');
  check('ledger-new-income-record-does-not-invent-a-purchase-from-prefilled-price');
  await dismissToast(page,'Lançamento salvo. Indicadores atualizados.');
  await page.setViewportSize({width:1000,height:760});await noOverflow(page,'ledger-1000-no-overflow');
  await page.setViewportSize({width:375,height:812});await noOverflow(page,'ledger-375-no-overflow');await shot(page,'mobile-ledger-types');
  await noUnexpectedCommands(page);await journal.context.close();

  // ---------- Web preview: no IPC, no local data, desktop-only actions disabled ----------
  const web=await openPage('web',{width:375,height:812});page=web.page;
  await page.locator('.preview-banner').getByText('Prévia da interface',{exact:true}).waitFor();
  await textMatches(sidebar(page).locator('.sidebar-status'),/^Prévia web, sem dados/);
  assert.equal(await page.evaluate(()=>'__TAURI_INTERNALS__' in window),false);
  assert.equal(await page.getByRole('combobox',{name:'Carteira',exact:true}).isDisabled(),true);
  const importButtons=page.getByRole('button',{name:'Importar posição',exact:true});
  assert.equal(await importButtons.count(),2);for(const button of await importButtons.all())assert.equal(await button.isDisabled(),true);
  assert.equal(await page.getByRole('button',{name:'Importar histórico',exact:true}).isDisabled(),true);
  await page.getByRole('heading',{name:'Nenhum fundo no histórico',exact:true}).waitFor();
  assert.doesNotMatch(await page.locator('#main-content').innerText(),/R\$/);
  await noOverflow(page,'web-preview-no-overflow');await shot(page,'web-preview');
  for(const label of pages.slice(1)){await navigate(page,label);assert.doesNotMatch(await page.locator('#main-content').innerText(),/R\$\s?[1-9]/);await noOverflow(page,'web-preview-'+label.normalize('NFD').replace(/\p{M}/gu,'').replace(/\W+/g,'-').toLowerCase()+'-no-overflow');}
  assert.deepEqual(web.externalRequests,[],'Web preview must not consult external public APIs');
  check('web-preview-has-no-local-financial-data-or-external-HTTP');await web.context.close();

  await verifyTooltips({browser,base,check,shot,browserErrors});

  // ---------- Vite never serves the local portfolio or verification data ----------
  const privacy=[];
  // Request JSON explicitly: a missing file otherwise returns Vite's SPA HTML
  // with status 200 on clean checkouts. Also verify existing synthetic files in
  // an isolated root, without relying on or reading the user's local portfolio.
  for(const requestPath of ['/data/fiis_data.json','/.local/last-verification.json']){const response=await fetch(base+requestPath,{headers:{Accept:'application/json'}});assert.ok([403,404].includes(response.status),`${requestPath} returned ${response.status}`);privacy.push({path:requestPath,status:response.status});await response.body?.cancel();}
  privacy.push(...await verifyVitePrivacy());
  check('vite-protected-paths-denied');
  assert.deepEqual(browserErrors,[],'Browser runtime errors occurred');
  const report={status:'PASS',uiVersion,syntheticDataOnly:true,checks,privacy,browserErrors,screenshots,screenshotsDirectory:'.local/ui-verification',zoomMethod:'200% equivalent: 720 CSS px at DPR 2, layout viewport of a 1440 px window; no OS settings changed.'};
  await fs.writeFile(path.join(output,'report.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify({status:report.status,checks:checks.length,screenshots:screenshots.length,browserErrors:browserErrors.length}));
}catch(error){await fs.writeFile(path.join(output,reportName),JSON.stringify({status:'FAIL',uiVersion,message:error.message,checks,browserErrors,screenshots},null,2));throw error;}
finally{await browser?.close();await server?.close();}
}
await run();

# Núcleo da carteira

Biblioteca Rust independente de Tauri, reutilizável por desktop, MCP e API.
`Repository::open(path)` cria o SQLite separado do JSON legado. Os DTOs usam
`camelCase` e números decimais como texto; o cálculo usa `rust_decimal`, sem `f64`.

## Migração e preservação

- `import_legacy_file(path)` lê a origem sem modificá-la, calcula SHA-256 e cria
  `backups/legacy-<sha256>.json` ao lado do banco, com os mesmos bytes da origem.
- Validação, importação, recibo e fila local são atômicos no SQLite. Erros de
  formato, precisão ou cálculo não criam uma carteira parcial.
- O SHA-256 torna a repetição da mesma importação idempotente. Um arquivo
  diferente exige carteira vazia; não há mesclagem ou substituição implícita.
- IDs determinísticos preservam lançamentos distintos, inclusive quando têm o
  mesmo mês e os mesmos valores. A edição usa o ID do lançamento.
- `dividend_total` explícito é preservado, incluindo zero e casas decimais
  adicionais. `null` continua identificando um valor calculado automaticamente.
- O esquema SQLite 7 acrescenta somente `monthly_records.purchase_date`,
  anulável. Os registros existentes continuam sem uma data de compra inferida,
  e a migração não reescreve valores, revisões ou a fila de sincronização.
- `RecordInput.purchaseDate` e `MonthlyEntry.purchaseDate` são opcionais. A
  data informada precisa ser válida em `AAAA-MM-DD`, pertencer ao mês do
  lançamento e acompanhar uma quantidade positiva de cotas compradas. Ela
  permite conferir a elegibilidade em eventos com data de corte. Não altera
  automaticamente totais explícitos ou os cálculos mensais do histórico antigo.
- O backup legado usa `purchase_date` somente quando há uma data informada.
  Backups e entradas antigos continuam compatíveis, sem acrescentar campos
  ausentes ou inventar dias de compra.
- `export_legacy_json()` retorna o esquema legado com números JSON decimais,
  sem conversão por ponto flutuante. O backup preserva também formatação e
  quaisquer campos extras que o esquema do aplicativo ainda não interpreta.

## Regras implementadas

### Posição observada na corretora

`Repository::import_broker_position_file(path)` importa a transcrição de posição
em português (`versao_formato: 1`, moeda BRL), uma posição normalizada ou um
backup completo contendo `broker_position`. A API retorna `BrokerPosition` com
chaves camelCase, valores decimais em strings e proveniência (hash SHA-256,
arquivo original e instante da importação). Nenhum dado pessoal está embutido
no executável ou nas fixtures de teste.

O snapshot sempre inclui `brokerPosition`, inicialmente `null`. A posição é
persistida separadamente no SQLite, sem transformar preços médios ou saldos
em compras, dividendos ou alterações do histórico. Panorama com caixa e
detalhamento de fundos listados mantêm seus próprios totais e horários;
diferenças entre essas telas não são somadas. `return12Months` usa pontos
percentuais, e `reportedEarnings` do pós-fixado não integra dividendos recebidos.

A importação valida datas, horários, números, somas e identidades entre custo,
posição e resultado. O preço médio exibido admite arredondamento de centavos,
preservando o custo total informado. Antes de gravar a posição, salva uma cópia
byte a byte em `backups/broker-position-<sha256>.json`. Um backup existente é
conferido e nunca sobrescrito. Repetir um hash já importado preserva a posição
mais recentemente salva, mesmo que esse hash corresponda a um arquivo anterior.

O esquema SQLite migra da versão 1 para 2 sem alterar lançamentos. A exportação
legada acrescenta `broker_position` somente quando existe uma posição; sua
importação restaura posição e histórico na mesma transação. Backups completos
preservam a proveniência original e continuam compatíveis com arquivos antigos
que contêm apenas `fiis`.

Um rendimento automático usa todas as cotas registradas até o mês do
lançamento, inclusive o próprio mês, e nunca compras em meses posteriores.
Compras retroativas atualizam apenas valores derivados; totais explícitos não
mudam. Os registros mensais antigos não informam a data de corte do provento:
esta convenção mensal é uma aproximação explícita até a introdução do modelo
de operações e eventos por data.

A estimativa mensal usa a posição atual e a média dos últimos seis registros
positivos de rendimento por cota. As projeções têm janela configurável, compra
mensal em **cotas** e reinvestimento opcional. O preço usado é o custo médio
histórico; não é uma cotação de mercado. Compras planejadas precedem o rendimento;
reinvestimentos em cotas inteiras acontecem depois, com saldo restante para o
mês seguinte. Valores derivados de dinheiro arredondam para duas casas com
empate para longe de zero. Valores explícitos importados nunca são arredondados.

`pending_change_count()` e `pending_changes()` expõem a fila durável de
alterações. Esta biblioteca não simula uma conexão Vercel e não remove pendências
sem a confirmação de um futuro servidor autenticado.

## APIs

```text
Repository::open(path) -> Result<Repository>
import_legacy_file(path) -> Result<ImportReceipt>
snapshot() -> Result<PortfolioSnapshot>
export_legacy_json() -> Result<String>
upsert_fund(FundInput) -> Result<()>
save_monthly_record(RecordInput) -> Result<()>
project(ProjectionInput) -> Result<Vec<ProjectionPoint>>
pending_change_count() -> Result<usize>
pending_changes() -> Result<Vec<PendingChange>>
```

Todos os métodos de instância recebem `&self`; a conexão tem proteção interna
por mutex e SQLite WAL. Consultas da carteira usam uma transação de leitura para
manter os fundos e lançamentos consistentes mesmo com um processo MCP gravando
no mesmo banco. Criar um ticker já cadastrado é rejeitado; alterações de fundos
existentes exigem o ID explícito para proteger os metadados.

Os testes usam apenas dados sintéticos e diretórios
temporários: `cargo test -p ivestfii-core`.

## Proventos publicados por data

`income::calculate_monthly_income(snapshot, events, start_month, end_month,
today)` produz uma série mensal derivada, sem gravar recibos ou alterar
`monthly_records`, os totais contábeis e a fila de sincronização. O intervalo
inclui seus dois meses, limita-se a 1200 meses e não admite meses futuros.

Um total informado pelo usuário, inclusive zero, ou um rendimento por cota
positivo com cotas no histórico tem prioridade sobre a API no mesmo fundo/mês.
Sem esse registro, cada `IncomeEvent` multiplica o valor publicado por cota
pelas compras até sua **data-com**, inclusive, e entra no mês do pagamento.
As cotas continuam nos meses seguintes sem exigir novas compras. Compras do
mesmo mês da data-com sem dia informado tornam o evento `date_required`;
compras de meses anteriores permanecem elegíveis. Eventos incompletos, futuros
ou com o mesmo ID e dados financeiros conflitantes não completam lacunas.

O relatório inclui cotas no mês, cotas elegíveis por evento, fonte, URL,
data-com, data de pagamento e motivos de ausência. Valores de API são
calculados a partir de informações públicas, sem confirmar um crédito na
corretora. `amount: null` distingue informação ausente de zero. Um mês
incompleto mantém `total: null`, expondo somente `knownTotal`, `recordedAmount`
e `apiAmount` como subtotais conhecidos, além das contagens por origem.

`performance::calculate_fii_performance_with_income(snapshot, quotes, rates,
events, today)` usa o mesmo cálculo para complementar os rendimentos acumulados,
o resultado dos FIIs e a diferença para o CDI. Soma somente `apiAmount` ao
histórico; `knownTotal` já contém os lançamentos manuais e não deve ser somado
novamente. Os proventos complementares entram também nos totais da carteira com
as caixinhas conciliadas. Não alteram o extrato, os saldos ou a sincronização.

O complemento usa a data comum das cotações como limite de pagamento. Um evento
da API pago depois dela não entra em um resultado de mercado anterior. Sem
cotações comuns, os rendimentos conhecidos podem ser apurados até `today`, mas
o resultado de mercado e sua comparação continuam indisponíveis. Compras com
dia informado usam essa data no CDI, com juros apenas depois do dia da compra;
compras antigas sem dia mantêm a aproximação pelo fim do mês. Compras posteriores
à cotação são excluídas, e impedem apresentar a parcela antiga como resultado
completo da posição atual.

`FiiPerformance.income` identifica `recordedAmount`, `apiAmount`, o período,
a data de corte e os totais por fundo em `byFund`. Cobertura `partial` indica
meses sem dados suficientes; `missingFundMonthCount` inclui os casos de
`dateRequiredFundMonthCount`. Esses subtotais conhecidos não comprovam que todos
os pagamentos históricos tenham sido encontrados. Falta de cobertura de
proventos é independente da conciliação de custo e saldo das caixinhas.
`calculate_fii_performance` permanece disponível para cálculos sem complemento.

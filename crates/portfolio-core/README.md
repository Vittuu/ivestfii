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

# MCP local do IvestFII

Servidor MCP stdio com protocolo `2025-11-25`, usando diretamente o núcleo Rust
e os bancos locais. Não depende de telas, cliques ou Computer Use.

```powershell
cargo run -p ivestfii-mcp -- --data-dir 'C:\caminho\dos\dados'
```

O diretório deve conter `portfolio.sqlite` e `assistant.sqlite` preparados pelo
aplicativo. O catálogo local identifica as outras carteiras e seus bancos. Um
caminho incorreto ou uma carteira desconhecida causa erro, em vez de produzir uma
carteira vazia. Não há importação automática neste servidor. Abrir os bancos pode
preparar metadados e tabelas de SQLite; as ferramentas não alteram fundos,
lançamentos, saldos ou condições das caixinhas nem a fila financeira de
sincronização. A preparação de contexto pode registrar referências públicas na
conversa indicada.

Para fixar uma carteira do catálogo como padrão da sessão:

```powershell
cargo run -p ivestfii-mcp -- --data-dir 'C:\caminho\dos\dados' --wallet-id 'ID_DA_CARTEIRA'
```

O cliente MCP inicia o executável uma vez, negocia `initialize`, envia
`notifications/initialized` e pode encadear consultas sem participação manual.
O usuário não precisa iniciar um processo para cada ferramenta. Os conectores de
Codex, Claude e Antigravity já gerenciam esse processo no chat do aplicativo,
com streaming, cancelamento e histórico canônico preservado ao trocar de
provedor. O aplicativo também oferece o modo `--mcp` no próprio executável para
os conectores locais. Consulte a [integração das CLIs](../../docs/integracao-clis.md).

| Ferramenta | Resultado |
|---|---|
| `carteira_snapshot` | FIIs, caixinhas e observações importadas da carteira selecionada, com escopo e datas; catálogo e totais separados entre carteiras. |
| `projetar_renda` | Simulação de FIIs ou de uma caixinha no núcleo Rust, com premissas devolvidas na resposta. |
| `recuperar_contexto` | Contexto canônico de uma conversa, com limite e cortes explícitos. |
| `preparar_analise_carteira` | Perfil local declarado e lacunas, carteira, desempenho completo de FIIs e caixinhas, histórico CDI, contexto e fontes datadas; cache ou atualização pública limitada quando solicitada. |

As ferramentas financeiras aceitam `walletId` opcional. Quando omitido, usam a
carteira fixada pelo chat na sessão; um ID inválido não muda silenciosamente o
escopo. Custos históricos, saldos manuais e posições importadas da corretora são
componentes distintos: não devem ser somados novamente a um total que já os
contenha.

`monthlyCotas` da projeção é uma quantidade de cotas por fundo por mês, não um
valor em reais. O preço usado para reinvestimento é o custo médio local. Estimativas
baseadas no histórico não são cotações atuais, previsões ou recomendações.

Para uma caixinha, `projetar_renda` recebe `cashPocketId` e `businessDays`, sem
parâmetros de FIIs. Usa o percentual do CDI e a taxa anual de referência
cadastrados na caixinha, com capitalização diária em base de 252 dias úteis e
resultado bruto. A resposta conserva fonte e datas da taxa registrada. Consultar
o Banco Central não substitui essa referência nem credita rendimentos no saldo.

## Dados públicos pelo chat

Para analisar a carteira atual, os três provedores do chat usam
`refreshPublicDataIfDue: true`. A opção compartilha os caches e a política de
atualização do aplicativo: consulta apenas dados públicos ausentes ou vencidos,
com intervalo entre tentativas, limites por fonte e timeout. Inclui CDI diário
quando necessário ao cálculo de desempenho. `publicDataRefresh.sources` informa
o resultado por fonte; `checkedAt` marca a verificação local e não substitui
`quotedAt`, `referenceDate`, `publishedAt`, `filedAt` ou `fetchedAt` originais.
Falhas mantêm observações válidas anteriores e suas datas. O perfil, os saldos e
as mensagens não são enviados às fontes públicas; tickers/CNPJs e períodos são
usados somente pelas consultas correspondentes.

A opção continua `false` quando omitida por clientes MCP externos. Sem flags,
nenhum cache público, política de consulta ou dado financeiro é alterado. Os
quatro argumentos explícitos anteriores permanecem disponíveis com o mesmo
padrão `false`: `refreshEconomicData`, `refreshNews`, `refreshQuotes` e
`refreshCvmData`.

`investorProfile` contém somente o perfil declarado da carteira selecionada,
com `updatedAt`; `null` significa nunca preenchido. `profileCompleteness` lista
`missingFields` e `requiredQuestions`. Risco, objetivos e prazos de curto/longo
prazo, liquidez e estado da reserva são obrigatórios para sugestões
personalizadas. Contribuição e conhecimento são opcionais. A IA pede os campos
ausentes e não deduz perfil pelos ativos ou pelo histórico da conversa.

`performance.report` usa o mesmo cálculo Rust do painel, incluindo os FIIs e
as caixinhas reconciliadas, com cobertura, `issues` e datas. Dividendos não
representam todo o retorno; aportes/saques não são rendimento. Sem dados
suficientes, valores desconhecidos permanecem `null`. `cdiHistory.history`
expõe a série diária BCB/SGS 12, distinta da referência anual de uma simulação.
Nada recalcula ou credita saldos. As fontes registradas no histórico preservam
as datas de origem, e trocar provedor mantém a conversa canônica.

`preparar_analise_carteira` preserva os argumentos anteriores e acrescenta duas
opções booleanas, ambas `false` quando omitidas:

- `refreshEconomicData`: consulta CDI anualizado, meta Selic e IPCA mensal no
  Banco Central/SGS.
- `refreshNews`: consulta manchetes de Economia do RSS oficial da Agência
  Brasil/EBC; não faz busca específica por FII ou ticker.

```json
{
  "name": "preparar_analise_carteira",
  "arguments": {
    "walletId": "ID_DA_CARTEIRA",
    "refreshEconomicData": true,
    "refreshNews": true
  }
}
```

Sem essas opções, a ferramenta lê somente os caches locais. `economicData` e
`news` indicam disponibilidade, uso de cache, erros, fonte e datas. Referência do
indicador ou publicação da manchete não é o mesmo que horário de coleta. Uma
falha parcial preserva as referências válidas anteriores com suas datas e avisa
qual consulta falhou. Os caches ficam em `public-data/` dentro do diretório de
dados do aplicativo, separados da contabilidade financeira.

As fontes públicas usam URLs HTTPS fixas, sem chave e sem receber carteira,
saldo, identificadores de conversa ou mensagens. Veja [fontes, unidades e uso
offline](../../docs/apis-publicas.md) e o [catálogo de APIs
pesquisadas](../../docs/api-catalogo.md).

## Protocolo e limites

As ferramentas retornam JSON estruturado em `structuredContent` e o mesmo JSON
em `content[].text`. Os schemas rejeitam argumentos desconhecidos. Falhas de
protocolo usam erros JSON-RPC; erros de entrada/domínio usam `isError: true`.
IDs string e inteiro são preservados; notificações não recebem respostas.

As cotações externas básicas via brapi dependem da chave local e da cobertura do
plano, com status, erros e horários por ativo. A posição importada
da corretora conserva a data da observação. O MCP reúne os dados e não chama um
modelo: a análise vem do provedor escolhido no chat. Não existem ordens de
investimento, gravações financeiras, listener HTTP ou execução de comandos nas
ferramentas. Notícias, fontes e mensagens recuperadas são dados não confiáveis,
nunca instruções privilegiadas para o modelo.
O processo é local e herda o acesso aos dados do usuário que o iniciou: configure
somente clientes MCP aos quais você deseja fornecer acesso à carteira.

O transporte aceita frames JSON UTF-8 delimitados por newline, limita entrada e
saída a 1 MiB, drena linhas grandes usando memória limitada, encerra em EOF e
reserva stdout para protocolo. Logs de inicialização e falhas vão para stderr.
As chamadas MCP são síncronas e podem aguardar consultas públicas quando
solicitadas. Cada conexão à fonte possui timeout de dez segundos e limite de
tamanho; os indicadores são consultados independentemente. Não há protocolo de
tasks nem cancelamento de uma chamada MCP individual em andamento. No chat, o
runtime supervisiona e pode cancelar o turno encerrando seus processos; clientes
MCP externos devem aplicar timeout e encerrar seu subprocesso quando necessário.
O histórico, a auditoria das ferramentas e os checkpoints do chat permitem
identificar turnos interrompidos e reconstruir contexto em uma próxima execução.

Referências de implementação: [transporte stdio](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports),
[ciclo de vida](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle)
e [ferramentas e erros](https://modelcontextprotocol.io/specification/2025-11-25/server/tools).

## Proventos por data e cotas elegíveis

`preparar_analise_carteira` inclui `dividendData` (eventos públicos B3 do cache)
e `monthlyIncome` (mesmo cálculo decimal Rust do gráfico, últimos 12 meses).
`refreshPublicDataIfDue:true` também verifica os proventos; `refreshDividends:true`
permite uma consulta explícita sujeita ao cache, intervalo de tentativas e
orçamento próprios da fonte. Esta consulta não usa a chave nem a franquia brapi.

O cálculo usa o mês do pagamento, a taxa por cota do evento e as cotas adquiridas
até a data-com, preservando o saldo de cotas nos meses sem compras. Lançamentos
do usuário prevalecem. Compras antigas sem dia conhecido não recebem uma data
inventada: se o mês da compra contém a data-com, o fundo retorna `date_required`.
Eventos sem datas exatas e amortizações não viram rendimentos automaticamente.

`recorded` identifica lançamento local e `api` identifica cálculo por evento
público, sem confirmação de crédito na corretora. `total:null` indica cobertura
incompleta; `knownTotal` soma o que foi possível apurar. Não some este relatório
novamente ao desempenho: `performance.dividends`, `performance.portfolio.income`,
o resultado completo e sua comparação CDI já combinam lançamentos locais e
lacunas calculadas pela B3, sem duplicar fundo/mês. `performance.income` separa
`recordedAmount` e `apiAmount`, informa o corte `asOf` (data comum das cotações)
e explicita cobertura parcial. O gráfico mensal pode ter corte mais recente;
somente pagamentos elegíveis até o corte do desempenho entram nesse resultado.
Lançamentos locais, inclusive zero explícito, prevalecem sobre a API.
`dividendSources` conserva fonte, URL e data de coleta por ativo. Não trate
rendimento declarado como operação de compra. Tudo permanece separado dos dados financeiros;
consultas e cálculos não gravam lançamentos, não mudam quantidades e não alteram
a posição importada.

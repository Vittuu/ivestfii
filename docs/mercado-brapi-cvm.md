# Cotações brapi e informes CVM

Integração disponível no aplicativo Windows e nas quatro ferramentas MCP já usadas pelos conectores das CLIs. A versão 0.1.1 acrescenta informes FIAGRO da base própria da CVM, incluindo VGIA11. Não altera a posição importada, quantidades, compras, dividendos recebidos, saldos das caixinhas ou totais patrimoniais.

## Cotações

Em **Configurações → Dados de mercado**, cadastre a chave da brapi. O campo fica vazio ao reabrir a tela: a chave é guardada no Gerenciador de Credenciais do Windows, separada por diretório de dados. Ela não entra no SQLite, JSON de configurações, exportações de carteira, histórico do chat, argumentos da CLI ou sincronização. O MCP acessa o mesmo cofre na conta Windows atual. `BRAPI_API_KEY` ou `BRAPI_TOKEN` são alternativas por ambiente quando não há chave local; remover a chave local não remove uma variável de ambiente.

Ao abrir o aplicativo, as cotações dos símbolos da carteira selecionada são atualizadas em segundo plano quando necessário, incluindo os fundos da posição importada. A consulta automática reutiliza preços coletados há menos de uma hora. Em **Mercado**, **Atualizar cotações** também respeita o cache mínimo e os controles de consumo. Cada ativo usa uma requisição em `https://brapi.dev/api/v2/stocks/quote?symbols=TICKER`, com autenticação no header. Somente os símbolos são enviados à brapi, sem quantidades, valores da carteira ou mensagens.

Cada observação mantém preço decimal, moeda, fonte, URL sem credencial, horário de mercado `quotedAt` e coleta `fetchedAt`. O horário da coleta não substitui um horário de mercado ausente. As cotações alimentam o resultado completo dos FIIs e sua comparação com o CDI na Visão geral. Os valores originais da posição importada conservam sua referência e não são reescritos.

O [plano gratuito publicado](https://brapi.dev/faq/api-e-gratis-mesmo), conferido em 20/10/2026, informa 15 mil requisições por ciclo, um ativo por chamada e atraso aproximado de 30 minutos. A [janela de consumo](https://web-next.brapi.dev/faq/quais-as-limitacoes) reinicia na data de renovação indicada no painel, não necessariamente no primeiro dia do mês. O aplicativo consulta sequencialmente e limita seu próprio consumo a 80 chamadas por dia em Brasília, compartilhadas entre abertura automática, botões e MCP. Essa margem não contabiliza outros programas usando a mesma chave. A integração usa somente a cotação básica, sem módulos pagos.

As consultas autenticadas de MXRF11, VGHF11, VGIA11 e XPML11 retornaram preço em BRL e horário de mercado. Sem chave, esses mesmos ativos retornaram HTTP 401. A validação comprova a cobertura básica da chave utilizada, não o plano contratado nem a gratuidade de outros módulos.

Falhas de autenticação, recurso não incluído, limite e rede são distintas. HTTP 429 interrompe o restante do lote e conserva `Retry-After` no cache. Chamadas são sequenciais, inclusive entre aplicativo e MCP, e não fazem repetição automática. Uma falha mantém a última observação com a data original e um erro visível.

Uma leitura local não repara arquivos inválidos. Em uma consulta permitida pela política de atualização, um cache inválido de até 2 MiB pode ser reconstruído somente se pelo menos uma cotação nova for validada. Antes da substituição é preservada uma cópia exata `quotes.corrupt-UUID.json` na pasta pública local; valores antigos inválidos nunca são reaproveitados. Se toda a consulta falhar, o arquivo original permanece intacto. Um controle separado `quotes.recovery-backoff.json` conserva um eventual novo `Retry-After` mesmo nessa falha, compartilhado com o MCP. Arquivos maiores ou falhas de leitura continuam com diagnóstico, sem cópia ilimitada nem reconstrução automática.

## Proventos históricos gratuitos da B3

O gráfico mensal preenche lacunas com a consulta pública usada pela [página oficial de fundos da B3](https://sistemaswebb3-listados.b3.com.br/fundsListedPage/events-main/MXRF), sem chave. O endpoint `fundsListedProxy/Search/GetEventsCorporateActions` recebe somente o código público do fundo. Quantidades, datas de compra e dados da carteira ficam no computador. A integração foi validada para MXRF11, VGHF11, VGIA11 e XPML11. O serviço pertence ao site público da B3 e pode mudar sem aviso; não há SLA de API contratado.

São conservados o valor decimal por cota, a data-com, a data do pagamento e o ISIN. Recibos de subscrição e amortizações não são tratados como rendimento das cotas normais. O mês do gráfico é o mês do pagamento. A quantidade elegível soma as compras até a data-com, inclusive. As cotas anteriores continuam valendo sem novos lançamentos; o aplicativo não repete o valor do provento do mês anterior.

Compras novas pedem o dia da compra, no mesmo mês de referência. Os registros antigos continuam sem data exata quando ela não era conhecida. Se uma compra antiga estiver no mês da data-com e faltar o dia, o aplicativo pede essa data em vez de presumir o direito. Compras de meses anteriores continuam disponíveis para calcular o pagamento. Um rendimento mensal informado pelo usuário tem prioridade sobre a consulta, inclusive um total explicitamente igual a zero.

O gráfico distingue os cálculos automáticos dos valores do histórico, com fórmula, cotas elegíveis, data-com, pagamento e fonte na explicação de cada mês. Valores da B3 são cálculos de proventos devidos; não criam lançamentos nem confirmam crédito na corretora.

Os proventos da B3 que faltavam no histórico também entram nos rendimentos acumulados, no resultado dos FIIs e da carteira e na comparação com o CDI. A conciliação é por fundo e mês: qualquer rendimento informado pelo usuário, inclusive zero, prevalece sobre os eventos da fonte nesse mês. O cálculo acrescenta somente a parcela `apiAmount`, nunca o total do relatório mensal por cima dos recebimentos existentes. A decomposição `income.recordedAmount` / `income.apiAmount` e os totais `income.byFund` tornam essa soma conferível no app e no MCP.

Para comparar os resultados na mesma data, os pagamentos da API são limitados à data das cotações utilizadas. Pagamentos posteriores ficam fora desse resultado, mesmo que já apareçam em uma consulta mais recente. O cenário CDI continua calculando o retorno das mesmas compras; os proventos dos FIIs não são reaplicados automaticamente. Meses fora da cobertura ou com data de compra pendente permanecem identificados como informação incompleta, sem presumir pagamentos. A carteira original e os lançamentos continuam preservados; importar ou registrar depois um recebimento substitui a estimativa daquele fundo e mês, em vez de duplicá-la.

A abertura do aplicativo atualiza o cache de proventos quando necessário. Navegar e passar o mouse usam somente dados locais. O cache dura 24 horas, com no máximo 40 consultas B3 por dia em Brasília, compartilhadas entre carteiras, MCP e aplicativo. Falhas têm intervalo mínimo de cinco minutos; HTTP 429 respeita `Retry-After`. Dados salvos são mantidos durante falhas. A B3 retorna uma janela recente de eventos, observada em torno de doze meses; ausência de evento fora da cobertura não vira rendimento zero.

O cache conserva pagamentos anteriores à primeira data de pagamento retornada pela janela atual, inclusive quando a janela começa no meio de um mês. Assim, o acumulado não perde um rendimento já consultado quando esse evento deixa a janela pública. Cada evento preserva a coleta da sua versão em `lastObservedAt`; a data da consulta mais recente não substitui essa origem. A primeira data retornada e os pagamentos seguintes são substituídos pelo retrato atual da B3 para aceitar correções de taxa e datas sem duplicar pagamentos. Uma correção que cruza a fronteira do arquivo é conciliada somente por referência única de declaração ou competência, além de ISIN e tipo. Identificações ausentes ou ambíguas ficam marcadas como `reconciliationPending`, com aviso, e não viram dinheiro no cálculo; os demais eventos continuam atualizados. Respostas vazias ou inteiramente sem datas preservam o histórico anterior com aviso. Limites de tamanho suspendem a atualização e preservam o arquivo anterior, sem truncar pagamentos. Essa preservação é local: instalar em outro computador sem transferir o cache não recupera eventos que já saíram da janela da B3.

A [API de dividendos de FIIs da brapi](https://brapi.dev/docs/fiis/dividendos) e a de [outros fundos](https://brapi.dev/docs/fundos/dividendos) exigem plano Pro para cobrir a carteira completa. Por isso, o preenchimento gratuito usa a fonte B3 e não consome a cota de requisições da chave brapi.

## Informes da CVM

O conector usa os [informes mensais estruturados dos FIIs](https://dados.cvm.gov.br/dataset/fii-doc-inf_mensal), os [informes mensais FIAGRO](https://dados.cvm.gov.br/dataset/fiagro-doc-inf_mensal) e o [cadastro de fundos e classes](https://dados.cvm.gov.br/dataset/fi-cad), sem chave. Baixa os arquivos oficiais e filtra localmente: nenhum saldo, carteira, ticker ou CNPJ é enviado em parâmetros para a CVM.

Os arquivos fornecem CNPJ, mas não um vínculo confiável com ticker. Vínculos iniciais foram conferidos em documentos públicos do Fundos.NET:

| Símbolo | CNPJ do fundo | Fonte do vínculo | Resultado validado |
| --- | --- | --- | --- |
| MXRF11 | 97.521.225/0001-25 | [Fundos.NET](https://fnet.bmfbovespa.com.br/fnet/publico/exibirDocumento?cod=0&cod_neg=MXRF11&id=1272055&tipo_comunicado=rendimentos_e_amortizacoes_aviso_aos_cotistas_-_estruturado) | Informe de setembro/2026, entregue em 15/10/2026 |
| VGHF11 | 36.771.692/0001-19 | [Fundos.NET](https://fnet.bmfbovespa.com.br/fnet/publico/exibirDocumento?cvm=true&id=1272014) | Informe de setembro/2026, entregue em 15/10/2026 |
| XPML11 | 28.757.546/0001-00 | [Fundos.NET](https://fnet.bmfbovespa.com.br/fnet/publico/exibirDocumento?cvm=true&id=1296177) | Informe de setembro/2026, entregue em 15/10/2026 |
| VGIA11 | 41.081.088/0001-09 | [Fundos.NET](https://fnet.bmfbovespa.com.br/fnet/publico/exibirDocumento?cvm=true&id=1285939) e [arquivo FIAGRO CVM](https://dados.cvm.gov.br/dados/FIAGRO/DOC/INF_MENSAL/DADOS/inf_mensal_fiagro_202609.zip) | FIAGRO: informe de setembro/2026, entregue em 15/10/2026 |

Em **Mercado → Informes CVM**, é possível cadastrar, corrigir ou remover o CNPJ de cada símbolo. Os dígitos verificadores são validados. Remover um vínculo também desativa seu padrão inicial, e um relatório de outro CNPJ deixa de aparecer imediatamente. Essas configurações são locais e públicas, separadas dos dados financeiros.

A classificação usa o registro mais recente da CVM, a situação e a classe. Isso impede usar o antigo cadastro cancelado de FII do VGIA11 no lugar da sua classe atual de FIAGRO. Não são inferidas identidades por nome ou sufixo do ticker.

O painel apresenta patrimônio líquido, valor patrimonial por cota, cotistas e cotas emitidas quando publicados, com competência `referenceDate`, entrega `filedAt` e coleta `fetchedAt` separadas. `fundType` identifica FII ou FIAGRO; a fonte é o arquivo próprio de cada tipo. Campos ausentes permanecem ausentes. VP por cota é um dado contábil, não preço de negociação. Reapresentações são consideradas na escolha do informe mais recente da mesma competência. Dividendos futuros e outros tipos de documentos da CVM continuam fora desta etapa.

A base FIAGRO publica arquivos mensais referentes aos últimos doze meses e os atualiza semanalmente com apresentações e reapresentações. O vínculo atual identifica o CNPJ da classe, não uma subclasse por ticker. O conector usa o informe principal da classe; se houver múltiplas subclasses, não escolhe arbitrariamente uma delas para preencher VP por cota. Nessa situação o campo permanece ausente, mantendo os demais dados publicados da classe. O cache de FIIs da versão 0.1.0 continua válido e é interpretado como `fundType: FII`.

Os arquivos anuais/cadastro são reutilizados por seis horas entre carteiras. São limitados em tamanho comprimido/descomprimido e não são extraídos para caminhos de arquivos vindos do ZIP. O cache é compartilhado com o MCP, com bloqueio entre processos e substituição atômica. A leitura local do painel não cria nem repara caches. A consulta automática reutiliza o resultado por 24 horas, considerando também os vínculos de CNPJ; os arquivos oficiais já baixados continuam compartilhados entre carteiras.

## Consulta pelo chat

O catálogo continua com quatro ferramentas, compatível com Codex, Claude e Antigravity. Em `preparar_analise_carteira`:

```json
{
  "walletId": "ID_DA_CARTEIRA",
  "refreshQuotes": true,
  "refreshCvmData": true
}
```

Quando omitidas, ambas as opções são `false` e a ferramenta lê apenas cache. São independentes de `refreshEconomicData` e `refreshNews`. `marketData` retorna a cobertura/status por símbolo; `cvmData` retorna informes, `fundType`, fonte e limitações por CNPJ. Cada observação usada pode ser registrada como fonte na conversa, com suas datas originais. O modelo recebe os dados, nunca a chave. A mensagem inicial do servidor MCP e as instruções dos três provedores descrevem essas mesmas capacidades.

As consultas de cada lote de cotações/CVM têm orçamento de 45 segundos; uma falha conserva o cache e informa o erro. O transporte das CLIs admite 90 segundos para a ferramenta com novas consultas, incluindo os indicadores/notícias opcionais. Nenhuma dessas consultas executa ordens ou muda dados financeiros.

## Verificação

Os testes de parsing, falhas e MCP usam somente dados sintéticos e não exigem chave. Exemplos de verificação real aceitam símbolos/CNPJs e leem a chave do cofre, sem imprimi-la:

```powershell
cargo run -p ivestfii-market-data --example live_quotes -- --data-dir C:\CAMINHO\DOS\DADOS MXRF11 VGHF11 VGIA11 XPML11
cargo run -p ivestfii-market-data --example verify_cvm -- --data-dir C:\CAMINHO\DOS\DADOS\public-data MXRF11 97521225000125 VGHF11 36771692000119 XPML11 28757546000100 VGIA11 41081088000109
python -B scripts/verify-market-mcp.py --binary target/debug/examples/ivestfii-preview.exe --data-dir C:\CAMINHO\DOS\DADOS --refresh
```

A versão web e a sincronização Vercel continuam pendentes. As chaves locais não devem ser enviadas por sincronização para um futuro servidor.

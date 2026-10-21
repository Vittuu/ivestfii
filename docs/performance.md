# Resultado da carteira e evolução mensal frente ao CDI

A visão geral reúne FIIs, pós-fixado identificado e caixinhas da carteira selecionada. O total importado da corretora fica no quadro próprio, com sua data original. Caixa em conta, proventos a receber e patrimônio agregado não são somados novamente aos investimentos registrados.

## Resultado acumulado

- Total aplicado: custo das compras de FIIs, aplicações acumuladas das caixinhas e valor aplicado conhecido no pós-fixado.
- Rendimentos acumulados: proventos conhecidos dos FIIs, resultado das caixinhas e ganho acumulado no saldo do pós-fixado.
- Resultado da carteira: variação das cotas e rendimentos dos FIIs, mais os resultados do pós-fixado e das caixinhas.

Para uma caixinha, `resultado = saldo + resgates acumulados − aplicações acumuladas`. Resgates podem incluir principal e juros. O saldo já contém os rendimentos; não se soma uma projeção de juros nem o indicador de 12 meses do pós-fixado. Os proventos da API completam somente os fundos e meses sem rendimento declarado, preservando a prioridade de um valor manual, inclusive zero.

Os cartões podem reunir os últimos saldos de datas diferentes; a tabela identifica essas datas. Um registro mensal da carteira inteira exige que todos os componentes ativos tenham valores na mesma data. Saldo ausente ou base de aplicação desconhecida não equivalem a zero.

## Duas linhas mensais

O gráfico mostra **ganho acumulado em reais**, desde os primeiros investimentos, para a carteira inteira e um cenário de 100% do CDI. Aportes não aparecem como lucro. O detalhe de cada mês informa a data observada, os dois resultados e a diferença. A diferença só é calculada quando ambos os resultados são conhecidos na mesma data e usam a mesma base de movimentos.

A linha da carteira reúne registros observados e fechamentos reconstruídos com dados históricos das APIs. Para cada mês encerrado, calcula a posição com as compras e aplicações já registradas, usando o último preço ou cota disponível naquele mês. O detalhe identifica a origem, a data e a fonte de cada investimento. Um registro observado válido tem prioridade sobre a reconstrução.

Nos FIIs, o resultado é `quantidade acumulada × fechamento histórico sem ajuste − compras acumuladas + proventos conhecidos acumulados`. As cotas permanecem nos meses seguintes, mesmo quando não há novas compras. O fechamento vem do endpoint público documentado `Dados B3 /fiis/{ticker}/desempenho`, que informa a data efetiva do pregão e o preço negociado original do COTAHIST. O app ignora o retorno calculado pelo provedor, preços ajustados e P/VP; soma os próprios proventos uma única vez, respeitando a prioridade manual/API. Respostas com saltos suspeitos ou inconsistência não são usadas.

No pós-fixado, o app identifica o fundo/classe pelo cadastro oficial da CVM e consulta `VL_QUOTA` no Informe Diário. Cada lote usa o valor aplicado e a data de aplicação informados, divididos pela cota daquela data; as cotas calculadas são valorizadas pela cota histórica do mês. A reconstrução informa essas premissas: não comprova a posição pessoal antiga da corretora e não cria resgates, impostos ou mudanças de quantidade de cotas não cadastrados. A cota pública já contém as despesas do fundo; não se desconta novamente a taxa de administração nem se inventa come-cotas individual. Os valores originais da carteira permanecem preservados.

Meses sem preços, cotas de entrada ou base suficiente continuam com uma lacuna: não são preenchidos com zero, repetidos do mês anterior ou interpolados a partir do saldo atual. Caixinhas sem saldo mensal não são excluídas silenciosamente do total. Rendimentos de FIIs sem cobertura ficam identificados como incompletos; o gráfico pode mostrar o resultado conhecido, mas não afirma superar ou perder para o CDI enquanto a diferença estiver indisponível. Não é possível afirmar em que momento a carteira cruzou o CDI dentro de uma lacuna.

O marcador verde **Últimos valores** mostra o resultado completo já conhecido nos cartões, mesmo quando os investimentos têm datas diferentes. Cada componente mantém sua data original, apresentada no gráfico. Esse marcador é independente da série mensal: não liga pontos antigos, não aumenta a contagem de meses registrados e não calcula diferença contra o CDI. Se já existir um ponto mensal com a mesma data e valor, o marcador não é repetido. Carteira incompleta ou com uma data ausente não produz um marcador com subtotal apresentado como total.

O aplicativo guarda automaticamente o último registro completo de cada mês, por carteira, ao carregar ou atualizar o resultado na interface. Isso não cria compras, rendimentos recebidos ou itens na fila de sincronização. Datas mais recentes substituem o registro anterior do mesmo mês; correções na mesma data também são aceitas. Uma impressão dos dados financeiros daquele período permite identificar mudanças retroativas: uma compra ou rendimento antigo corrigido invalida o valor derivado antigo, enquanto movimentos posteriores preservam os pontos anteriores. Renomeações e horários de consulta não alteram a base econômica.

O histórico privado observado fica na tabela SQLite `performance_history` (schema9) e acompanha a exportação/restauração como `performanceHistory`. Backups anteriores têm histórico observado vazio. A reconstrução pelas APIs usa caches públicos separados e não escreve nessa tabela, nos lançamentos ou na fila de sincronização. A leitura pelo MCP é somente leitura e não captura novos registros; o relatório contém a mesma série mensal da interface e a proveniência de cada ponto.

O relatório fornece `portfolioEvolution.latestKnown` separadamente de `points`, com `fromDate`, `toDate` e as datas dos componentes. Esse valor já faz parte de `portfolio.totalResult`; não deve ser somado novamente nem tratado como saldo histórico na data mais recente do intervalo. O marcador é derivado dos dados disponíveis, sem gravação em `performance_history` e sem novas consultas de API.

## Cenário CDI

A linha CDI utiliza taxas diárias históricas **SGS12**, em percentual ao dia. Para cada aporte, o juro começa depois da data informada; os resgates reduzem o cenário na data correspondente. Nas compras antigas sem dia conhecido, mantém-se a hipótese de fim do mês, explicitada na interface. Percentual contratado de uma caixinha e a taxa anual fixada para projeções não substituem a série histórica a 100% do CDI.

O CDI pode estar disponível em um mês sem saldo real da carteira. Sem histórico diário suficiente ou sem datas de movimentos, o respectivo resultado fica indisponível. Não se inventam taxas de dias ainda não publicados. Fins de semana usam a cobertura publicada de sexta-feira; para meses sem registro real, a data do último CDI publicado daquele mês é informada no ponto.

Na reconstrução de um mês encerrado, uma cauda de até sete dias sem nova taxa publicada conserva o último fator disponível somente quando a série já retomou as publicações após o fim do mês, sem ultrapassar sete dias entre as observações. O detalhe informa a última data usada. Movimentos posteriores mantêm suas datas originais e não recebem juros inventados. Essa regra não classifica os dias como feriados nem cobre lacunas longas; um mês recém-encerrado com taxa final ainda pendente, saldos atuais e meses ainda abertos mantêm a exigência de cobertura original.

São valores brutos, sem cálculo de impostos ou taxas. Proventos dos FIIs entram uma vez no resultado e não são reinvestidos automaticamente no cenário. Ganho dividido pelo total aplicado não é uma taxa anualizada, TWR ou XIRR.

## Fontes e limites

O gráfico reaproveita o histórico diário CDI e os proventos já consultados pelo aplicativo. A primeira reconstrução consulta os fechamentos históricos que faltam por ativo/mês e os arquivos mensais de cotas CVM, priorizando os últimos meses e as datas de entrada. Os resultados são guardados incrementalmente em `public-data/historical-prices.json` e `public-data/fund-nav.json`; as próximas leituras do gráfico e do MCP são locais. O preenchimento inicial ocorre em lotes limitados, sem exigir clique a cada mês. A interface pode ler o progresso do cache enquanto a consulta está em andamento.

O histórico de preços tem orçamento próprio de 160 consultas em 24 horas, até 48 por lote, preservando o limite gratuito do provedor. Cotas CVM usam até 16 arquivos mensais por lote e orçamento de 48 arquivos por dia, incluindo o cadastro. Meses encerrados têm cache semanal; falhas e limites preservam os dados anteriores e respeitam o intervalo de tentativas. Não há consumo adicional da chave brapi para essas fontes. Somente nomes/códigos públicos dos fundos, CNPJs e períodos são usados nas consultas, sem valores dos aportes, carteira, perfil ou conversa.

A cotação atual não comprova preços históricos. Dados ajustados de mercado não podem ser somados novamente aos proventos como se fossem preços brutos. Os meses antigos não são calculados repetindo o rendimento atual ou usando um indicador de 12 meses. Datas de aplicação e valores já informados são usados como base declarada, sem pressupor depósitos originais diferentes.

- [Banco Central: série diária SGS12](https://www3.bcb.gov.br/sgspub/consultarvalores/consultarValoresSeries.do?hdOidSeriesSelecionadas=12&method=consultarGraficoPorId).
- [brapi: cotação atual](https://brapi.dev/docs/acoes/cotacao).
- [brapi: regras dos preços históricos ajustados e brutos](https://brapi.dev/docs/acoes/historico).
- [Dados B3: API e fontes dos preços de FIIs](https://dadosb3.com/api).
- [CVM: Informe Diário de Fundos e cotas históricas](https://dados.cvm.gov.br/dataset/fi-doc-inf_diario).

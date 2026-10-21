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

A linha da carteira usa registros reais dos saldos e os rendimentos disponíveis. Meses sem um registro ficam com uma lacuna: não são preenchidos com zero, repetidos do mês anterior ou interpolados a partir do saldo atual. Não é possível afirmar em que momento a carteira cruzou o CDI dentro de uma lacuna. Um único mês registrado aparece como ponto, até existirem outros registros mensais. O último registro pode ser anterior ao fim do mês, e sua data permanece visível.

O aplicativo guarda automaticamente o último registro completo de cada mês, por carteira, ao carregar ou atualizar o resultado na interface. Isso não cria compras, rendimentos recebidos ou itens na fila de sincronização. Datas mais recentes substituem o registro anterior do mesmo mês; correções na mesma data também são aceitas. Uma impressão dos dados financeiros daquele período permite identificar mudanças retroativas: uma compra ou rendimento antigo corrigido invalida o valor derivado antigo, enquanto movimentos posteriores preservam os pontos anteriores. Renomeações e horários de consulta não alteram a base econômica.

O histórico privado fica na tabela SQLite `performance_history` (schema9) e acompanha a exportação/restauração como `performanceHistory`. Backups anteriores têm histórico vazio. A migração acrescenta a tabela sem reescrever dados financeiros existentes. A leitura pelo MCP é somente leitura e não captura novos registros; o relatório contém a mesma série mensal da interface.

## Cenário CDI

A linha CDI utiliza taxas diárias históricas **SGS12**, em percentual ao dia. Para cada aporte, o juro começa depois da data informada; os resgates reduzem o cenário na data correspondente. Nas compras antigas sem dia conhecido, mantém-se a hipótese de fim do mês, explicitada na interface. Percentual contratado de uma caixinha e a taxa anual fixada para projeções não substituem a série histórica a 100% do CDI.

O CDI pode estar disponível em um mês sem saldo real da carteira. Sem histórico diário suficiente ou sem datas de movimentos, o respectivo resultado fica indisponível. Não se inventam taxas de dias ainda não publicados. Fins de semana usam a cobertura publicada de sexta-feira; para meses sem registro real, a data do último CDI publicado daquele mês é informada no ponto.

São valores brutos, sem cálculo de impostos ou taxas. Proventos dos FIIs entram uma vez no resultado e não são reinvestidos automaticamente no cenário. Ganho dividido pelo total aplicado não é uma taxa anualizada, TWR ou XIRR.

## Fontes e limites

O gráfico reaproveita as cotações e o histórico diário CDI já consultados pelo aplicativo. Não faz uma chamada de API para cada mês e mantém os limites, cache, intervalo de tentativas e tratamento de falhas existentes. Somente códigos públicos e datas são enviados aos provedores, sem os saldos da carteira.

A cotação atual não comprova preços históricos. Dados ajustados de mercado não podem ser somados novamente aos proventos como se fossem preços brutos. Saldos antigos das caixinhas e do pós-fixado não são reconstruídos usando o rendimento atual ou um indicador de 12 meses. Onde não há observação real, o aplicativo informa a falta de registro.

- [Banco Central: série diária SGS12](https://www3.bcb.gov.br/sgspub/consultarvalores/consultarValoresSeries.do?hdOidSeriesSelecionadas=12&method=consultarGraficoPorId).
- [brapi: cotação atual](https://brapi.dev/docs/acoes/cotacao).
- [brapi: regras dos preços históricos ajustados e brutos](https://brapi.dev/docs/acoes/historico).

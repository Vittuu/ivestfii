# Catálogo de fontes públicas

Pesquisa e consultas HTTP verificadas em 16/10/2026. O [public-apis](https://github.com/public-apis/public-apis) serve como índice de descoberta; autenticação, disponibilidade e limites foram conferidos nas fontes de cada serviço. A presença no índice não garante cobertura brasileira nem acesso gratuito em produção.

**Atualização de 20/10/2026:** Banco Central/SGS, RSS de Economia da Agência Brasil, cotações básicas autenticadas da brapi e informes mensais CVM de FII e FIAGRO estão integrados ao aplicativo e ao MCP. Veja [APIs públicas](apis-publicas.md) e [cotações e CVM](mercado-brapi-cvm.md). AwesomeAPI, Frankfurter e os serviços de notícias com chave abaixo continuam apenas pesquisados. As consultas brapi sem chave descritas na pesquisa de 16/10 são históricas; a validação autenticada posterior cobriu os quatro símbolos da carteira, sem demonstrar acesso gratuito a módulos detalhados.

## Fontes úteis sem chave

| Fonte | Uso possível | Endpoint de exemplo | Datas e limitações |
| --- | --- | --- | --- |
| [AwesomeAPI](https://docs.awesomeapi.com.br/api-de-moedas) | Câmbio USD/BRL e EUR/BRL | `https://economia.awesomeapi.com.br/json/last/USD-BRL,EUR-BRL` | `bid` é compra; `ask` é venda. `timestamp` informa a negociação em UTC; `create_date` usa UTC-3. Chamadas sem chave têm cache de um minuto. Consulta pública retornou HTTP 200. |
| [Frankfurter v2](https://frankfurter.dev/) | Câmbio de referência diário | `https://api.frankfurter.dev/v2/rates?base=usd&quotes=brl,eur&expand=providers` | Cada par possui `date`, `base`, `quote` e `rate`. A data pode variar entre os pares. `expand=providers` informa fontes e datas subjacentes. Sem chave ou cota diária/mensal; há controle contra abuso. Consulta retornou HTTP 200. |
| [Agência Brasil — RSS de Economia](https://agenciabrasil.ebc.com.br/feed/) | Notícias econômicas em português | `https://agenciabrasil.ebc.com.br/rss/economia/feed.xml` | Feed RSS oficial com `title`, `link`, `pubDate`, autor e categorias. Consulta retornou HTTP 200 e dez notícias. Exibir publicação e coleta separadamente; a última notícia pode ser do dia anterior. Não é cobertura dedicada de cada FII. |

AwesomeAPI e Frankfurter constam da seção de câmbio do índice. O RSS oficial da Agência Brasil foi encontrado ao verificar alternativas locais às APIs de notícias que exigem credenciais. Para a primeira integração de notícias, bastam título, data, fonte e link; não é necessário importar a íntegra dos artigos.

## Cotações e indicadores de FIIs

A [brapi](https://brapi.dev/docs) é uma alternativa brasileira, encontrada fora do índice. As regras oficiais distinguem ações de teste e os endpoints específicos de FIIs:

- [Autenticação](https://brapi.dev/docs/authentication): ações de teste PETR4, MGLU3, VALE3 e ITUB4 sem chave; sandbox limitado a vinte requisições por minuto por IP.
- [FIIs](https://brapi.dev/docs/fiis): endpoints `/api/v2/fii/` aceitam MXRF11 e HGLG11 sem chave para teste; cobertura geral requer o plano correspondente. Indicadores e relatórios têm datas de referência próprias.
- [Indicadores](https://brapi.dev/docs/fiis/indicadores): `https://brapi.dev/api/v2/fii/indicators?symbols=MXRF11` retornou HTTP 200, mas `asOfDate` foi **01/08/2026**. O preço desse registro não pode ser apresentado como cotação atual.
- Na mesma verificação, `https://brapi.dev/api/quote/MXRF11` respondeu HTTP 401 por ausência de token, apesar de a documentação descrever cotação simples como disponível em todos os planos. “Todos os planos” não significa acesso sem autenticação.

Os [planos publicados](https://brapi.dev/pricing) informam frequências e cotas distintas, e não prometem cotações em tempo real. Registrar a capacidade como dependente de configuração até validar uma chave e o recurso contratado; não substituir silenciosamente preços de mercado pelo custo médio da carteira ou por preços de outros ativos.

## Notícias com credencial

| Serviço encontrado no índice | Condições verificadas | Decisão inicial |
| --- | --- | --- |
| [Marketaux](https://www.marketaux.com/documentation) | Token obrigatório; endpoint `/v1/news/all`, filtros por idioma, assunto e entidades; retorno com `published_at`, URL e fonte. [Plano gratuito](https://www.marketaux.com/pricing): cem chamadas/dia e três notícias por chamada. | Candidato para etapa com configuração de chave; cobertura de tickers brasileiros deve ser validada antes de anunciar suporte. |
| [GNews](https://gnews.io/pricing) | Chave obrigatória; gratuito para uso não comercial, cem chamadas/dia, dez notícias por resposta e atraso de doze horas. | Opcional após configurar credencial; apresentar o atraso. |
| [NewsAPI](https://newsapi.org/pricing) | Chave obrigatória; plano Developer somente para desenvolvimento/testes, cem chamadas/dia e atraso de 24 horas. Uso em produção, inclusive interno, exige outro plano. | Não usar o plano gratuito como fonte padrão do aplicativo distribuído. |

## Contrato de integração

Cada observação deve manter valor, unidade, fonte, URL de origem, data de referência/publicação e instante de coleta. Cache offline conserva esses campos e deve indicar quando a consulta falhou. Falha de rede, HTTP 401/403/429 e ausência de cobertura são estados distintos de um resultado válido. Chamadas devem ter limite de tamanho, timeout, cache e intervalo entre atualizações.

O chat MCP deve devolver os mesmos metadados que a interface mostra. Notícias e taxas fornecem contexto para análise; não devem criar uma recomendação automaticamente nem alterar transações, saldos, dividendos ou a carteira armazenada.

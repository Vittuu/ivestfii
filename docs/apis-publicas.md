# Dados econômicos e notícias públicas

O aplicativo Windows consulta referências do Banco Central e manchetes de Economia da Agência Brasil/EBC sem chave de API. Ao abrir o aplicativo, a carteira aparece com os dados salvos enquanto uma consulta em segundo plano verifica quais fontes precisam de atualização. A **Visão geral** reúne referências econômicas, cotações, informes CVM e manchetes; **Mercado** permite conferir os detalhes e solicitar uma atualização.

O [public-apis](https://github.com/public-apis/public-apis) foi usado como ponto de partida para descobrir serviços. As fontes, unidades, credenciais e limitações foram conferidas na documentação de cada provedor. O [catálogo da pesquisa](api-catalogo.md) distingue as opções disponíveis sem chave das que exigem conta ou plano.

## Referências implementadas

| Indicador | Fonte e série | Unidade apresentada | Significado da data |
| --- | --- | --- | --- |
| CDI | [Banco Central — SGS 4389](https://www3.bcb.gov.br/sgspub/consultarvalores/consultarValoresSeries.do?method=consultarSeries&series=4389) | Percentual ao ano, anualizado em base de 252 dias úteis | Dia da observação publicada |
| Meta Selic | [Banco Central — SGS 432](https://www3.bcb.gov.br/sgspub/consultarvalores/consultarValoresSeries.do?method=consultarSeries&series=432) | Percentual ao ano | Dia ao qual se aplica a observação da meta |
| IPCA | [Banco Central — SGS 433](https://www3.bcb.gov.br/sgspub/consultarvalores/consultarValoresSeries.do?method=consultarSeries&series=433) | Variação percentual no mês | Competência mensal da observação; não representa inflação de um único dia |

CDI e IPCA usam os endpoints públicos SGS abaixo:

```text
https://api.bcb.gov.br/dados/serie/bcdata.sgs.4389/dados/ultimos/1?formato=json
https://api.bcb.gov.br/dados/serie/bcdata.sgs.433/dados/ultimos/1?formato=json
```

A meta Selic é consultada na série `432` com `dataInicial` e `dataFinal`, limitadas aos últimos trinta dias até a data atual em Brasília. O aplicativo seleciona a observação válida mais recente e descarta referências futuras, inclusive quando a fonte já publicou observações programadas.

Cada indicador mantém o percentual decimal, a unidade, a série, a URL efetivamente consultada, a data de referência (`referenceDate`) e o horário da coleta (`fetchedAt`). O horário da coleta não transforma uma referência antiga em uma taxa de hoje. As três séries possuem calendários de publicação diferentes.

As notícias vêm do [RSS oficial de Economia da Agência Brasil](https://agenciabrasil.ebc.com.br/feed/):

```text
https://agenciabrasil.ebc.com.br/rss/economia/feed.xml
```

São guardados até vinte títulos, com link, fonte e data de publicação quando disponível. A data RSS é convertida para UTC, e a interface a apresenta no horário local. Na ausência de data válida, o aplicativo informa que a publicação não possui data; não usa o horário da consulta como substituto. Os artigos completos não são armazenados. Esse feed oferece contexto econômico geral e não faz uma busca específica por cada FII da carteira.

## Consultar o CDI em uma caixinha

Ao criar ou editar uma caixinha, **Buscar CDI no Banco Central** preenche a taxa anual de referência e sua data no formulário. O saldo e o percentual contratado, como `120% do CDI`, continuam com os valores informados pelo usuário.

A taxa só passa a integrar o cadastro quando o usuário salva a caixinha. A origem BCB, a URL da série e o horário da consulta ficam preservados junto à referência. Se a taxa ou a data forem editadas manualmente, a origem passa a ser manual. Se o formulário for fechado sem salvar, a referência consultada não substitui o cadastro da caixinha.

Consultar o painel econômico também não substitui automaticamente as taxas de todas as caixinhas. A simulação usa a referência cadastrada na caixinha selecionada, com taxa constante, capitalização diária em base de 252 dias úteis e valores brutos. Ela não atualiza o saldo nem movimenta dinheiro entre carteiras.

## Cache e falhas

As consultas automáticas reutilizam cotações por uma hora, indicadores econômicos e histórico diário do CDI por seis horas, notícias por uma hora e informes CVM por 24 horas. Esses intervalos são de consulta, não de publicação: a tela conserva a data original da cotação, do indicador, do informe ou da notícia. Trocar de página não inicia outro lote. Ao trocar de carteira, os dados necessários passam pela mesma política compartilhada de cache.

Os controles persistem entre aberturas e são compartilhados com as consultas manuais e o MCP. Falhas recebem um intervalo antes de uma nova tentativa; a brapi respeita também o `Retry-After` e um orçamento local. Esse orçamento cobre este diretório de dados do app; o painel da brapi continua sendo a referência para o consumo total da chave, inclusive por outros programas.

As referências externas ficam separadas dos bancos da carteira e do histórico do chat:

```text
<diretório de dados do aplicativo>/public-data/economics.json
<diretório de dados do aplicativo>/public-data/news.json
```

As leituras desses arquivos funcionam sem conexão. As atualizações usam arquivos temporários, substituição atômica e bloqueio entre processos para que o aplicativo e o MCP compartilhem o cache. Uma resposta atrasada não substitui uma coleta mais recente.

Os indicadores são consultados independentemente. Se uma série falhar, as séries que responderam podem ser atualizadas, e a última observação válida da série com falha é mantida com suas datas originais e um aviso específico. Uma falha ao atualizar notícias mantém as manchetes anteriores. Nenhuma dessas situações produz valores ou títulos fictícios.

As conexões usam HTTPS, timeout de dez segundos e destinos fixos, sem seguir redirecionamentos. Respostas econômicas são limitadas a 256 KiB; o feed RSS, a 512 KiB. Links de notícias precisam usar HTTPS no domínio da Agência Brasil. Nenhuma requisição a essas fontes contém saldos, carteira, transações ou mensagens do chat.

## Ferramentas MCP pelo chat

Os quatro nomes já usados pelos conectores de Codex, Claude e Antigravity permanecem:

| Ferramenta | Uso |
| --- | --- |
| `carteira_snapshot` | Consulta os dados locais da carteira selecionada e as datas das observações importadas. |
| `projetar_renda` | Simula FIIs ou uma caixinha com suas premissas cadastradas, sem alterar saldos. |
| `recuperar_contexto` | Recupera o histórico canônico da conversa, independentemente do provedor. |
| `preparar_analise_carteira` | Reúne a carteira, contexto e referências públicas disponíveis para a análise do modelo. |

Em `preparar_analise_carteira`, os argumentos opcionais `refreshEconomicData` e `refreshNews` solicitam uma consulta, sujeita à cadência compartilhada. Quando omitidos ou `false`, a ferramenta usa somente o que já estiver no cache, inclusive as referências obtidas na abertura do aplicativo. Os dois controles são independentes:

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

O retorno identifica fontes, datas e falhas, permitindo ao assistente distinguir informação atualizada, cache anterior e dado indisponível. A ferramenta prepara os dados; ela própria não chama outro modelo de IA. A consulta pública não salva uma nova taxa nas caixinhas, não executa ordens e não altera o patrimônio cadastrado.

## Limites desta etapa

Além de indicadores e manchetes, a integração inclui cotações básicas brapi e informes mensais CVM de FII e FIAGRO por carteira, conforme o [guia de cotações e CVM](mercado-brapi-cvm.md). A posição importada da corretora conserva sua data e seus valores; os preços consultados ficam separados. Busca de notícias por ticker e sincronização com a Vercel continuam pendentes.

## Conferência sem consumir IA

Depois de compilar a interface e a prévia Windows, execute a migração isolada e o teste MCP com consultas públicas reais:

```powershell
python -B scripts/verify-migration.py --binary target/debug/examples/ivestfii-preview.exe
python -B scripts/verify-mcp.py --desktop --binary target/debug/examples/ivestfii-preview.exe --refresh-public-data
```

O teste confere as três séries e as manchetes recebidas, datas e URLs, releitura do cache sem pedir atualização, integridade do JSON original e contagens financeiras do banco isolado. Falhas de conexão são reportadas como falhas de consulta; não são substituídas por fixtures. Os testes Rust regulares usam exemplos sintéticos e não dependem dessas APIs estarem disponíveis.

A cotação básica brapi foi validada com a chave cadastrada para MXRF11, VGHF11, VGIA11 e XPML11. Os módulos detalhados de FIIs têm cobertura de plano própria e não são usados como cotação. AwesomeAPI, Frankfurter e APIs de notícias com chave continuam apenas pesquisadas. Os argumentos `refreshQuotes` e `refreshCvmData` de `preparar_analise_carteira` e o armazenamento seguro da chave estão documentados no [guia específico](mercado-brapi-cvm.md).

## Conferir a atualização de abertura

O exemplo nativo abaixo executa o mesmo coordenador usado ao abrir o app, repete a verificação e confere que a segunda passagem não reservou novas consultas e que o retrato financeiro da carteira permaneceu igual. Ele não inicia a interface nem imprime chaves ou saldos. Use uma cópia SQLite consistente do diretório de dados para a verificação offline.

```powershell
cargo run -p ivestfii-desktop --example verify-public-refresh --offline -- --data-dir C:\CAMINHO\DA\COPIA
```

Sem `--live`, o processo força um proxy local fechado para impedir consultas externas. Acrescentar `--live` habilita consultas públicas sujeitas à política de cache e usa a credencial guardada para aquele diretório. O relatório distingue as fontes atualizadas das reutilizadas e indisponíveis.

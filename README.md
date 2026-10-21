# IvestFII em Rust e Tauri

Migração em andamento para um aplicativo Windows com carteira local, interface web compartilhada e assistente independente de provedor. O [plano completo](docs/plano-de-implementacao.md) registra as decisões, etapas e critérios de aceite.

Para testar ou desenvolver em outro dispositivo, siga o [guia de outro computador](docs/continuar-em-outro-computador.md). O código e exemplos fictícios estão no GitHub; carteira, conversas e credenciais permanecem locais. O [guia de análise pelo assistente](docs/analise-assistente.md) explica o perfil por carteira e o uso de dados atualizados pelo MCP.

## Instalar no Windows

O pacote de distribuição é um único **IvestFII-Setup.exe**. Instale uma vez; versões assinadas publicadas no canal do projeto são verificadas ao abrir e a cada seis horas. O aplicativo baixa a atualização e a aplica quando está recolhido, sem conversa em andamento ou alterações pendentes. Também é possível verificar e aplicar em **Configurações → Atualizações**, ou desativar as verificações automáticas nas preferências.

A instalação mantém `%LOCALAPPDATA%\com.ivestfii.desktop`, preservando carteiras, conversas e a chave brapi do cofre Windows. Node.js, Rust e Python são necessários apenas para desenvolver. Geração do instalador, assinatura e publicação estão no [guia Windows](docs/distribuicao-windows.md).

## Executar a implementação nova

Pré-requisitos: Node.js, Rust estável, ferramentas C++ de build do Visual Studio e WebView2 no Windows.

```powershell
npm install
npm run tauri dev
```

A execução de desenvolvimento localiza `data/fiis_data.json` a partir da pasta do projeto e tenta importar uma única vez, sem modificar o original. Para importar outro caminho, use o formulário da carteira. A base nova fica em `%LOCALAPPDATA%\com.ivestfii.desktop`, com bancos `portfolio.sqlite`, `assistant.sqlite`, preferências e pasta `backups`. `IVESTFII_DATA_DIR` permite usar um diretório isolado.

O importador cria uma cópia byte a byte, registra SHA256 e confirma todos os registros numa transação. Repetir o mesmo arquivo não duplica os dados. Um arquivo diferente não substitui uma carteira preenchida. A exportação conserva o formato legado e os totais de dividendos informados.

O seletor **Carteira ativa** permite criar carteiras por instituição, como Nubank, Rico e Mercado Pago. O banco original permanece na Carteira principal; cada carteira nova começa vazia e tem seu próprio banco. O mesmo FII pode existir em duas instituições sem misturar os lançamentos. O lápis ao lado do seletor altera somente nome e instituição.

Em **Caixinhas CDI**, cadastre o saldo manual e a remuneração, como `120% do CDI`. Taxa CDI anual e data de referência são opcionais para guardar o saldo e obrigatórias para simular. A projeção é bruta, com taxa constante, capitalização diária e base de 252 dias úteis; não credita juros automaticamente. Os detalhes de uso, cálculo e backups estão em [carteiras e caixinhas](docs/carteiras-e-caixinhas.md).

Em **Mercado**, consulte CDI, meta Selic e IPCA no Banco Central, notícias econômicas da Agência Brasil, cotações básicas da brapi e informes mensais oficiais da CVM de FII e FIAGRO por carteira. As referências têm fonte, data e cache local. Ao abrir o app, uma atualização em segundo plano reutiliza dados recentes e respeita os controles dos planos gratuitos. A Visão geral reúne as referências e usa as cotações no resultado dos FIIs; os botões e o chat MCP compartilham os mesmos caches e controles. O botão **Buscar CDI no Banco Central** preenche a referência de uma caixinha, conservando saldo e percentual contratado; ela só é cadastrada ao salvar. A chave brapi fica no Gerenciador de Credenciais do Windows. Fontes e limites estão em [APIs públicas](docs/apis-publicas.md), [cotações e CVM](docs/mercado-brapi-cvm.md) e no [catálogo](docs/api-catalogo.md).

### Carteira completa

A visão geral mostra o patrimônio com caixa, o total em investimentos e a composição em FIIs, pós-fixado, saldo disponível e proventos da posição. Em **FIIs e posições → Posição**, o detalhamento inclui os FIIs com preços e resultados observados, o saldo bruto e líquido do pós-fixado, sua rentabilidade de 12 meses e os prazos de resgate. A instituição, a data de referência e os horários das telas acompanham os valores: as posições são importadas de documentos e não representam cotações em tempo real.

Na primeira importação da posição, o aplicativo procura um `posicao_*.json` na pasta local de dados e, depois, junto ao JSON legado. A posição validada fica no SQLite e é preservada nas próximas aberturas, mesmo sem o arquivo de origem. Para atualizar, use **Atualizar posição** ou **Configurações → Importar posição**. A importação aceita o arquivo de posição conferido com os documentos e o campo `broker_position` de um backup completo. Nenhum arquivo pessoal é incluído no código da interface ou no instalador.

O panorama completo usa os totais da mesma tela. O detalhamento da bolsa pode ter outro horário e outro valor; os dois permanecem identificados separadamente. Custos e dividendos do histórico continuam acessíveis abaixo e não são somados novamente ao patrimônio. A posição não reescreve compras antigas. A exportação da carteira inclui a posição, quando disponível, e sua restauração preserva histórico e posição numa única transação.

O pós-fixado aceita um complemento com data própria pelo mesmo **Importar posição**. Ele mostra o nome completo, saldo, valor aplicado, resultado no saldo e os aportes, mantendo o panorama anterior com sua data original. O resultado acumulado no saldo é separado do indicador dos últimos 12 meses e da data desse indicador. Saldo líquido e prazos de uma captura antiga permanecem identificados no registro anterior. O complemento não cria uma caixinha nem um lançamento de FII. Em **Resultado da carteira**, seu valor aplicado e seu ganho acumulado entram uma vez, junto aos FIIs e caixinhas; o saldo já contém esse ganho. A tabela mostra o pós-fixado e a data do saldo. Sem uma base de aplicação conhecida, o aplicativo mantém o saldo e identifica os totais incompletos. Os indicadores de 12 meses e os aportes detalhados não são somados novamente. A comparação com 100% do CDI histórico abrange FIIs, pós-fixado e caixinhas, com os mesmos aportes e retiradas até a data do saldo de cada componente. As datas ficam visíveis; saldos de datas diferentes não são projetados até hoje. Nas caixinhas, cadastre os movimentos datados que conciliam os totais aplicados e resgatados. Sem datas ou cobertura diária suficiente, o comparativo fica pendente, preservando os totais conhecidos.

Na visão geral, **Últimas compras** e **Últimos rendimentos** consultam separadamente o histórico existente. Compras antigas continuam visíveis quando os registros recentes são somente recebimentos. Cada compra mostra sua quantidade, preço e valor; quando o dia não foi informado, conserva apenas o mês. Essa apresentação não cria compras nos meses em que houve somente rendimento.

O JSON do complemento usa `type: "broker_fixed_income_observation"`, `institution`, `expectedPositionSourceHash` e `observation`. O hash vincula o complemento à posição da carteira selecionada. A observação inclui os valores, datas, aportes e nomes/hashes das fontes; caminho, hash e horário da importação são obtidos do arquivo real. A importação valida as somas em centavos e preserva uma cópia exata do arquivo. Reimportações não duplicam dados; a exportação e restauração da carteira preservam o complemento junto à posição. Para manutenção local explícita, `cargo run -p ivestfii-core --example import-broker-observation -- <banco-existente> <complemento.json>` utiliza o mesmo importador, sem abrir o aplicativo ou reimportar lançamentos antigos. Os arquivos pessoais continuam fora do Git e do instalador.

```powershell
# Somente interface web, sem publicar ou carregar dados pessoais.
npm run dev

# Testes dos núcleos, adaptadores e servidor MCP, sem consumir assinaturas de IA.
cargo test

# Verificação de tipos e build da interface.
npm run build

# Compilação do executável desktop sem instalar no Windows.
npm run tauri build -- --no-bundle
```

Os scripts de conferência usam um banco isolado em `.local/`, sem alterar a base principal ou consumir IA:

```powershell
npm run build
cargo build -p ivestfii-desktop --features custom-protocol
cargo build -p ivestfii-mcp
python -B scripts/verify-migration.py
python -B scripts/verify-complete-portfolio.py
python -B scripts/verify-mcp.py
python -B scripts/verify-mcp.py --desktop
node scripts/verify-ui.mjs
```

O executável de desenvolvimento com a interface embutida fica em `target/debug/ivestfii-desktop.exe`. A verificação nativa usa um JSON fictício versionado por padrão e compara campos exportados, backup, hash e repetição da importação. Use `--source C:\CAMINHO\backup.json` para conferir um arquivo privado sem alterá-lo. Não abre janelas nem altera o autostart. A validação do MCP usa esse mesmo banco isolado e um subprocesso real.

Se esse executável já estiver aberto, o Windows impede substituí-lo. É possível compilar a mesma aplicação como prévia sem encerrar a instância em uso:

```powershell
cargo build -p ivestfii-desktop --example ivestfii-preview --features custom-protocol
python -B scripts/verify-migration.py --binary target/debug/examples/ivestfii-preview.exe
python -B scripts/verify-mcp.py --desktop --binary target/debug/examples/ivestfii-preview.exe
```

Para conferir também consultas reais BCB/EBC e releitura do cache pelo MCP, acrescente `--refresh-public-data` ao último comando. Essa verificação exige conexão, usa somente os endpoints públicos fixos e não consome uma assinatura de IA.

Para abrir a interface atualizada em `target/debug/examples/ivestfii-preview.exe`, use **Sair** na versão anterior e depois execute a prévia. A instância única continua ativa e os dados usam a mesma pasta local.

No navegador a interface informa que o acesso aos dados ainda não está conectado. Carteira, migração e histórico funcionam pelos comandos locais do Tauri. O build web vai para `web-dist/`, preservando o `dist/` Python existente. O instalador não inclui a carteira pessoal.

## Funcionalidades implementadas

- SQLite com cálculos decimais, IDs por lançamento, fila local para sincronização futura e importação protegida.
- Carteira completa persistente: FIIs, pós-fixado e caixa, com fontes, datas e backup que restaura a posição junto ao histórico.
- Dashboard, cadastro/edição de fundos e lançamentos, histórico e projeções com reinvestimento configurável.
- Múltiplas carteiras com cadastros independentes e resumo por instituição; caixinhas com percentual do CDI, saldo e simulação bruta.
- Indicadores oficiais BCB, manchetes RSS da Agência Brasil e busca de referência CDI nas caixinhas, com datas, proveniência e cache compartilhado com o MCP.
- Chat pelas CLIs locais de Codex, Claude e Antigravity, com seleção de modelo, cancelamento e histórico persistente ao trocar de provedor. Fluxos, versões e limites em [docs/integracao-clis.md](docs/integracao-clis.md).
- MCP stdio local com ferramentas de leitura sobre o núcleo Rust; sem Computer Use. Configuração e execução em [crates/mcp-server/README.md](crates/mcp-server/README.md).
- Bandeja, instância única, início com Windows e configurações persistidas. Autostart começa desligado em desenvolvimento; em release, ligado. A opção iniciar na bandeja vale para o início automático. Abrir manualmente mostra a janela.

Vercel/Postgres, autenticação online, sincronização efetiva e cotações externas de ações e FIIs são etapas seguintes. O chat usa a carteira local, suas projeções e as referências públicas datadas; manchetes de economia não substituem pesquisa específica de um ativo.

As CLIs devem estar instaladas e autenticadas pelo fluxo oficial de cada provedor. Uma verificação de versão não comprova login. Nenhuma inferência é iniciada ao abrir o aplicativo ou iniciar o Windows. A integração usa executáveis nativos, inclusive quando um atalho de shell antigo está quebrado.

O teste real opcional abaixo consome as assinaturas das CLIs selecionadas e usa somente uma carteira vazia descartável. Confere chamadas MCP e continuidade do código de teste numa única conversa:

```powershell
cargo run -p ivestfii-runtime --example verify_cli -- codex,claude,antigravity
```

## Validação desta entrega

- `cargo test --workspace --lib --tests`: testes de carteira, posição completa, histórico, adaptadores, cancelamento, protocolo MCP e preferências. A seleção explícita evita recompilar executáveis de exemplo que possam estar em execução no Windows.
- `cargo clippy --workspace --all-targets -- -D warnings`: sem avisos.
- `npm run build` e executável Windows com a interface embutida: compilados.
- Carteira real: conferência em banco isolado, backup byte-exato, exportação equivalente e segunda importação sem duplicações. O script lê a versão atual do JSON e verifica que continua intacta.
- Posição completa: importação, reabertura e restauração nativas a partir do backup exportado, com valores e proveniência preservados.
- MCP: handshake e quatro ferramentas conferidos em subprocesso real sobre banco isolado; nenhuma gravação financeira ou consulta de IA.
- CLIs: teste real Codex → Claude → Antigravity na mesma conversa; os três executaram `carteira_snapshot` e preservaram o código de contexto. Só a carteira sintética vazia foi exposta aos provedores.
- Carteiras e caixinhas: isolamento de dados, concorrência, migração, precisão decimal, backups e cálculo CDI testados. Teste real do Claude consultou e simulou uma caixinha em uma carteira sintética separada, pelo MCP.
- APIs públicas: respostas e caches BCB/EBC validados, falhas parciais, datas e origem CDI preservadas. Consultas reais e leitura do cache são verificáveis pelo MCP com a opção acima.
- Interface revisada no navegador; arquivos de carteira e verificação retornam HTTP 403 no servidor de desenvolvimento.

Ainda falta validar interativamente bandeja, registro de autostart e diálogos no Windows, além de gerar/testar o instalador. O binário compilado foi executado no modo de verificação de armazenamento, que não altera a inicialização do Windows.

## Estrutura nova

| Diretório | Responsabilidade |
| --- | --- |
| `crates/portfolio-core` | Dados, importação, cálculos e projeções. |
| `crates/assistant-core` | Conversas, contexto, fontes e checkpoints. |
| `crates/assistant-runtime` | Adaptadores de CLIs, processos supervisionados e execução do chat. |
| `crates/mcp-server` | Protocolo MCP stdio e ferramentas locais. |
| `src-tauri` | Aplicativo Windows, comandos e ciclo de vida. |
| `src` | Interface React e adaptação para navegador. |

## Implementação Python anterior

O código Python e o backend Flask anterior permanecem no repositório para referência. São independentes do aplicativo Rust/Tauri e não implementam sua sincronização web. As instruções anteriores estão em [legado Python](docs/legado-python.md).


A comparação mensal aparece em duas linhas: ganho acumulado da carteira inteira e cenário de 100% do CDI, com os mesmos aportes e resgates. O aplicativo busca fechamentos mensais sem ajustes dos FIIs pela API aberta Dados B3 e cotas diárias do fundo pós-fixado pela CVM. As quantidades acumuladas dos FIIs e os aportes datados do pós-fixado permitem reconstruir meses encerrados. Cada ponto identifica as fontes, as datas e a base declarada usada no cálculo; não representa um extrato histórico da conta. Saldos reais arquivados têm prioridade. Movimentos de tributação ou resgate não informados não são presumidos, e caixinhas sem saldo histórico continuam com lacunas explicadas. O marcador **Últimos valores** mantém visível o resultado atual quando os componentes têm datas diferentes. O chat MCP consulta os mesmos resultados.

Os proventos antigos são preenchidos pelos avisos oficiais gratuitos do Fundos.NET, com valor exato por cota, data-base, pagamento e documento de origem. Rendimentos manuais têm prioridade, inclusive zero; eventos iguais da B3 e do Fundos.NET são contados uma vez, e conflitos ficam pendentes. Nenhum lançamento é criado ou alterado para completar o gráfico. Datas de compra desconhecidas, documentos ausentes ou cobertura incompleta mantêm o resultado identificado como parcial, sem uma conclusão contra o CDI. As consultas enviam apenas códigos públicos dos fundos e períodos; cotas, valores da carteira, perfil e conversas ficam locais. O cache compartilhado com o MCP é preenchido em lotes de até 24 requisições, com limite local de 240 por dia e por janela de 24 horas. Os registros financeiros e saldos arquivados acompanham os backups; as séries públicas podem ser consultadas novamente. Veja [o cálculo, as fontes e os limites](docs/performance.md).

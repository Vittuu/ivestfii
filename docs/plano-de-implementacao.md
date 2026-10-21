> Estado atual: aplicativo Windows funcional, atualizações assinadas, APIs públicas com cache automático e análise MCP orientada pelo perfil local. Sincronização Vercel continua pendente. Este documento também preserva o roteiro original.

# Plano de migração do IvestFII para Rust e Tauri

O IvestFII terá o Windows como experiência principal, com funcionamento offline e interface web compartilhada. A migração preserva o histórico da carteira atual, substitui os cálculos e a persistência Python por um núcleo Rust e acrescenta sincronização online, assistente com provedores intercambiáveis e ferramentas MCP diretas.

## Decisões definidas

| Área | Decisão |
| --- | --- |
| Desktop | Tauri 2, Rust e WebView2 no Windows. |
| Interface | React, TypeScript e Vite; mesma base visual para desktop e navegador. |
| Persistência local | SQLite na pasta de dados do usuário, fora do executável e do repositório. |
| Carteiras e reservas | Múltiplas carteiras por instituição, com FIIs e caixinhas separados; saldos e percentual contratado do CDI são cadastrados pelo usuário. |
| Dinheiro | Cálculos decimais no Rust; valores transportados como strings decimais. |
| Nuvem | Site e API na Vercel, Postgres conectado por integração, inicialmente Neon. |
| Disponibilidade | O desktop funciona sem rede. O navegador acessará a última versão sincronizada mesmo com o PC desligado. |
| IA | Conectores locais para Codex App Server, Claude Code CLI e Antigravity CLI `agy`, usando o fluxo oficial de autenticação de cada serviço. |
| Conversas | Histórico do aplicativo, independente das sessões das CLIs. Trocar provedor mantém mensagens e contexto reconstruído. |
| MCP | Ferramentas sobre o domínio Rust, com entradas e saídas estruturadas e sem Computer Use. |
| Contexto público | CDI, meta Selic e IPCA via Banco Central/SGS e manchetes econômicas via RSS da Agência Brasil/EBC; consultas explícitas, cache local e datas preservadas. |
| Ciclo de vida | Início com Windows configurável, fechar para a bandeja, instância única e comando Sair. |

## Proteção dos dados durante a migração

- `data/fiis_data.json` é somente leitura para a implementação nova. O aplicativo Python permanece disponível.
- A importação registra SHA256 e salva uma cópia byte a byte antes da transação SQLite.
- Importar o mesmo arquivo novamente não duplica fundos ou lançamentos. Importar um arquivo diferente em uma carteira já preenchida exige um fluxo explícito de mesclagem, ainda fora desta primeira entrega.
- Cada lançamento recebe ID estável. Mais de um lançamento no mesmo mês não é descartado nem confundido com uma edição.
- Os totais de dividendos informados no JSON são preservados. Inferências para campos em branco usam posição histórica, sem contar compras futuras.
- Erros de validação interrompem a importação, sem converter um arquivo danificado em carteira vazia.
- A aceitação compara quantidade de fundos, quantidade de lançamentos, todos os campos e valores exportados, hash do original e hash do backup.

A integridade da migração é conferida por SHA256 em relatórios locais ignorados pelo Git. Dados financeiros pessoais não integram os exemplos, o frontend ou o instalador.

## Arquitetura

```text
Interface React compartilhada
  ├─ Desktop → comandos Tauri → núcleo Rust → SQLite local
  │                            ├─ histórico canônico do assistente
  │                            ├─ ferramentas MCP de domínio
  │                            ├─ market-data → BCB/EBC → cache público local
  │                            └─ conectores Codex / Claude / Antigravity
  └─ Navegador → API autenticada na Vercel → Postgres

SQLite → fila de alterações → API de sincronização → Postgres
```

`crates/portfolio-core` concentra importação, catálogo de carteiras, persistência e cálculos de FIIs e caixinhas. `crates/assistant-core` concentra conversas, construção de contexto, fontes e checkpoints. `crates/assistant-runtime` supervisiona as CLIs e o ciclo de execução do chat. `crates/market-data` consulta indicadores BCB e manchetes EBC, mantendo cache separado dos bancos financeiros. `crates/mcp-server` expõe ferramentas stdio sobre esses serviços. `src-tauri` controla janelas, configurações e integração Windows. `src` contém a interface web.

## Entregas e critérios de aceite

### Etapa 1 Base local e migração segura

Implementar núcleo Rust sem dependência de Tauri, banco versionado, importação com backup e recibo, consulta da carteira, exportação, cadastro/edição e projeções com premissas explícitas. Construir dashboard, carteira, histórico e configurações na interface.

A base local já permite separar carteiras por instituição e cadastrar caixinhas com saldo, percentual do CDI e referência anual opcional. A consulta ao BCB preenche taxa e data no formulário; a referência só entra no cadastro ao salvar, com origem, URL e horário da coleta. Edição manual da taxa ou data passa a identificar a origem como manual. A consulta não altera saldo ou percentual contratado, e a simulação bruta não credita rendimentos.

Aceite: testes sintéticos de migração e cálculos, importação real em diretório isolado, comparação integral da exportação e compilação Windows. O JSON original conserva o hash. Não publicar a carteira em dados estáticos da web.

### Etapa 2 Histórico e ferramentas MCP

Persistir mensagens, resultados de ferramentas, fontes e tarefas. Expor consulta de carteira, projeções, recuperação de contexto e preparação de análise por MCP stdio. O servidor deve validar argumentos, negociar capacidades, retornar erros estruturados, limitar mensagens e encerrar corretamente.

Uma tarefa pode encadear várias consultas automaticamente. A camada Rust registra etapas e resultados para permitir recuperação; não exige cliques nem comandos manuais entre etapas. As cinco ferramentas atuais não alteram os dados financeiros; `simular_aporte` acrescenta o cenário por classe com orçamento informado, valores e pesos antes/depois, preservando datas e lacunas, sem calcular retorno nem executar ordens. `preparar_analise_carteira` lê o cache público e aceita `refreshEconomicData` e `refreshNews`, opcionais e desligados por padrão, para consultar BCB e EBC. Fontes e datas acompanham os resultados; falhas parciais preservam as referências anteriores com aviso. O cache e as referências da conversa podem ser gravados, sem modificar a carteira. Cotações externas de ativos permanecem pendentes.

Aceite: cliente de teste MCP executa handshake, lista e chama ferramentas, verifica erros e roda sem Computer Use. Um reinício preserva a conversa. Uma troca de provedor conserva objetivos e resultados disponíveis no histórico.

### Etapa 3 Conectores de IA e análise fundamentada

Integrar Codex App Server e modos programáticos oficiais de Claude e Antigravity em processos supervisionados. Login e credenciais ficam nos mecanismos oficiais locais. Separar modo assinatura de API por consumo. Listar capacidades e modelos disponíveis, exibir streaming, cancelamento, falhas e limites. Não executar modelos no início do Windows.

Os adaptadores usam a mesma conversa canônica. O construtor de contexto reúne instruções, perfil, mensagens recentes, resumo rastreável e trechos anteriores relevantes dentro da janela do modelo. O histórico completo permanece consultável; o estado interno privado de um modelo não é transferido. O extrato determinístico inicial não equivale a um resumo semântico nem resolve sozinho a seleção por tokens.

Já disponíveis: consulta de carteiras e caixinhas, simulação de cenários, indicadores BCB e manchetes econômicas EBC com atualização explícita pelo chat. Os conectores usam esses dados no contexto do provedor escolhido; a ferramenta de preparo não chama outro modelo nem substitui as taxas cadastradas.

Permanecem no roteiro: perfil financeiro estruturado, cotações de ações/FIIs, busca de notícias por ativo e uma análise fundamentada na adequação ao prazo, liquidez e perfil. As respostas devem apresentar datas, fontes, hipóteses e riscos. O noticiário econômico geral não equivale a uma pesquisa completa sobre cada ativo, e os indicadores não constituem um feed de negociação. Os cálculos permanecem no Rust.

Aceite: análise real com um provedor, troca para outro mantendo decisões e pendências, teste de conversa longa e recuperação após queda de processo. Chamadas reais são feitas apenas em testes explícitos, sem disparar consumo oculto no bootstrap.

### Etapa 4 Sincronização Vercel e Postgres

Validar implantação da API Rust na Vercel; o runtime oficial consultado está em beta. Implementar autenticação, isolamento por usuário, contratos versionados, IDs de operação, revisões no servidor, exclusões registradas e consulta incremental por cursor. Nunca sincronizar o arquivo SQLite inteiro.

Cada gravação local e item de fila entram na mesma transação. Reenvios são idempotentes. Edições concorrentes do mesmo registro geram conflito visível, preservando ambas as versões. O cliente sincroniza ao iniciar, após alterações e ao recuperar a conexão. Mostrar pendências e a última sincronização confirmada. Credenciais dos provedores não entram nesse fluxo. Sincronização de conversas/relatórios tem configuração própria.

Aceite: editar offline, reconectar sem duplicar registros, conferir no navegador, detectar conflito entre dois clientes e reabrir após uma falha entre envio e confirmação. A API não será exposta sem autenticação e banco configurados.

### Etapa 5 Distribuição e operação no Windows

Integrar inicialização automática, iniciar na bandeja, fechar para bandeja, restaurar janela e instância única. Ao fechar a janela, tarefas já iniciadas continuam. Sair salva o estado, encerra processos supervisionados e termina o aplicativo. Desligamento do sistema tem espera limitada. Processos de IA são iniciados sob demanda.

No build de desenvolvimento a inicialização automática começa desligada para não registrar caminhos temporários de build no Windows. Na distribuição release, a preferência inicial é ligada e pode ser desativada nas configurações. Uma desativação feita pelo Windows não é revertida a cada execução.

Aceite: instalar, iniciar sessão Windows, alternar configurações, fechar/restaurar várias vezes, abrir um segundo atalho, sair sem processos órfãos e preservar pendências no próximo início. Produzir instalador NSIS e registrar versão/schema para futuras atualizações.

## Estado da primeira implementação

Esta seção preserva o registro da primeira implementação. O catálogo atual tem cinco ferramentas: `simular_aporte` foi acrescentada às quatro iniciais para calcular a divisão de um orçamento por classe. Seu [contrato atual](integracao-clis.md#próximos-aportes-e-cenário-por-classe) descreve as bases datadas e as limitações; as validações históricas abaixo não representam testes reais adicionais de provedores.

| Etapa | Estado nesta entrega |
| --- | --- |
| Base local e migração | Implementadas; importação real conferida, original intacto e exportação equivalente. |
| Carteiras e caixinhas | Múltiplas carteiras, saldos manuais, percentual do CDI e simulação bruta implementados; referência manual ou BCB conserva fonte e datas no cadastro. |
| Histórico e MCP | Histórico canônico e quatro ferramentas implementados, com auditoria, recuperação de turnos interrompidos e consultas públicas opcionais sem alteração financeira. |
| Conectores de IA | Adaptadores locais implementados com streaming, cancelamento, replay de contexto e ferramentas MCP. Compatibilidade e validação real descritas em `docs/integracao-clis.md`. |
| Indicadores e notícias | BCB/SGS e RSS de Economia EBC implementados, com consulta explícita e cache offline; cotações por ativo, busca por ticker e perfil estruturado pendentes. |
| Vercel e Postgres | Plano e fila local preparados; autenticação, servidor e sincronização online pendentes. |
| Ciclo de vida Windows | Código de bandeja, instância única e preferências compilado; validação interativa e instalador pendentes. |

O estado verificado e os comandos reproduzíveis ficam no README. Funcionalidades não conectadas aparecem como indisponíveis, sem dados fictícios, sem respostas de IA simuladas e sem indicador de sincronização concluída quando não houve comunicação com o servidor.

Os fluxos entregues estão descritos em [carteiras e caixinhas](carteiras-e-caixinhas.md), [integração das CLIs](integracao-clis.md) e [APIs públicas e cache](apis-publicas.md). O [catálogo de APIs pesquisadas](api-catalogo.md) distingue fontes ativas de possibilidades que ainda exigem integração ou credenciais.

## Referências técnicas

- [Tauri 2](https://v2.tauri.app/start/), [bandeja](https://v2.tauri.app/learn/system-tray/), [inicialização](https://v2.tauri.app/plugin/autostart/).
- [Codex App Server](https://learn.chatgpt.com/docs/app-server), [Claude programático](https://code.claude.com/docs/en/headless), [Claude e assinatura](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan), [Antigravity headless](https://www.antigravity.google/docs/cli/headless/).
- [Ferramentas MCP](https://modelcontextprotocol.io/specification/2025-11-25/server/tools).
- [Rust na Vercel](https://vercel.com/docs/functions/runtimes/rust), [Postgres integrado](https://vercel.com/docs/postgres), [armazenamento persistente e SQLite](https://vercel.com/kb/guide/is-sqlite-supported-in-vercel).

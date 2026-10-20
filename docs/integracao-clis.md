# Chat local e integração das CLIs

O chat do IvestFII usa os executáveis oficiais instalados no Windows para conversar com Codex, Claude e Antigravity. O aplicativo mantém seu próprio histórico em SQLite e reconstrói o contexto a cada mensagem. A troca de provedor conserva a conversa; não depende de importar arquivos internos ou transferir sessões privadas entre serviços.

Este documento descreve o contrato implementado. Compilação e testes de protocolo não comprovam login, disponibilidade de modelo ou uma chamada real bem-sucedida. A autenticação e os limites continuam sujeitos à CLI, à conta e ao plano do provedor.

## Fluxo no aplicativo

1. Abra **Assistente**, selecione uma conversa ou **Nova conversa**.
2. Escolha Codex, Claude ou Antigravity. O campo **Modelo** aceita um identificador do provedor; vazio usa o padrão anunciado pelo provedor. O Codex consulta seu catálogo e valida escolhas explícitas antes de enviar o contexto.
3. Envie uma pergunta pelo botão ou por `Ctrl + Enter`. O host grava a mensagem uma vez e inicia um processo supervisionado em segundo plano.
4. Codex e Claude exibem texto durante a resposta. No Antigravity, o chat exibe o progresso e as ferramentas e mostra o texto depois de validar a resposta final. Quando a IA consulta o MCP, a linha do tempo mostra a ferramenta, os argumentos, o resultado e seu estado. As chamadas encadeadas não exigem abrir um terminal nem operar telas.
5. **Interromper** cancela a execução. A mensagem do usuário e o texto parcial ficam preservados; uma resposta parcial é identificada como tal.
6. Após concluir ou interromper, escolha outro provedor e continue na mesma conversa. A seleção não cria uma conversa nova.

O seletor de provedor e o modelo ficam bloqueados durante a resposta da conversa selecionada. É possível abrir outra conversa; o trabalho já enviado continua no host. Ao voltar, o frontend consulta o estado da execução e o histórico. Fechar para a bandeja mantém o aplicativo e as tarefas; **Sair** encerra os processos supervisionados.

No navegador, as ações das CLIs locais ficam desabilitadas. A sincronização Vercel, a autenticação online e o acesso remoto ao chat ainda precisam de implementação.

## Detecção e autenticação

`get_providers` localiza os executáveis e executa uma verificação limitada de `--version`, fora da thread da interface. Um resultado `ready: true` significa que a CLI respondeu à verificação; não confirma login, saldo, cota ou acesso ao modelo.

O usuário conclui o login nos mecanismos oficiais de `codex`, `claude` e `agy`. A integração preserva esse fluxo e não copia credenciais para o banco ou para a interface. No Windows, o executor usa binários nativos e informa erro quando só encontra um atalho de shell incompatível.

Os processos iniciados para o chat removem variáveis de API como `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `GOOGLE_API_KEY` e `GEMINI_API_KEY`, para impedir que uma variável herdada substitua silenciosamente o fluxo da assinatura. Cada provedor ainda decide a disponibilidade e a contabilização conforme sua conta.

Nenhuma inferência é iniciada pelo autostart do Windows ou pela abertura da tela. As CLIs são chamadas para responder somente depois de enviar uma mensagem.

## Contrato entre React e Tauri

Os argumentos e os resultados JSON usam `camelCase`. Os comandos principais são:

| Comando | Argumentos | Resultado |
| --- | --- | --- |
| `get_providers` | nenhum | Lista com `id`, `name`, `installed`, `executable`, `transport`, `ready`, `version` e `error`. |
| `list_conversations` | nenhum | Conversas persistidas. |
| `create_conversation` | `title` | Conversa criada, com ID estável. |
| `get_messages` | `conversationId` | Mensagens canônicas na ordem persistida. |
| `start_chat_turn` | `conversationId`, `provider`, `model` opcional, `content` | Snapshot `ChatRun`; o próprio comando grava a mensagem do usuário. |
| `get_chat_run` | `conversationId` | Último `ChatRun` conhecido ou `null`. |
| `cancel_chat_turn` | `runId` | Solicitação de cancelamento; o evento posterior confirma o estado terminal. |
| `get_tool_events` | `conversationId` | Registro das chamadas MCP. |
| `build_chat_context` | `conversationId`, `maxChars` | Prévia limitada do histórico canônico, com cortes identificados. |

`ChatRun` contém `id`, `conversationId`, `provider`, `model`, `status`, `text`, `error`, `createdAt`, `updatedAt` e `statusMessage` opcional. Os estados expostos à interface são `running`, `completed`, `failed` e `cancelled`.

O evento Tauri `chat-run` publica o snapshot completo. A interface se inscreve no evento e consulta mensagens, execução e ferramentas a cada segundo enquanto a conversa está aberta. A consulta recompõe o estado depois de navegar ou perder um evento. Snapshots atrasados não podem transformar uma execução terminal em `running`, encurtar o texto já recebido nem reverter uma ferramenta concluída para execução.

O frontend não chama `append_message` antes de `start_chat_turn`, evitando salvar a pergunta duas vezes. A resposta final persistida substitui visualmente o cartão de streaming; respostas parciais de execuções canceladas ou falhas não são apresentadas como respostas concluídas.

## Histórico e continuidade

`assistant.sqlite` guarda conversas, mensagens, checkpoints, chamadas de ferramentas e referências de fontes. `portfolio.sqlite` mantém os dados financeiros. As preferências de seleção da interface usam armazenamento local do WebView; o histórico da conversa permanece no banco.

Cada turno abre uma sessão nova do provedor e envia o histórico canônico reconstruído, incluindo papéis e a pergunta atual. O contexto de mensagens usa um orçamento local de 64.000 caracteres. Resultados recentes de até 12 ferramentas têm um orçamento adicional de 16.000 caracteres. Esses limites são de caracteres, não uma contagem de tokens ou garantia de caber na janela de qualquer modelo. O histórico completo continua armazenado, e a ferramenta `recuperar_contexto` permite consultar a conversa.

Resultados antigos de ferramentas são identificados como potencialmente desatualizados. Para números atuais, o assistente deve consultar novamente a carteira. O contexto instrui a IA a distinguir dados observados de hipóteses e a não inventar notícias, preços externos ou garantias de rendimento.

Pergunta e checkpoint inicial são gravados na mesma transação. A conclusão grava resposta final e checkpoint juntos, impedindo duplicar a resposta ao repetir a finalização. Falhas e cancelamentos preservam texto parcial no checkpoint. Ao reiniciar o aplicativo, um turno que ficou `running` é marcado como interrompido e aparece como falha recuperável; não é reenviado automaticamente ao provedor.

## Ferramentas MCP disponíveis

O chat oferece o mesmo catálogo de quatro ferramentas aos três provedores. O executável desktop também hospeda o transporte stdio em `--mcp --data-dir <diretório>`, antes de iniciar qualquer janela ou registro de autostart. Isso permite empacotar o chat sem exigir que o usuário inicie um servidor à parte.

| Provedor | Quem executa a conexão MCP | Atualizações no chat |
| --- | --- | --- |
| Codex | O App Server conecta ao servidor local configurado pelo aplicativo. | Texto incremental, estados e chamadas de ferramentas. |
| Claude | A CLI conecta ao servidor local com configuração MCP específica do turno. | Texto incremental, estados e chamadas de ferramentas. |
| Antigravity | O runtime do IvestFII mantém a conexão MCP real e executa as ações estruturadas escolhidas pelo modelo. | Estados, chamadas de ferramentas e texto final validado. |

O Antigravity 1.1.14 exige esse fluxo de compatibilidade: a configuração MCP apenas no workspace não foi suficiente para disponibilizar as ferramentas nativas da CLI. O aplicativo obtém o catálogo por `tools/list` e o apresenta ao modelo com o schema de cada ferramenta. A CLI responde com um envelope `action`, `tool`, `arguments` e `text`; o runtime valida a ação, executa `tools/call` no servidor real e devolve o resultado à mesma sessão da CLI. Esse ciclo permite até oito chamadas de ferramenta por turno, sem interação manual entre elas.

Na versão 1.1.14, o contrato passado por `--json-schema` usa `arguments` como **string contendo JSON**, por compatibilidade com o backend do provedor. Por exemplo: `{"action":"tool","tool":"carteira_snapshot","arguments":"{}","text":""}`. O runtime interpreta essa string e exige um objeto JSON válido antes de chamar o MCP; também aceita um objeto recebido diretamente por compatibilidade. O argumento enviado a `tools/call` é sempre um objeto validado, sem encaminhar texto arbitrário como comando.

O Antigravity usa o modo de planejamento padrão, sem agente personalizado nem desativação de permissões. Sua ferramenta interna `finish`, usada para finalizar a saída estruturada, é esperada e não representa uma consulta da carteira nem um efeito externo. Ela não aparece como ferramenta MCP auditada. O runtime mantém separado esse mecanismo de protocolo das quatro ferramentas do IvestFII.

O JSON de controle e o raciocínio privado não são mostrados como mensagem do assistente. Somente um envelope final `action: "answer"` validado publica o texto da resposta. Esse caminho não depende de alterar as configurações MCP globais do Antigravity, nem afirma que a CLI conectou diretamente ao servidor. A auditoria e a interface de ferramentas são as mesmas usadas para os outros provedores.

| Ferramenta | Uso |
| --- | --- |
| `carteira_snapshot` | Consulta fundos, lançamentos e totais calculados pelo núcleo Rust. |
| `projetar_renda` | Executa uma simulação local e devolve suas premissas. `monthlyCotas` representa cotas por fundo por mês; o reinvestimento usa custo médio local. |
| `recuperar_contexto` | Recupera contexto de uma conversa por `conversationId`, com `maxChars` e truncamentos explícitos. |
| `preparar_analise_carteira` | Reúne carteira, contexto opcional e fontes já registradas para o modelo analisar. |

As quatro ferramentas são somente leitura sobre carteira e contexto. O runtime registra separadamente a auditoria de cada chamada em `assistant.sqlite`: `id`, `conversationId`, `taskId`, `toolName`, `input`, `output`, `status`, `startedAt` e `completedAt`. O frontend exibe prévias limitadas de payloads; o registro persistido conserva o conteúdo completo recebido.

Os adaptadores permitem somente o MCP da carteira e recusam interações fora desse contrato. Eles não dependem de Computer Use. `preparar_analise_carteira` pode consultar CDI/Selic/IPCA do Banco Central com `refreshEconomicData: true`, manchetes de Economia da Agência Brasil com `refreshNews: true`, cotações básicas brapi com `refreshQuotes: true` e informes mensais CVM de FII e FIAGRO com `refreshCvmData: true`. Sem essas opções, lê apenas o cache local. As respostas identificam fonte, tipo do fundo, datas e falhas. Manchetes gerais não constituem busca específica de um ativo. As consultas não alteram posições e não enviam ordens. A ferramenta prepara dados; a resposta em linguagem natural vem do provedor escolhido. Consulte [APIs públicas](apis-publicas.md) e [cotações e CVM](mercado-brapi-cvm.md).

## Processos e limites

`crates/assistant-runtime` contém o supervisor e os adaptadores: Codex App Server por stdio, Claude em modo JSON e Antigravity em modo JSON com ciclo MCP gerenciado pelo runtime. Cada turno usa um diretório próprio em `cli-workspaces` para as configurações de execução necessárias ao provedor. O histórico canônico não depende dos arquivos desse diretório.

No Windows, os processos são iniciados sem janela e associados a um Job Object. Encerrar o turno, cancelar ou sair do aplicativo fecha o processo iniciado e seus descendentes MCP. A integração não encerra instâncias independentes das CLIs abertas pelo usuário. O supervisor limita o turno a 10 minutos e o texto final a 256.000 caracteres; erros de protocolo e limites são apresentados como falha, sem simular uma resposta.

Uma CLI sem login, incompatível ou indisponível deve apresentar um erro acionável. Corrigir a instalação ou autenticação e enviar uma nova mensagem permite continuar a mesma conversa. A recuperação automática do estado não refaz inferências nem promete retomar a execução interna de um provedor.

## Verificação

`npm run build` verifica os tipos e empacota a interface. Os testes Rust dos adaptadores usam eventos de protocolo e não consomem uma assinatura; os testes do armazenamento verificam a persistência atômica e a continuidade entre provedores. Testes reais de inferência devem usar uma conversa e uma carteira sintéticas em diretório isolado, sem modificar o JSON ou o banco principal.

O exemplo optativo `cargo run -p ivestfii-runtime --example verify_cli -- codex,claude,antigravity` faz chamadas reais às CLIs, usando uma carteira vazia descartável e a mesma conversa entre provedores. Ele exige uma ferramenta MCP concluída e a retenção de um código sintético no histórico, além do estado de conclusão. Diferentemente dos testes de protocolo, esse comando usa a conta da CLI e pode consumir sua cota. O resultado deve ser conferido por provedor, inclusive quando apenas parte da sequência passa.

O estado de uma validação real deve registrar, por provedor, versão da CLI, modelo quando informado, conclusão ou erro, evidência de ferramenta MCP executada e continuidade da conversa. Um build aprovado, uma CLI detectada ou um teste com eventos sintéticos não deve ser descrito como inferência real aprovada.

Validação real em 16/10/2026: a sequência Codex → Claude → Antigravity terminou com sucesso na mesma conversa e carteira descartável vazia. Cada provedor executou `carteira_snapshot` via MCP e respondeu com o mesmo código `CONTEXTO-7462` e zero fundos. Não foram enviados dados da carteira pessoal.

| CLI instalada | Versão | Consulta MCP real | Continuidade ao trocar |
| --- | --- | --- | --- |
| Codex | 0.147.0 | Concluída | Código registrado no histórico canônico. |
| Claude Code | 2.1.263 | Concluída | Recuperou o código da conversa iniciada no Codex. |
| Antigravity | 1.1.14 | Concluída pelo MCP gerenciado | Recuperou o código após Codex e Claude. |

O Codex selecionou `gpt-5.6-sol`, o padrão anunciado pelo catálogo dessa instalação. Claude e Antigravity usaram os padrões das respectivas CLIs. Os testes de inferência demonstram esse cenário e essas versões; os testes determinísticos cobrem falhas, cancelamento, limites, protocolos e persistência.

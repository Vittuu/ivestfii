# Histórico local do assistente

Esta biblioteca guarda a conversa como dado do aplicativo. Trocar entre Codex,
Claude e Antigravity não cria outro histórico: cada mensagem recebe um UUID,
conversa, papel, provedor/modelo opcionais e data. A ordem usa uma sequência
SQLite, evitando reordenar mensagens criadas no mesmo milissegundo.

`AssistantStore::open(path)` cria somente suas tabelas `assistant_*`. Pode ser
usado em um arquivo independente ou compartilhar o banco do aplicativo, sem
alterar `PRAGMA user_version` ou tabelas da carteira. Mensagens, registros de
ferramentas, fontes e checkpoints ficam persistidos. Resultados de ferramentas
finalizados não podem ser sobrescritos.

## Contexto limitado

`build_context(conversation_id, max_chars)` devolve conteúdo de mensagens dentro
de um limite explícito de caracteres Unicode. Metadados JSON e tokens do modelo
não são equivalentes a esse limite: o futuro conector deverá também respeitar o
orçamento real de tokens e reservar espaço para instruções de protocolo e saída.

- Todas as mensagens `system` são mantidas. Se elas não couberem, ocorre erro
  explícito; nenhuma instrução é descartada silenciosamente.
- Mensagens recentes são priorizadas. Uma mensagem maior que todo o orçamento
  disponível pode chegar como fragmento final, indicado por `truncatedMessageIds`.
- `truncated` e `omittedMessageCount` informam a perda de contexto na requisição.
- `summary` é um extrato determinístico rotulado, sem interpretação por IA. Não
  garante preservar todas as decisões nem substitui busca semântica ou memória
  revisada. Usa IDs abreviados para localizar as mensagens originais na conversa.
- Os originais continuam íntegros em `messages(conversation_id)`.
- Fontes e eventos completos são recuperados por `sources` e `tool_events`.
  Não são anexados implicitamente fora do orçamento de contexto.

## Estado dos conectores

`detect_providers()` verifica arquivos executáveis no PATH e em diretórios
conhecidos, incluindo `%LOCALAPPDATA%/agy/bin`. Não executa comandos, não lê
credenciais e não confirma login ou disponibilidade do modelo.

`connector_specifications()` descreve as capacidades implementadas pelos
adaptadores de `crates/assistant-runtime`, sem afirmar que uma conta está
autenticada. O runtime verifica versões separadamente, supervisiona processos,
envia o contexto canônico, transmite respostas e registra chamadas MCP. Os
detalhes e as condições de compatibilidade ficam em
[integração das CLIs](../../docs/integracao-clis.md).

Esta biblioteca persiste dados; o runtime executa os turnos. Pergunta e tarefa
são gravadas atomicamente por `start_chat_task`; resposta final e conclusão por
`complete_chat_task`. Checkpoints e ferramentas finalizados são imutáveis. Na
reabertura, o runtime sinaliza tarefas interrompidas e preserva parciais; não
refaz inferências automaticamente.

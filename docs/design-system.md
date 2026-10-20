# Interface e experiência do IvestFII

## Direção: "Instrumento"

Escolhida pelo usuário em 16/10/2026 entre três alternativas (Instrumento, Editorial e Nativo Windows 11). Os mockups comparados ficam em `.local/mockups/` (fora do controle de versão).

O usuário gosta do layout em **blocos**, mas não da estética de painel gerado por IA. A direção Instrumento mantém os blocos e os organiza como um painel de instrumento financeiro, com hierarquia clara e só os enfeites que informam algo.

- Grafite quente com um único destaque âmbar.
- IBM Plex Sans na interface e IBM Plex Mono em números, tickers e datas.
- Blocos de cantos quase retos, colados por linhas de 1 px.
- Um número dominante por tela; os demais ficam subordinados.
- Data de referência visível; origem e horários técnicos acessíveis nos detalhes, sem repetir notas na tela inteira.

### O que não fazer

Estes traços faziam a versão anterior parecer gerada por IA:

- Menta/teal sobre azul-marinho; gradientes, brilho e sombras grandes.
- Eyebrows em CAIXA ALTA espaçada sobre os títulos; títulos poéticos ou de marketing ("Seu patrimônio, em perspectiva."); títulos em dois tons; slogans no rodapé.
- Cartão promocional do assistente com sparkles e chips de provedores; cartão "Seus dados. Seu controle." na barra lateral; avatar e pílula "Modo local".
- Ícone decorativo no canto de cada cartão; seta ↗ em todo link; pontinhos de status com micro-rótulos.
- Quatro cartões de KPI iguais, sem hierarquia; contagens com zero à esquerda ("04"); percentuais com 4 casas.
- Gráfico de rosca decorativo com texto no centro.

## Tokens

Todos em `src/styles/tokens.css`. Telas não definem cores próprias.

| Grupo | Tokens | Uso |
| --- | --- | --- |
| Superfícies | `--bg-sidebar #121110`, `--bg-app #161514`, `--surface #1d1c1a`, `--surface-2`, `--surface-zebra`, `--surface-3`, `--surface-inset` | Degraus de grafite quente |
| Linhas | `--line #3a3632`, `--line-strong`, `--line-control #7a7268` | Rejunte, botões, contorno de campos (3,6:1) |
| Texto | `--text-1 #ece7df`, `--text-2 #b3aca2`, `--text-3 #918a80` | Contraste sobre `--surface`: 13,8 · 7,6 · 5,0 |
| Destaque | `--accent #e6a23c`, `--accent-dim #a8792f` | Ação primária, item ativo, foco e no máximo um destaque por gráfico; séries históricas em `--accent-dim` |
| Resultado | `--pos`, `--neg`, `--warn` | Só para positivo/negativo/atenção, sempre com sinal ou rótulo |
| Participação | `--share-bar` | Barras de participação em cinza quente, nunca âmbar |
| Tipografia | `--font-ui` (IBM Plex Sans), `--font-num` (IBM Plex Mono); 12, 13, 14, 16, 20, 28, 48 px | Texto auxiliar a partir de 12 px; 48 px só no número principal |
| Espaço e forma | múltiplos de 4 px; raios de 2–4 px; linha de tabela 36 px; cabeçalho de tabela 32 px; controles de 32 px (44 px em toque) | Ritmo vertical constante |

As fontes são empacotadas via `@fontsource/ibm-plex-sans` e `@fontsource/ibm-plex-mono` (subconjunto latin), então funcionam offline.

## Estrutura

- **Shell** (`src/App.tsx`, `src/styles/base.css`, `src/styles/shell-controls.css`): a barra lateral sempre inicia recolhida, com ícones e nomes acessíveis; pode ser expandida durante a sessão para mostrar o seletor de carteira e os rótulos. A largura dos controles deve caber na área interna, sem gerar rolagem horizontal. O conteúdo ocupa toda a largura disponível, reservando apenas a faixa necessária para o assistente. A barra superior mostra o título da tela (`h1`), a carteira aberta, as ações da tela e o sino de notificações. As telas colocam suas ações ali com `<PageActions>` (`src/components.tsx`).
- **Grupos e blocos**: `.group-head` titula um grupo. `.modular` cola blocos relacionados com rejunte de 1 px numa grade de 12 colunas (`.span-N`). `.block` tem `.block-head` (`SectionHeading`), `.block-body` e `.block-foot` (fonte e notas).
- **Números**: `.figure-hero` para o número principal, `.figure` e `.readouts` para os demais. Use a classe `.num` (mono, tabular). Formate com `money`, `amount`, `percent` (2 casas), `monthShort` ("out/26") e `dateBR` de `src/lib/bridge.ts`.
- **Tabelas**: cabeçalho em 12 px cinza, linhas zebradas, números à direita em mono, `ShareBar` para participação.
- **Abas de seção** (`Tabs` em `src/components.tsx`): texto com filete âmbar de 2 px na aba ativa; setas, Home e End trocam de aba. FIIs e posições usa "Posição" (fundos listados, pós-fixado e saldo; o patrimônio e a composição ficam só na Visão geral) e "Histórico" (fundos e lançamentos). Ao entrar pela navegação, abre em Posição quando existe importação; sem posição importada, abre no histórico.
- **Explicações**: definições e ressalvas ficam em dicas (`Def`, sublinhado pontilhado, lidas pelo leitor de tela), não em notas cinza ou rodapés. A data do valor continua visível; origem, coleta e demais metadados ficam disponíveis em detalhes.
- **Simplicidade (revisada após retorno do usuário em 20/10/2026)**: valores e tabelas usados no dia a dia devem estar visíveis, com frases curtas que expliquem a leitura. Não esconda todas as informações em seções recolhidas. Detalhes devem explicar o item escolhido com valores concretos; caminhos, IDs, nomes de PDFs e notas técnicas pertencem à fonte de importação. Não transforme notas técnicas em uma parede de avisos. Preserve datas e sinalize inconsistências junto ao valor afetado, com uma notificação curta para consultar as observações.
- **Dicas de verdade**: `Def` usa `HelpTooltip`, disponível ao passar o mouse, focar pelo teclado ou tocar/clicar. Escape fecha só a dica; clicar fora também a dispensa. O texto deve explicar o significado em português simples, sem repetir o rótulo. A dica respeita as bordas da janela e aparece também sobre diálogos e tabelas, sem ocupar espaço permanente.
- **Estilos por tela** ficam em `src/styles/<tela>.css`, escopados pela classe raiz da tela e importados por `src/styles/views.css`.

## Movimento

Sutil e só de entrada (seção "Movimento" de `src/styles/base.css`, tokens `--dur-*` e `--ease-out`):

- Telas e abas surgem com esmaecer e subida de 4 px em 180 ms; modais, fundo escurecido e avisos, no mesmo ritmo.
- Barras de participação e do gráfico de proventos crescem a partir da base (320 ms); a linha de projeção aparece logo depois dos eixos.
- Hover e foco mudam cor e fundo em 120 ms; detalhes recolhíveis abrem deslizando quando o navegador interpola altura automática.
- Atualizar dados não anima nada, e sem contagem de números. Tudo desliga com "reduzir movimento" do sistema; o teste de interface roda assim.

## Hierarquia financeira

- **Posição importada:** tem instituição, data e horários da fonte. O patrimônio usa o panorama importado, sem somar de novo caixa, custos ou proventos já incluídos.
- **Observações da importação:** a tela mostra uma notificação compacta; o detalhe deve indicar o assunto, os valores relevantes e onde aparecem. O texto original continua disponível para conferência, sem dominar o painel. Diferenças de captura e informações incompletas não devem ser apresentadas como vários erros genéricos.
- **Histórico de FIIs:** custo de aquisição, lançamentos e proventos registrados ficam em grupo próprio, marcado "Não somado ao patrimônio" (com a explicação na dica) quando há posição importada. Não representam cotação atual nem saldo disponível.
- **Compras e rendimentos (revisado após retorno do usuário):** o histórico mostra duas listas visíveis, Compras e Rendimentos, para o fundo escolhido. Compras mostra mês, cotas, preço e total pago. Rendimentos mostra mês e valor recebido. Um lançamento que contém ambos fornece a compra à primeira lista e o recebimento à segunda, mantendo o mesmo registro salvo. Não use filtro de tipos nem categorias misturadas na lista principal. Registros sem compra ou recebimento continuam acessíveis nos detalhes. Preço sem quantidade não representa compra; zero informado permanece zero; total vazio usa o cálculo do núcleo. Registros distintos no mesmo mês continuam distintos e sem datas inventadas.
- **Parte do valor comprado:** substitui o rótulo "Participação no custo" e explica o percentual do dinheiro usado nas compras de cada fundo sobre o total das compras. Use um exemplo simples: R$ 200 de R$ 1.000 = 20%. Não confunda esse percentual com rentabilidade ou valor atual de mercado.
- **Caixinhas:** o saldo informado e o percentual contratado são preservados. A taxa de referência identifica origem e data. Uma consulta só preenche o formulário; nada muda até salvar.
- **Projeções:** a renda recebida no mês e a renda estimada após reinvestir são grandezas distintas. Ao alterar as premissas, a tela indica que é preciso recalcular.
- **Mercado:** o app mostra o cache imediatamente e verifica atualizações em segundo plano ao abrir. A Visão geral usa os mesmos dados. A data de referência e o horário de coleta aparecem separados; uma consulta recente não transforma um informe mensal em dado do dia.
- **Assistente:** as conversas guardam o histórico entre provedores. Ferramentas, falhas e dados em cache têm rótulos legíveis.

## Verificação visual

`npm run verify:ui` sobe o Vite, simula o IPC do Tauri com dados fictícios e verifica páginas, formulários, chat, estados de erro e diferentes larguras/zoom. O relatório e as capturas ficam em `.local/ui-verification/`, fora do Git. Execute `npx playwright install chromium` uma vez no computador de desenvolvimento.

`npm run build` confere os tipos e gera a interface. O executável do Windows precisa ser recompilado depois do build web para incluir as telas atualizadas.

# Análise da carteira pelo assistente

Em **Configurações → Perfil de investimento**, registre tolerância a risco,
objetivos de curto e longo prazo, prazos em meses, necessidade de resgate e
situação da reserva de emergência. Aporte mensal, experiência, restrições e
observações são opcionais. É possível salvar um perfil parcial.

O mesmo formulário aparece em **Assistente → Completar perfil**. Cada carteira
tem seu próprio perfil, salvo localmente e incluído no seu backup financeiro.
As respostas não são inferidas pela carteira e não são publicadas no GitHub.

**Analisar carteira** envia uma mensagem explícita pela conversa e pelo provedor
selecionados. O assistente usa `preparar_analise_carteira` com
`refreshPublicDataIfDue: true`, consultando a mesma política de atualização
limitada usada na abertura do aplicativo. Abrir o chat ou salvar o perfil não
dispara uma resposta de IA.

A ferramenta retorna:

- Perfil declarado e perguntas sobre as informações que ainda faltam.
- FIIs, caixinhas e posição importada, com os escopos separados.
- Resultado calculado no Rust e comparação com o histórico diário do CDI.
- Cotações brapi, informes CVM, indicadores BCB e manchetes EBC disponíveis.
- Fontes, datas, referências salvas e falhas de consulta.

As instruções dos três conectores pedem que o modelo use esses dados antes de
sugerir melhorias. A resposta deve separar a situação observada, ações para o
curto prazo e alternativas para o longo prazo, considerando concentração,
liquidez, risco e objetivos. Campos ausentes e dados indisponíveis são tratados
como lacunas, sem inventar números ou prometer rentabilidade.

Quando o perfil está incompleto, o assistente pergunta o que ainda precisa
saber antes de personalizar sugestões. Respostas explícitas na conversa podem
ser usadas como premissas provisórias, identificadas na análise. Elas não
alteram o perfil salvo; use o formulário para atualizar o cadastro.

O modelo escolhido gera a análise. O MCP fornece dados e cálculos; não executa
ordens nem altera saldos. Trocar de provedor mantém a conversa canônica e o
histórico das ferramentas. Os dados enviados ao provedor escolhido incluem o
perfil e a carteira consultados para a análise.

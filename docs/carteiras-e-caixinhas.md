# Carteiras e caixinhas CDI

O aplicativo Windows permite manter carteiras separadas por instituição ou objetivo, por exemplo uma carteira de FIIs no Nubank e outra em uma corretora. Escolha a carteira ativa na barra lateral; os cadastros, lançamentos, caixinhas, importações e exportações usam essa seleção. Uma carteira nova começa vazia e não copia os investimentos da carteira principal.

O histórico anterior continua na **Carteira principal**. Seu banco permanece em `portfolio.sqlite`, dentro do diretório de dados exibido nas configurações. O catálogo fica em `wallets.sqlite`; cada carteira adicional tem um banco próprio em `wallets/<identificador>/portfolio.sqlite`. O mesmo ticker pode existir em carteiras diferentes, com quantidades, custos e rendimentos independentes. A seleção da interface fica lembrada neste computador.

## Registrar uma caixinha

Use o lápis ao lado do seletor para renomear uma carteira ou informar sua instituição, sem alterar os investimentos existentes. O assistente recebe a carteira selecionada em cada pergunta; a conversa permanece a mesma ao trocar de carteira ou provedor. As ferramentas MCP também podem consultar outra carteira por seu identificador explícito.

Em **Caixinhas CDI**, escolha **Nova caixinha** e informe nome, instituição, saldo atual em reais e percentual do CDI. Use vírgula ou ponto nos centavos, sem separador de milhar. O saldo é guardado com precisão decimal, com até duas casas decimais, e pode ser corrigido pela edição da caixinha.

O campo **Porcentagem do CDI** aceita, por exemplo, `120` para um produto remunerado a **120% do CDI**. A instituição e a porcentagem são dados cadastrados pelo usuário: o aplicativo não verifica uma oferta de Mercado Pago, Nubank ou outra instituição. As observações ajudam a registrar condições, limites de saldo e prazos do produto.

Para simular, preencha também **CDI anual (% a.a.)** e **Data dessa taxa**. Ambos são opcionais para guardar o saldo, mas devem ser preenchidos juntos para calcular o cenário. Você pode informar a referência manualmente ou usar **Buscar CDI no Banco Central**. A consulta preenche apenas o formulário; a referência passa ao cadastro quando você salva. A origem, URL e horário da coleta são preservados, e editar a taxa ou a data muda a origem para manual. O saldo e o percentual contratado permanecem como informados. A taxa cadastrada não recebe atualização automática. Veja [APIs públicas](apis-publicas.md).

## Entender a simulação

O cenário usa o saldo cadastrado, um prazo explícito entre 0 e 2.520 dias úteis e um CDI anual constante. A taxa diária equivalente é obtida na base de 252 dias úteis; a porcentagem do produto é aplicada sobre essa taxa diária:

```text
taxa_diaria_CDI = (1 + CDI_anual / 100)^(1 / 252) - 1
fator_diario_produto = 1 + taxa_diaria_CDI × percentual_CDI / 100
saldo_final = saldo_inicial × fator_diario_produto^dias_uteis
rendimento_bruto = saldo_final - saldo_inicial
```

Assim, `120% do CDI` usa 1,2 vez a taxa diária de referência. A [metodologia de acumulação do DI da B3](https://www.b3.com.br/pt_br/market-data-e-indices/indices/indices-de-segmentos-e-setoriais/di/metodologia-de-calcudo-acumulado-de-di/) descreve a aplicação do percentual aos fatores diários, e a [conversão exponencial da B3](https://www.b3.com.br/pt_br/produtos-e-servicos/negociacao/juros/s_di_train_prog/02-taxas-de-juros.htm) apresenta a base de 252 dias úteis.

Exemplo **hipotético de teste**: R$ 10.000,00, CDI anual constante de 10%, remuneração de 120% do CDI e 252 dias úteis resultam em saldo bruto simulado de **R$ 11.211,65**, com R$ 1.211,65 de rendimento bruto. A taxa de 10% é uma premissa desse exemplo, sem representar uma cotação atual.

Os valores são brutos, sem IR, IOF, tarifas, inflação, novos aportes ou resgates. O cenário não aplica automaticamente limites promocionais nem regras e arredondamentos próprios de cada instituição. Dias úteis são informados pelo usuário; o cálculo não consulta um calendário de feriados. A simulação nunca credita juros nem modifica o saldo cadastrado.

## Conferir os valores sem contar duas vezes

| Informação | O que representa |
| --- | --- |
| Custo registrado dos FIIs | Aquisições lançadas no histórico; não é uma cotação de mercado. |
| Caixinhas — saldo informado | Saldo atual cadastrado manualmente em cada reserva. |
| Posição importada da corretora | Retrato datado, com os valores e horários indicados na fonte. |
| Proventos recebidos no histórico | Acumulado dos lançamentos; não é o saldo disponível da conta. |

A visão por instituição apresenta os custos dos FIIs e os saldos das caixinhas em colunas separadas. A posição importada da corretora continua independente e não é adicionada novamente a esses totais. Se uma mesma reserva também aparecer em uma posição importada, ela continua sendo uma observação da fonte; o cadastro da caixinha não altera nem substitui essa posição.

## Exportar e restaurar

**Exportar carteira completa** salva somente a **carteira selecionada**, incluindo histórico de FIIs, caixinhas, posição importada quando houver e identificação da carteira. Faça uma exportação para cada carteira que deseja guardar; essa exportação não é um backup global do catálogo, das outras carteiras ou das conversas com a IA.

Para restaurar, crie ou selecione uma carteira vazia e use **Importar histórico ou backup**. O nome e a instituição da carteira de destino continuam definidos no catálogo. A importação valida os dados antes de gravar e preserva os identificadores das caixinhas. O mesmo arquivo é reconhecido para evitar duplicação; um arquivo diferente exige uma carteira vazia, evitando uma mesclagem silenciosa de saldos e lançamentos.

Os JSON de origem são somente lidos e recebem uma cópia de segurança exata antes da importação. A exportação também recusa sobrescrever arquivos existentes. Uma cópia integral do diretório de dados, feita com o aplicativo encerrado, preserva o catálogo e os bancos de todas as carteiras, além dos demais dados locais.

As informações continuam neste computador. A sincronização com a Vercel e o acesso aos mesmos dados pela web ainda dependem da próxima etapa do projeto.

## Verificação técnica

Os testes determinísticos cobrem independência entre carteiras, backups, importação, validações, precisão decimal e erros de armazenamento. A integração real com Claude foi validada com carteira e caixinha sintéticas: o modelo consultou `carteira_snapshot` na carteira selecionada e encadeou `projetar_renda`, recebendo o resultado hipotético do exemplo acima. A carteira pessoal não foi enviada ao provedor.

O teste optativo `cargo run -p ivestfii-runtime --example verify_cli -- claude --wallet-smoke` reproduz esse fluxo com dados descartáveis e usa a assinatura da CLI. Ele não altera o banco principal.

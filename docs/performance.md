# Resultado da carteira e comparação dos FIIs com o CDI

A visão geral reúne os FIIs e as caixinhas cadastradas na carteira selecionada.
Os valores importados da corretora continuam em seu quadro próprio: o seu total,
saldo em conta e pós-fixado não são somados de novo aos registros, evitando
contagem duplicada. O gráfico de comparação com o CDI cobre explicitamente os
FIIs, cujas compras mensais estão registradas.

## Caixinhas e totais consolidados

O saldo atual sozinho não identifica o capital aplicado ou o lucro. Cada
caixinha pode guardar um conjunto opcional de três informações, referentes ao
mesmo período desde o início: total aplicado, total resgatado e data efetiva do
saldo. Resgates incluem tanto capital quanto juros retirados.

`rendimento_caixinha = saldo_atual + resgates_acumulados − aplicações_acumuladas`

Isso mantém o rendimento correto inclusive com resgate parcial, resgate total
ou retirada dos juros. O resultado segue o saldo informado pelo usuário; não
pressupõe saldo bruto, calcula imposto ou credita os juros projetados.

- **Total aplicado:** compras de FIIs + aplicações acumuladas nas caixinhas.
  É um valor histórico bruto de aplicações, que pode incluir reaplicações; os
  resgates são apresentados separadamente. Não é o principal remanescente.
- **Rendimentos acumulados:** proventos registrados dos FIIs + resultado das
  caixinhas calculado pela fórmula acima.
- **Resultado da carteira:** resultado completo dos FIIs + resultado das
  caixinhas. A variação do preço das cotas entra uma única vez.

Sem os três dados de alguma caixinha, os totais que dependem deles ficam
indisponíveis; subtotais conhecidos são identificados como parciais. Ausência
não equivale a zero. O saldo continua disponível mesmo sem o histórico de
aplicações. Zero informado é um valor conhecido, e resultado negativo é válido.

Cada caixinha mantém sua data do saldo, enquanto os FIIs usam a data da cotação.
O consolidado reúne os últimos valores registrados de cada componente; não
afirma representar uma única data histórica. A data da taxa CDI, a data de
cadastro e o horário de edição não substituem a data efetiva do saldo.

Na migração SQLite 4→5, as novas colunas recebem NULL. Saldos, compras,
proventos, identificadores, revisões e fila de alterações existentes são
preservados. Clientes que omitem os novos campos não apagam dados já informados;
a limpeza exige `clearReconciliation=true`. Exportação e importação carregam
os três campos e preservam os arquivos de origem.

Sem as datas de cada aplicação e resgate, não há um cenário CDI equivalente
para as caixinhas. A sua taxa contratada continua servindo à simulação separada;
nem a simulação nem o CDI atual são tratados como rendimento realizado.

## Cálculo dos FIIs

- **Total investido:** soma de quantidade comprada × preço da compra.
- **Variação das cotas:** valor de mercado das cotas − custo das compras.
- **Resultado total:** variação das cotas + proventos do histórico.
- **Percentual acumulado:** resultado ÷ custo das compras × 100. Não é uma taxa
  anualizada, TWR, XIRR ou medida ajustada ao tempo dos fluxos.

Os proventos seguem o núcleo existente: o total explicitamente informado,
inclusive zero, prevalece. Na ausência de total, o sistema usa as cotas ao fim
do mês × provento por cota, com o mesmo arredondamento do histórico. Notas e
registros originais nunca são alterados para produzir uma comparação.

O gráfico compara dois resultados acumulados em reais, sem criar preços
históricos intermediários. Todas as cotações precisam ter a mesma data de
mercado, e o CDI precisa chegar à mesma data. Cotação ausente, custo incompleto,
quantidades incompatíveis ou cobertura insuficiente produzem uma indicação de
dados incompletos; ausência de informação não vira zero.

## Hipótese mensal para o CDI

O histórico guarda o mês, mas não o dia da compra. O comparativo trata cada
compra como uma aplicação independente feita no último dia do mês; para compras
no mês da cotação, utiliza a data da cotação, sem atribuir juros anteriores à
compra. Para cada compra C na data estimada d e data final D:

`saldo_CDI(C) = C × produto(1 + taxa_dia / 100), para d < dia <= D`

`resultado_CDI = soma(saldo_CDI) − soma(C)`

As taxas são as observações diárias históricas da série **SGS 12**, em **% ao
dia**, publicadas pelo Banco Central. Não se aplica retroativamente a taxa
anualizada atual. Fins de semana e feriados não recebem taxas inventadas.

É uma comparação estimada, antes de impostos e taxas, dos mesmos valores e
meses de compras. O histórico não informa se uma compra veio de dinheiro novo
ou de reinvestimento. Os proventos entram no resultado dos FIIs sem rendimento
adicional neste modelo; o CDI capitaliza diariamente. A interface explica as
hipóteses junto à comparação e nas ajudas dos respectivos valores.

## Fontes e armazenamento

- [Banco Central: metadados da série SGS 12](https://www3.bcb.gov.br/sgspub/consultarvalores/consultarValoresSeries.do?hdOidSeriesSelecionadas=12&method=consultarGraficoPorId).
- [API pública SGS 12](https://api.bcb.gov.br/dados/serie/bcdata.sgs.12/dados?formato=json).
- [brapi: cotação atual](https://brapi.dev/docs/acoes/cotacao).

Abrir a visão geral apenas lê os caches locais. O botão de atualização consulta
a brapi e o Banco Central, com limites de tempo e tamanho. Somente os códigos
dos ativos e o intervalo de datas saem do computador. A chave brapi permanece no
cofre do sistema operacional. O cache público do CDI fica separado do SQLite
financeiro; falhas de rede preservam os dados salvos.

## FIIs Tracker (CustomTkinter)

Ferramenta em Python para acompanhar aportes e rendimentos dos seus FIIs com uma interface totalmente grafica, sem menus no terminal. Os dados ficam no arquivo `data/fiis_data.json`, que voce pode versionar ou copiar como backup.

### Como executar

```bash
pip install -r requirements.txt
python main.py
```

Apos rodar `python main.py`, a janela do aplicativo e aberta e todas as acoes acontecem por ali.

### O que da para fazer na interface

- **Dashboard**: cards para todos os FIIs mostrando cotas, investido, yield e ultimo rendimento, alem de resumo com dia/hora atuais, ultima atualizacao do arquivo, renda mensal, DY medio e total de dividendos ja recebidos.
- **Gerenciar**: tabela completa com painel de detalhes e botoes para adicionar FIIs, registrar meses, abrir historico, editar lancamentos antigos e acessar a projecao rapida daquele ativo.
- **Projecoes**: aba dedicada com projecao individual (considerando compras mensais e exibindo renda acumulada) e projecao consolidada de toda a carteira, somando o rendimento das novas cotas planejadas para cada fundo.
- **Backups**: os dados sao persistidos automaticamente em `data/fiis_data.json`; copie esse arquivo sempre que quiser salvar uma versao.

### Estrutura basica do JSON

```json
{
  "fiis": [
    {
      "ticker": "KNRI11",
      "name": "Kinea Renda",
      "sector": "Hibrido",
      "entries": [
        {
          "month": "2024-03",
          "cotas_added": 1,
          "price_per_cota": 150.0,
          "dividend_per_cota": 0.85,
          "dividend_total": 12.75,
          "notes": "Reinvestindo proventos"
        }
      ]
    }
  ]
}
```

Voce pode editar o arquivo manualmente (com o app fechado) se precisar importar dados de outro lugar.

### Executavel (.exe)

Criamos um build com PyInstaller (`dist/FIIsTracker.exe`). Para recriar:

```bash
pyinstaller --clean --onefile --windowed --name FIIsTracker --add-data "data;data" main.py
```

Leve toda a pasta `dist/` para distribuir o app sem depender do Python instalado.

### Deploy?

O app e local, feito com CustomTkinter. Para ter a mesma experiencia na web seria preciso construir um frontend separado e uma API/banco compativel. Use esta versao para controle pessoal direto no computador.

### API Flask (prototipo)

Uma pasta `backend/` contem um servidor Flask + SQLAlchemy para centralizar dados e importar o `data/fiis_data.json`. Para rodar localmente:

```bash
cd backend
python -m venv .venv
.venv\Scripts\activate
pip install -r requirements.txt
set FLASK_APP=app.py
flask db upgrade   # cria o banco SQLite
set IMPORT_TOKEN=devtoken
flask run
```

Depois envie o JSON atual:

```bash
curl -X POST http://127.0.0.1:5000/api/import ^
  -H "Content-Type: application/json" ^
  -H "X-Import-Token: devtoken" ^
  --data "@..\data\fiis_data.json"
```

No deploy (ex.: PythonAnywhere), ajuste `DATABASE_URL`/`IMPORT_TOKEN` e reaproveite os mesmos comandos. A partir dai o app desktop pode consumir os dados via API em vez do JSON local.

### Git push rapido

```bash
git init
git remote add origin https://github.com/<usuario>/<repositorio>.git
git add .
git commit -m "feat: enviar FIIs Tracker para o GitHub"
git branch -M main
git push -u origin main
```

Lembre de criar o repositorio no GitHub antes de rodar o `push`.


# Continuar em outro computador

O GitHub contém o código Rust/Tauri, interface, testes, exemplos fictícios e
instruções. Bancos, carteiras, conversas, chaves e instaladores locais ficam fora
dos commits. O projeto principal é o aplicativo Windows; a sincronização de
dados com a Vercel ainda não está implementada.

## Testar o aplicativo no Windows

Baixe o `IvestFII-Setup.exe` na [release mais recente](https://github.com/Vittuu/ivestfii/releases/latest).
O instalador configura o aplicativo sem exigir Rust, Node.js ou Python.

Uma instalação nova começa sem sua carteira. No computador atual, exporte um
backup pelo aplicativo e transfira o JSON por um meio privado. No outro
computador, crie ou selecione a carteira de destino e importe o backup. Cada
carteira tem seu histórico, caixinhas e perfil. Confira a carteira selecionada
antes de importar.

Para levar também todas as conversas e carteiras, use **Sair** no aplicativo
antes de copiar a pasta `%LOCALAPPDATA%\com.ivestfii.desktop` para um backup
privado. No outro computador, feche o aplicativo, preserve uma cópia da pasta já
existente e restaure esse backup. Copiar bancos SQLite enquanto o aplicativo
está aberto pode deixar a cópia incompleta.

Cadastre novamente a chave brapi nas configurações e autentique as CLIs dos
provedores que pretende usar. O cofre de credenciais e os logins não são
transferidos pelo Git nem pelo backup financeiro. O perfil cadastrado é uma
declaração do usuário; a análise pede as informações que ainda faltarem.

## Desenvolver no Windows

Use Node.js 24, Rust estável (o CI usa 1.96.0), Git, ferramentas C++ de compilação
do Visual Studio e WebView2. Python 3.12 ou superior só é necessário para os
scripts de conferência de armazenamento e MCP.

```powershell
git clone https://github.com/Vittuu/ivestfii.git
cd ivestfii
npm ci
cargo fetch --locked
npm run tauri dev
```

O comando abre a versão de desenvolvimento. Para usar uma base isolada de teste,
defina o diretório antes de iniciar:

```powershell
$env:IVESTFII_DATA_DIR = Join-Path $PWD '.local/dev-data'
npm run tauri dev
```

Para abrir somente a interface no navegador:

```powershell
npm run dev
```

Essa prévia não tem acesso aos comandos nativos, à carteira local ou às CLIs.
O fluxo completo do assistente deve ser testado no aplicativo Tauri.

## Conferir antes de continuar

```powershell
cargo test --workspace --locked
npm run build
npm run verify:ledger
npm run verify:income
npx playwright install chromium
npm run verify:ui
npm run tauri -- build --ci --no-bundle -- --locked
python -B scripts/verify-migration.py --binary target/release/IvestFII.exe
python -B scripts/verify-mcp.py --desktop --binary target/release/IvestFII.exe
```

A migração usa `examples/portfolio.example.json` por padrão. Para conferir um
backup privado, passe `--source C:\CAMINHO\backup.json` ao primeiro script.
Os scripts usam bancos isolados em `.local/` e não acionam provedores de IA.
Testes de interface usam somente dados fictícios e simulam o IPC.

O workflow **Validar aplicativo** repete essas verificações a cada push em
`main` e em pull requests, e guarda um executável Windows de teste como artefato.
Esse executável não é o pacote assinado do atualizador.

## Publicar outra versão

O [guia de distribuição](distribuicao-windows.md) explica o instalador único e
a assinatura. A chave privada do atualizador não entra no Git. Outro computador
pode desenvolver e gerar builds de teste sem ela; para publicar atualizações
compatíveis, é necessário usar a chave original por um meio seguro.

O workflow **Distribuicao Windows** é iniciado manualmente e exige os secrets
`TAURI_SIGNING_PRIVATE_KEY` e `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` no GitHub
Actions. Esses secrets não foram enviados junto com o código. Também é possível
gerar o pacote assinado no computador que já possui a chave e anexar os artefatos
à release. Não substitua uma versão que já foi publicada.

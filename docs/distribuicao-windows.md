# Distribuicao e atualizacoes do Windows

O arquivo para instalar o aplicativo e **`dist/IvestFII-Setup.exe`**. O instalador NSIS contem o aplicativo e a interface web compilada. Nao e necessario distribuir a pasta `target`, Node.js, Rust, Python ou os executaveis de depuracao. O WebView2 e detectado durante a instalacao e, se faltar, o instalador baixa o runtime da Microsoft; essa primeira instalacao requer internet.

A instalacao e por usuario (`currentUser`), sem exigir administrador para instalar o aplicativo. O identificador `com.ivestfii.desktop` permanece estavel para continuar usando a carteira, o historico e as configuracoes em `%LOCALAPPDATA%\com.ivestfii.desktop`. Atualizar o aplicativo substitui seus arquivos, sem incluir ou apagar esse banco de dados. O desinstalador padrao do Tauri preserva os dados, salvo se o usuario marcar explicitamente a opcao de apagar dados; em modo de atualizacao essa remocao e bloqueada pelo proprio template NSIS.

## Gerar o instalador local

Requisitos: Windows x64, PowerShell 7, Node.js 24.16.0, Rust 1.96.0 MSVC e Microsoft C++ Build Tools. As dependencias JavaScript devem estar instaladas com `npm ci`. O build usa o `package-lock.json` e `Cargo.lock` existentes e falha se as versoes do npm, do Tauri e do workspace Rust divergirem.

A chave privada de atualizacao fica fora do repositorio, por padrao em `%LOCALAPPDATA%\IvestFII\release-keys\updater.key`, em uma pasta com acesso restrito ao usuario. Guarde uma copia segura: perder a chave impede que versoes ja instaladas aceitem futuras atualizacoes assinadas com outra chave. Nunca inclua a chave em um commit, no executavel, em arquivos de suporte ou em mensagens. A chave publica correspondente fica em `src-tauri/tauri.conf.json`.

```powershell
./scripts/build-windows-release.ps1 -ValidateOnly
./scripts/build-windows-release.ps1
```

Para uma chave em outro local, use `-SigningKeyPath 'C:\local-protegido\updater.key'`. Se a chave tiver senha, forneca-a somente pela variavel de ambiente `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` no processo que executa o build. O script tambem aceita a chave fornecida por `TAURI_SIGNING_PRIVATE_KEY`, usado pelo CI, sem registrar seu conteudo.

O script produz um unico instalador com nome fixo e os metadados tecnicos em `dist/release/`:

- `IvestFII-Setup.exe.sig`: assinatura do instalador gerada pelo Tauri.
- `latest.json`: versao, data, assinatura e URL da atualizacao para `windows-x86_64`.
- `SHA256SUMS.txt`: checksum para verificacao manual.

Os arquivos `.sig` e `latest.json` sao necessarios ao servico de atualizacao. O usuario baixa somente o `.exe`. Renomear o instalador nao altera sua assinatura, pois os bytes permanecem iguais. O script nao publica releases, nao instala o aplicativo e nao copia dados da carteira ou credenciais de APIs para a distribuicao.

Antes de concluir, o build executa uma verificacao offline usando a chave publica incorporada, o mesmo formato Minisign do atualizador e o manifesto gerado. Essa verificacao tambem comprova que alterar um byte do instalador ou da assinatura causa rejeicao; as alteracoes acontecem somente em memoria. Para repetir a verificacao sem gerar outro build nem executar o instalador:

```powershell
cargo run -p ivestfii-desktop --example verify-installer --locked --offline -- dist/IvestFII-Setup.exe dist/release/IvestFII-Setup.exe.sig dist/release/latest.json
```

## Canal de atualizacao

O aplicativo consulta `https://github.com/Vittuu/ivestfii/releases/latest/download/latest.json`. Cada manifesto aponta para `https://github.com/Vittuu/ivestfii/releases/download/vVERSAO/IvestFII-Setup.exe`. O Tauri confere a assinatura com a chave publica incorporada antes de instalar. O endpoint usa HTTPS e o instalador bloqueia downgrade de versao.

Uma release em rascunho nao fica acessivel pelo endpoint publico. A primeira publicacao e as seguintes precisam estar publicadas para que os aplicativos recebam atualizacoes. Nao substitua os arquivos de uma versao publicada; aumente a versao e gere uma nova release.

## Pipeline do GitHub

O workflow `.github/workflows/release-windows.yml` executa em Windows, usa `npm ci` e Cargo com `--locked`, roda os testes Rust, gera o NSIS assinado e prepara uma **release em rascunho**. E acionado manualmente, informando uma tag ja existente; assim a validacao comum do projeto nao exige acesso a chave de assinatura. A versao na tag deve corresponder ao codigo.

Antes de usar o workflow, cadastre os seguintes secrets em GitHub Settings > Secrets and variables > Actions:

- `TAURI_SIGNING_PRIVATE_KEY`: conteudo da mesma chave privada usada para a versao instalada.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: senha correspondente; pode ficar ausente para chave sem senha.

A chave deve ser inserida diretamente na tela de secrets ou pelo stdin de uma ferramenta segura; nunca em argumento de comando que possa ser salvo no historico. O workflow nao configura esses secrets nem publica o rascunho automaticamente. Depois da revisao dos artefatos, publique a release no GitHub para disponibilizar a atualizacao. O pipeline recusa substituir uma release ja publicada.

## Assinaturas do instalador

A assinatura Tauri autentica atualizacoes e e obrigatoria neste fluxo. Ela e diferente de um certificado Windows Authenticode: nao remove, por si so, o aviso de editor desconhecido ou a reputacao do SmartScreen. O fluxo atual nao exige comprar um certificado Windows.

Referencias oficiais: [atualizador Tauri](https://v2.tauri.app/plugin/updater/), [instalador Windows](https://v2.tauri.app/distribute/windows-installer/), [template NSIS e preservacao de dados](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi).

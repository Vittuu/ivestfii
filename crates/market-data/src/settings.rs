//! Non-financial provider configuration. Secrets never enter JSON, MCP, or sync.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDataSettings {
    pub brapi_key_configured: bool,
    pub brapi_key_source: Option<String>,
    pub cvm_mappings: BTreeMap<String, String>,
    pub cvm_mapping_sources: BTreeMap<String, String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MappingFile {
    version: u32,
    // A null override deliberately disables a verified built-in mapping.
    cvm_mappings: BTreeMap<String, Option<String>>,
}

#[derive(Clone)]
pub struct MarketSettingsStore {
    directory: PathBuf,
}

// Public fund identities verified in structured Fundos.NET disclosures. These
// are not inferred from names or from a user's balances. Other funds require an
// explicit CNPJ mapping; an override can also disable any built-in mapping.
fn verified_mappings() -> [(&'static str, &'static str, &'static str); 4] {
    [
        ("MXRF11", "97521225000125", "https://fnet.bmfbovespa.com.br/fnet/publico/exibirDocumento?cod=0&cod_neg=MXRF11&id=1272055&tipo_comunicado=rendimentos_e_amortizacoes_aviso_aos_cotistas_-_estruturado"),
        ("VGHF11", "36771692000119", "https://fnet.bmfbovespa.com.br/fnet/publico/exibirDocumento?cvm=true&id=1272014"),
        ("XPML11", "28757546000100", "https://fnet.bmfbovespa.com.br/fnet/publico/exibirDocumento?cvm=true&id=1296177"),
        ("VGIA11", "41081088000109", "https://fnet.bmfbovespa.com.br/fnet/publico/exibirDocumento?cvm=true&id=1285939"),
    ]
}

impl MarketSettingsStore {
    pub fn new(directory: impl AsRef<Path>) -> Result<Self, String> {
        let directory = directory.as_ref();
        if !directory.is_absolute() {
            return Err("O diretório de configuração precisa ser absoluto.".into());
        }
        Ok(Self {
            directory: directory.to_owned(),
        })
    }

    fn path(&self) -> PathBuf {
        self.directory.join("public-data/market-settings.json")
    }

    fn mappings_file(&self) -> Result<MappingFile, String> {
        let path = self.path();
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(MappingFile {
                    version: 1,
                    ..Default::default()
                })
            }
            Err(_) => return Err("Não foi possível ler as configurações das fontes.".into()),
        };
        if metadata.len() > 64 * 1024 {
            return Err("As configurações das fontes excedem o limite de tamanho.".into());
        }
        let parsed: MappingFile = serde_json::from_slice(
            &fs::read(path).map_err(|_| "Não foi possível ler as configurações das fontes.")?,
        )
        .map_err(|_| "As configurações das fontes são inválidas; arquivo preservado.")?;
        if parsed.version != 1 || parsed.cvm_mappings.len() > 300 {
            return Err(
                "Versão ou quantidade de vínculos CVM inválida; arquivo preservado.".into(),
            );
        }
        for (symbol, cnpj) in &parsed.cvm_mappings {
            if normalize_ticker(symbol)? != *symbol {
                return Err("Ticker inválido nas configurações CVM.".into());
            }
            if let Some(cnpj) = cnpj {
                crate::cvm::normalize_cnpj(cnpj)?;
            }
        }
        Ok(parsed)
    }

    pub fn mappings(&self) -> Result<(BTreeMap<String, String>, BTreeMap<String, String>), String> {
        let mut mappings = BTreeMap::new();
        let mut sources = BTreeMap::new();
        for (symbol, cnpj, source) in verified_mappings() {
            mappings.insert(symbol.into(), cnpj.into());
            sources.insert(symbol.into(), source.into());
        }
        for (symbol, cnpj) in self.mappings_file()?.cvm_mappings {
            sources.remove(&symbol);
            if let Some(cnpj) = cnpj {
                mappings.insert(symbol, cnpj);
            } else {
                mappings.remove(&symbol);
            }
        }
        Ok((mappings, sources))
    }

    pub fn read(&self) -> Result<MarketDataSettings, String> {
        let (cvm_mappings, cvm_mapping_sources) = self.mappings()?;
        let (key, source) = self.brapi_key()?;
        Ok(MarketDataSettings {
            brapi_key_configured: key.is_some(),
            brapi_key_source: source,
            cvm_mappings,
            cvm_mapping_sources,
        })
    }

    /// Only provider transport callers may use this result. It must never be
    /// serialized, logged, returned through IPC, or passed to a model.
    pub fn brapi_key(&self) -> Result<(Option<String>, Option<String>), String> {
        if let Some(key) = self.local_key()? {
            return Ok((Some(key), Some("local".into())));
        }
        for name in ["BRAPI_API_KEY", "BRAPI_TOKEN"] {
            if let Ok(key) = std::env::var(name) {
                if !key.trim().is_empty() {
                    return Ok((Some(validate_key(&key)?), Some("environment".into())));
                }
            }
        }
        Ok((None, None))
    }

    pub fn save_brapi_key(&self, key: &str) -> Result<MarketDataSettings, String> {
        let key = validate_key(key)?;
        #[cfg(windows)]
        self.entry()?.set_password(&key).map_err(|_| {
            "Não foi possível salvar a chave no Gerenciador de Credenciais do Windows."
        })?;
        #[cfg(not(windows))]
        {
            let _ = key;
            return Err("O armazenamento de chave está disponível no Windows; use BRAPI_API_KEY neste sistema.".into());
        }
        #[cfg(windows)]
        self.read()
    }

    pub fn clear_brapi_key(&self) -> Result<MarketDataSettings, String> {
        #[cfg(windows)]
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(_) => {
                return Err(
                    "Não foi possível remover a chave do Gerenciador de Credenciais do Windows."
                        .into(),
                )
            }
        }
        self.read()
    }

    #[cfg(windows)]
    fn entry(&self) -> Result<keyring::Entry, String> {
        use sha2::{Digest, Sha256};
        let canonical =
            fs::canonicalize(&self.directory).map_err(|_| "Diretório das fontes indisponível.")?;
        let identity = canonical
            .to_string_lossy()
            .replace('/', "\\")
            .to_lowercase();
        let account = format!("brapi-{:x}", Sha256::digest(identity.as_bytes()));
        keyring::Entry::new("com.ivestfii.market-data", &account)
            .map_err(|_| "Não foi possível acessar o Gerenciador de Credenciais do Windows.".into())
    }

    fn local_key(&self) -> Result<Option<String>, String> {
        #[cfg(windows)]
        match self.entry()?.get_password() {
            Ok(key) => Ok(Some(validate_key(&key)?)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => {
                Err("Não foi possível ler a chave do Gerenciador de Credenciais do Windows.".into())
            }
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }

    pub fn save_cvm_mapping(
        &self,
        symbol: &str,
        cnpj: Option<&str>,
    ) -> Result<MarketDataSettings, String> {
        let symbol = normalize_ticker(symbol)?;
        let cnpj = cnpj.map(crate::cvm::normalize_cnpj).transpose()?;
        let path = self.path();
        let parent = path.parent().ok_or("Diretório de configuração inválido.")?;
        fs::create_dir_all(parent)
            .map_err(|_| "Não foi possível preparar as configurações CVM.")?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(parent.join("market-settings.lock"))
            .map_err(|_| "Não foi possível bloquear as configurações CVM.")?;
        lock.lock()
            .map_err(|_| "Não foi possível bloquear as configurações CVM.")?;
        let mut data = self.mappings_file()?;
        if !data.cvm_mappings.contains_key(&symbol) && data.cvm_mappings.len() >= 300 {
            return Err("Limite de vínculos CVM atingido.".into());
        }
        data.cvm_mappings.insert(symbol, cnpj);
        let bytes = serde_json::to_vec_pretty(&data)
            .map_err(|_| "Não foi possível preparar as configurações CVM.")?;
        let temp = parent.join(format!(".market-settings-{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(|_| "Não foi possível salvar as configurações CVM.")?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| "Não foi possível salvar as configurações CVM.")?;
            drop(file);
            fs::rename(&temp, &path)
                .map_err(|_| "Não foi possível substituir as configurações CVM.")?;
            Ok::<(), String>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result?;
        drop(lock);
        self.read()
    }
}

fn validate_key(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty() || value.len() > 4096 || !value.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("Informe uma chave brapi válida, sem espaços ou quebras de linha.".into());
    }
    Ok(value.to_owned())
}

pub fn normalize_ticker(raw: &str) -> Result<String, String> {
    let symbol = raw.trim().to_ascii_uppercase();
    if !(4..=12).contains(&symbol.len())
        || !symbol.bytes().all(|c| c.is_ascii_alphanumeric())
        || !symbol.bytes().any(|c| c.is_ascii_digit())
        || !symbol.bytes().any(|c| c.is_ascii_alphabetic())
    {
        return Err("Informe um ticker válido.".into());
    }
    Ok(symbol)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verified_fund_ids_have_provenance_and_can_be_overridden_or_removed() {
        let dir = tempfile::tempdir().unwrap();
        let store = MarketSettingsStore::new(dir.path()).unwrap();
        let (mappings, sources) = store.mappings().unwrap();
        assert_eq!(mappings["MXRF11"], "97521225000125");
        assert!(sources["MXRF11"].starts_with("https://fnet.bmfbovespa.com.br/"));
        // Write the public config directly: tests must never write OS secrets.
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(
            store.path(),
            r#"{"version":1,"cvmMappings":{"MXRF11":null,"TEST11":"28757546000100"}}"#,
        )
        .unwrap();
        let (mapped, sources) = store.mappings().unwrap();
        assert!(!mapped.contains_key("MXRF11"));
        assert_eq!(mapped["TEST11"], "28757546000100");
        assert!(!sources.contains_key("TEST11"));
    }
    #[test]
    fn malformed_config_is_preserved_and_secret_validation_does_not_echo_input() {
        let dir = tempfile::tempdir().unwrap();
        let store = MarketSettingsStore::new(dir.path()).unwrap();
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(store.path(), b"not-json").unwrap();
        assert!(store.mappings().is_err());
        assert_eq!(fs::read(store.path()).unwrap(), b"not-json");
        let error = validate_key("SECRET\nOTHER").unwrap_err();
        assert!(!error.contains("SECRET"));
        assert!(normalize_ticker("MXRF11&token=secret").is_err());
    }
}

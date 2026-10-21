//! Free, dated distribution filings from the public Fundos.NET document index.
//! Only public fund identities and calendar ranges are sent. This archive never
//! writes transactions, balances, or a presumed payment for an absent filing.

use crate::{
    dividends::{DividendEvent, DividendItem, DividendObservation},
    fund_income_document::parse_fund_income_document,
    quotes::normalize_symbol,
    refresh_policy::{fresh, RefreshGate, RefreshOutcome},
};
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, SecondsFormat, Utc};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue, COOKIE, SET_COOKIE},
    redirect::Policy,
    Client,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

pub const HISTORICAL_DIVIDEND_SOURCE: &str = "Fundos.NET · Avisos de rendimentos";
const BASE: &str = "https://fnet.bmfbovespa.com.br/fnet/publico/";
const INDEX: &str = "https://fnet.bmfbovespa.com.br/fnet/publico/pesquisarGerenciadorDocumentosCVM?paginaCertificados=false&tipoFundo=1";
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_CACHE: usize = 32 * 1024 * 1024;
const MAX_FUNDS: usize = 100;
const MAX_DOCUMENTS: usize = 2000;
const PAGE_SIZE: usize = 50;
const MAX_BATCH: usize = 24;
const DAILY_LIMIT: u32 = 240;
const TTL: i64 = 24 * 3600;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalDividendRequest {
    pub symbol: String,
    pub cnpj: String,
    #[serde(default)]
    pub isin: Option<String>,
    pub start_date: String,
    pub end_date: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalDividendPanel {
    pub items: Vec<DividendItem>,
    pub coverage: Vec<HistoricalDividendCoverage>,
    pub issues: Vec<String>,
    pub pending_documents: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalDividendCoverage {
    pub symbol: String,
    pub start_date: String,
    pub end_date: String,
    pub listing_complete: bool,
    pub documents_total: usize,
    pub documents_pending: usize,
    pub complete: bool,
    pub fetched_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    id: u64,
    version: u32,
    reference_date: String,
    delivered_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SavedDocument {
    version: u32,
    fetched_at: String,
    events: Vec<DividendEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Listing {
    from_date: String,
    through_date: String,
    checked_at: String,
    complete: bool,
    next_start: usize,
    total: usize,
    documents: Vec<Document>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fund {
    request: HistoricalDividendRequest,
    listing: Option<Listing>,
    #[serde(default)]
    previous_listing: Option<Listing>,
    documents: BTreeMap<u64, SavedDocument>,
    #[serde(default)]
    errors: BTreeMap<u64, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cache {
    version: u32,
    funds: BTreeMap<String, Fund>,
    /// Count every attempt, including failures, before HTTP. A calendar rollover
    /// cannot double the local budget during the same rolling 24 hours.
    attempts: Vec<String>,
    retry_after: Option<String>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            version: 1,
            funds: BTreeMap::new(),
            attempts: vec![],
            retry_after: None,
        }
    }
}

#[derive(Clone)]
pub struct HistoricalDividendService {
    client: Client,
    cache_path: PathBuf,
    session: Arc<tokio::sync::Mutex<Option<Session>>>,
}

impl HistoricalDividendService {
    pub fn new(cache_path: PathBuf) -> Result<Self, String> {
        if cache_path.file_name().is_none() {
            return Err("Informe um arquivo para o histórico de proventos.".into());
        }
        let client = Client::builder()
            .https_only(true)
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(18))
            .user_agent("IvestFII/0.1 (public Fundos.NET historical distribution filings)")
            .build()
            .map_err(|_| "Não foi possível preparar o histórico de proventos.".to_string())?;
        Ok(Self {
            client,
            cache_path,
            session: Arc::new(tokio::sync::Mutex::new(None)),
        })
    }

    /// Pure local read. A missing archive does not create files or reservations.
    pub fn read_cache(
        &self,
        requests: &[HistoricalDividendRequest],
    ) -> Result<HistoricalDividendPanel, String> {
        let requests = normalize_requests(requests, Utc::now())?;
        Ok(panel(&read_file(&self.cache_path)?, &requests, vec![]))
    }

    pub async fn refresh_if_due(
        &self,
        requests: &[HistoricalDividendRequest],
    ) -> RefreshOutcome<HistoricalDividendPanel> {
        let before = self.read_cache(requests).ok();
        match self.refresh(requests).await {
            Ok(data) => {
                let changed = before.as_ref().is_none_or(|previous| {
                    previous.items != data.items || previous.coverage != data.coverage
                });
                let message = (!data.issues.is_empty()).then(|| data.issues.join(" "));
                let mut outcome = if changed {
                    RefreshOutcome::updated(data)
                } else {
                    RefreshOutcome::cached(data)
                };
                outcome.message = message;
                outcome
            }
            Err(error) => RefreshOutcome::failed(self.read_cache(requests).ok(), error),
        }
    }

    pub async fn refresh(
        &self,
        requests: &[HistoricalDividendRequest],
    ) -> Result<HistoricalDividendPanel, String> {
        let requests = normalize_requests(requests, Utc::now())?;
        match tokio::time::timeout(Duration::from_secs(180), self.refresh_inner(&requests)).await {
            Ok(result) => result,
            Err(_) => {
                let mut result = self.read_cache(&requests)?;
                result.issues.push("A consulta histórica atingiu 180 segundos. O progresso salvo foi preservado para continuar depois.".into());
                Ok(result)
            }
        }
    }

    async fn refresh_inner(
        &self,
        requests: &[HistoricalDividendRequest],
    ) -> Result<HistoricalDividendPanel, String> {
        let mut gate = RefreshGate::acquire(&self.cache_path).await?;
        let mut cache = read_file(&self.cache_path)?;
        // Ordinary public cookies/CSRF exist only in memory, shared by bounded
        // continuation batches. Reopening the index for every 24 documents
        // needlessly consumes requests and creates another failure point.
        let mut session = self.session.lock().await;
        if session.as_ref().is_some_and(|session| {
            session
                .created_at
                .is_none_or(|at| Utc::now() - at > ChronoDuration::minutes(20))
        }) {
            *session = None;
        }
        let mut calls = 0;
        let mut issues = vec![];
        if let Some(until) = cache
            .retry_after
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        {
            if until > Utc::now() {
                return Ok(panel(
                    &cache,
                    requests,
                    vec![format!(
                        "Fundos.NET: nova tentativa após {}.",
                        until.to_rfc3339()
                    )],
                ));
            }
        }
        for request in requests {
            if calls >= MAX_BATCH {
                break;
            }
            let key = identity(request);
            if !cache.funds.contains_key(&key) && cache.funds.len() >= MAX_FUNDS {
                return Err(
                    "Arquivo histórico atingiu o limite de fundos. Dados preservados.".into(),
                );
            }
            cache.funds.entry(key.clone()).or_insert_with(|| Fund {
                request: request.clone(),
                listing: None,
                previous_listing: None,
                documents: BTreeMap::new(),
                errors: BTreeMap::new(),
            });
            let from = lookup_start(request)?;
            let through = Utc::now().date_naive().to_string();
            let reset = cache.funds[&key].listing.as_ref().is_none_or(|listing| {
                listing.from_date > from
                    || listing.through_date < request.end_date
                    || (listing.complete && !fresh(Some(&listing.checked_at), TTL, Utc::now()))
            });
            if reset {
                let fund = cache.funds.get_mut(&key).unwrap();
                if fund
                    .listing
                    .as_ref()
                    .is_some_and(|listing| listing.complete)
                {
                    fund.previous_listing = fund.listing.take();
                }
                fund.listing = Some(Listing {
                    from_date: from.clone(),
                    through_date: through.clone(),
                    checked_at: timestamp(),
                    complete: false,
                    next_start: 0,
                    total: 0,
                    documents: vec![],
                });
                write_file(&self.cache_path, &cache)?;
            }
            // Finish the index before selecting active filing versions. Old
            // documents are retained locally but cannot remain counted after a
            // replacement or cancellation disappears from the active index.
            while !cache.funds[&key].listing.as_ref().unwrap().complete && calls < MAX_BATCH {
                let listing = cache.funds[&key].listing.as_ref().unwrap().clone();
                let work = format!(
                    "fnet-list-{}-{}",
                    short_hash(&format!(
                        "{key}:{}:{}",
                        listing.from_date, listing.through_date
                    )),
                    listing.next_start
                );
                if session.is_none() {
                    match self.session(&work, &mut cache, &mut gate, &mut calls).await {
                        Ok(value) => *session = Some(value),
                        Err(error) => {
                            issues.push(error);
                            return Ok(panel(&cache, requests, issues));
                        }
                    }
                }
                if calls >= MAX_BATCH {
                    break;
                }
                if let Some(reason) =
                    reserve(&self.cache_path, &mut cache, &mut gate, &work, &mut calls)?
                {
                    issues.push(reason);
                    break;
                }
                let parameters = listing_parameters(request, &listing);
                let result = self
                    .get(
                        &format!("{BASE}pesquisarGerenciadorDocumentosDados"),
                        Some(&parameters),
                        session.as_mut().unwrap(),
                        &mut cache,
                    )
                    .await
                    .and_then(|bytes| parse_listing(&bytes, listing.next_start));
                match result {
                    Ok(page) => {
                        let listing = cache.funds.get_mut(&key).unwrap().listing.as_mut().unwrap();
                        if listing.next_start > 0 && listing.total != page.total {
                            issues.push(format!("{}: o índice de documentos mudou durante a consulta; a cobertura será conferida novamente.",request.symbol));
                            listing.next_start = 0;
                            listing.documents.clear();
                            listing.total = 0;
                            write_file(&self.cache_path, &cache)?;
                            break;
                        }
                        append_page(listing, page)?;
                        write_file(&self.cache_path, &cache)?;
                    }
                    Err(error) => {
                        issues.push(format!("{}: {error}", request.symbol));
                        write_file(&self.cache_path, &cache)?;
                        break;
                    }
                }
            }
            if !cache.funds[&key].listing.as_ref().unwrap().complete {
                continue;
            }
            let documents =
                selected_documents(&cache.funds[&key].listing.as_ref().unwrap().documents)?;
            for document in documents {
                if calls >= MAX_BATCH {
                    break;
                }
                if cache.funds[&key]
                    .documents
                    .get(&document.id)
                    .is_some_and(|saved| saved.version == document.version)
                {
                    continue;
                }
                let work = format!("fnet-doc-{}-{}", document.id, document.version);
                if session.is_none() {
                    match self.session(&work, &mut cache, &mut gate, &mut calls).await {
                        Ok(value) => *session = Some(value),
                        Err(error) => {
                            issues.push(error);
                            return Ok(panel(&cache, requests, issues));
                        }
                    }
                }
                if calls >= MAX_BATCH {
                    break;
                }
                if let Some(reason) =
                    reserve(&self.cache_path, &mut cache, &mut gate, &work, &mut calls)?
                {
                    issues.push(reason);
                    continue;
                }
                let url = format!("{BASE}exibirDocumento?id={}", document.id);
                let fetched_at = timestamp();
                let result = self
                    .get(&url, None, session.as_mut().unwrap(), &mut cache)
                    .await
                    .and_then(|bytes| {
                        parse_fund_income_document(
                            &bytes,
                            &request.symbol,
                            &request.cnpj,
                            document.id,
                            &fetched_at,
                        )
                    });
                match result {
                    Ok(mut events) => {
                        if let Some(expected) = &request.isin {
                            if events.iter().any(|event| {
                                event
                                    .isin_code
                                    .as_ref()
                                    .is_some_and(|isin| isin != expected)
                            }) {
                                cache.funds.get_mut(&key).unwrap().errors.insert(
                                    document.id,
                                    "ISIN do documento não corresponde ao ativo solicitado.".into(),
                                );
                                write_file(&self.cache_path, &cache)?;
                                continue;
                            }
                        }
                        for event in &mut events {
                            event.remarks = Some(format!(
                                "Aviso oficial Fundos.NET nº {}, versão {}. {}",
                                document.id,
                                document.version,
                                event.remarks.as_deref().unwrap_or("")
                            ));
                        }
                        let fund = cache.funds.get_mut(&key).unwrap();
                        fund.documents.insert(
                            document.id,
                            SavedDocument {
                                version: document.version,
                                fetched_at,
                                events,
                            },
                        );
                        fund.errors.remove(&document.id);
                    }
                    Err(error) => {
                        cache
                            .funds
                            .get_mut(&key)
                            .unwrap()
                            .errors
                            .insert(document.id, error.clone());
                        issues.push(format!(
                            "{}: documento {} pendente: {error}",
                            request.symbol, document.id
                        ));
                    }
                }
                write_file(&self.cache_path, &cache)?;
                if cache
                    .retry_after
                    .as_deref()
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                    .is_some_and(|until| until > Utc::now())
                {
                    return Ok(panel(&cache, requests, issues));
                }
            }
        }
        Ok(panel(&cache, requests, issues))
    }

    async fn session(
        &self,
        work: &str,
        cache: &mut Cache,
        gate: &mut RefreshGate,
        calls: &mut usize,
    ) -> Result<Session, String> {
        let key = format!("fnet-session-{}", short_hash(work));
        if let Some(reason) = reserve(&self.cache_path, cache, gate, &key, calls)? {
            return Err(reason);
        }
        let mut session = Session::default();
        let body = self.get(INDEX, None, &mut session, cache).await?;
        let text = std::str::from_utf8(&body)
            .map_err(|_| "Página pública do Fundos.NET com codificação inesperada.".to_string())?;
        session.csrf = csrf_headers(text)?;
        if !text.contains("formListarDocumentosEnviadosCVM") {
            return Err("Fundos.NET não retornou o índice público esperado. Nenhuma proteção de acesso foi contornada.".into());
        }
        session.created_at = Some(Utc::now());
        Ok(session)
    }

    async fn get(
        &self,
        url: &str,
        query: Option<&Vec<(String, String)>>,
        session: &mut Session,
        cache: &mut Cache,
    ) -> Result<Vec<u8>, String> {
        let mut request = self.client.get(url).headers(session.csrf.clone());
        if !session.cookies.is_empty() {
            request = request.header(
                COOKIE,
                session
                    .cookies
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        if let Some(query) = query {
            request = request.query(query);
        }
        let response = request.send().await.map_err(|_| {
            "Não foi possível consultar o Fundos.NET; o progresso salvo foi preservado.".to_string()
        })?;
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let seconds = retry_seconds(
                response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok()),
                Utc::now(),
            );
            cache.retry_after = Some(
                (Utc::now() + ChronoDuration::seconds(seconds))
                    .to_rfc3339_opts(SecondsFormat::Secs, true),
            );
            write_file(&self.cache_path, cache)?;
            return Err(
                "Fundos.NET solicitou uma pausa nas consultas. Os dados salvos foram preservados."
                    .into(),
            );
        }
        if !response.status().is_success() {
            if [
                reqwest::StatusCode::FORBIDDEN,
                reqwest::StatusCode::UNAUTHORIZED,
            ]
            .contains(&response.status())
            {
                cache.retry_after = Some(
                    (Utc::now() + ChronoDuration::hours(1))
                        .to_rfc3339_opts(SecondsFormat::Secs, true),
                );
                write_file(&self.cache_path, cache)?;
            }
            return Err(format!(
                "Fundos.NET retornou HTTP {}. Nenhuma proteção de acesso foi contornada.",
                response.status().as_u16()
            ));
        }
        for cookie in response.headers().get_all(SET_COOKIE) {
            if let Ok(value) = cookie.to_str() {
                if let Some(pair) = value.split(';').next() {
                    if let Some((name, _)) = pair.split_once('=') {
                        if name.len() < 100 && pair.len() < 4096 && session.cookies.len() < 8 {
                            session.cookies.insert(name.to_string(), pair.to_string());
                        }
                    }
                }
            }
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_BYTES as u64)
        {
            return Err("Resposta do Fundos.NET excedeu o limite seguro.".into());
        }
        let mut response = response;
        let mut bytes = vec![];
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Resposta do Fundos.NET incompleta.".to_string())?
        {
            if bytes.len() + chunk.len() > MAX_BYTES {
                return Err("Resposta do Fundos.NET excedeu o limite seguro.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

#[derive(Default)]
struct Session {
    cookies: BTreeMap<String, String>,
    csrf: HeaderMap,
    created_at: Option<DateTime<Utc>>,
}

fn csrf_headers(html: &str) -> Result<HeaderMap, String> {
    let mut values = BTreeMap::new();
    for part in html.split('<').filter(|part| part.starts_with("meta ")) {
        let part = part.split('>').next().unwrap_or_default();
        let name = attribute(part, "name");
        let value = attribute(part, "content");
        if let (Some(name), Some(value)) = (name, value) {
            if name == "_csrf" || name == "_csrf_header" {
                values.insert(name, value);
            }
        }
    }
    let mut result = HeaderMap::new();
    if let (Some(name), Some(value)) = (values.get("_csrf_header"), values.get("_csrf")) {
        if name.to_ascii_lowercase() != "x-csrf-token"
            && name.to_ascii_lowercase() != "x-xsrf-token"
        {
            return Err("Cabeçalho público CSRF inesperado no Fundos.NET.".into());
        }
        result.insert(
            HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| "Cabeçalho CSRF inválido.".to_string())?,
            HeaderValue::from_str(value).map_err(|_| "Valor CSRF inválido.".to_string())?,
        );
    }
    Ok(result)
}
fn attribute(tag: &str, name: &str) -> Option<String> {
    for quote in ['\"', '\''] {
        let marker = format!("{name}={quote}");
        if let Some((_, rest)) = tag.split_once(&marker) {
            return rest.split(quote).next().map(str::to_string);
        }
    }
    None
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexPage {
    data: Vec<IndexDocument>,
    records_filtered: usize,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexDocument {
    id: u64,
    versao: u32,
    data_referencia: String,
    data_entrega: String,
    categoria_documento: String,
    tipo_documento: String,
    status: String,
}
struct ParsedPage {
    documents: Vec<Document>,
    total: usize,
    received: usize,
}
fn parse_listing(bytes: &[u8], start: usize) -> Result<ParsedPage, String> {
    let page: IndexPage = serde_json::from_slice(bytes)
        .map_err(|_| "Índice histórico do Fundos.NET inválido.".to_string())?;
    if page.records_filtered > MAX_DOCUMENTS
        || page.data.len() > PAGE_SIZE
        || start > page.records_filtered
        || page.data.len() > page.records_filtered.saturating_sub(start)
    {
        return Err("Quantidade de documentos fora do limite seguro.".into());
    }
    if page.data.is_empty() && start < page.records_filtered {
        return Err(
            "Página histórica vazia antes do fim do índice; cobertura não confirmada.".into(),
        );
    }
    let received = page.data.len();
    let mut documents = vec![];
    for row in page.data {
        // The server-side category filter is checked again. An unexpected PDF,
        // inactive version, or another category must never become cash income.
        if !row.status.starts_with('A') {
            continue;
        }
        if normalize_text(&row.categoria_documento) != "avisoaoscotistasestruturado"
            || normalize_text(&row.tipo_documento) != "rendimentoseamortizacoes"
        {
            continue;
        }
        if row.id == 0
            || row.versao == 0
            || row.versao > 1000
            || NaiveDate::parse_from_str(&row.data_referencia, "%d/%m/%Y").is_err()
            || chrono::NaiveDateTime::parse_from_str(&row.data_entrega, "%d/%m/%Y %H:%M").is_err()
        {
            return Err("Metadados do aviso de rendimentos inválidos.".into());
        }
        documents.push(Document {
            id: row.id,
            version: row.versao,
            reference_date: row.data_referencia,
            delivered_at: row.data_entrega,
        });
    }
    Ok(ParsedPage {
        documents,
        total: page.records_filtered,
        received,
    })
}
fn append_page(listing: &mut Listing, page: ParsedPage) -> Result<(), String> {
    let mut ids: BTreeSet<_> = listing.documents.iter().map(|value| value.id).collect();
    if page.documents.iter().any(|doc| !ids.insert(doc.id)) {
        return Err("Documento repetido entre páginas; cobertura histórica não confirmada.".into());
    }
    listing.next_start += page.received;
    listing.total = page.total;
    listing.documents.extend(page.documents);
    listing.complete = listing.next_start == listing.total;
    listing.checked_at = timestamp();
    Ok(())
}
fn selected_documents(documents: &[Document]) -> Result<Vec<Document>, String> {
    // The completed active index is authoritative for cancelled/replaced IDs.
    // Two active filings on one date can concern different subscription receipts;
    // only the parsed ordinary ticker can establish an economic duplicate.
    let mut ids = BTreeSet::new();
    if documents.iter().any(|document| !ids.insert(document.id)) {
        return Err("ID de documento histórico repetido.".into());
    }
    let mut selected = documents.to_vec();
    selected.sort_by_key(|document| {
        NaiveDate::parse_from_str(&document.reference_date, "%d/%m/%Y").ok()
    });
    Ok(selected)
}
fn listing_parameters(
    request: &HistoricalDividendRequest,
    listing: &Listing,
) -> Vec<(String, String)> {
    // An unset type is omitted by the official form. Literal zero is *not* all
    // types: it silently returns an empty list for otherwise valid funds.
    [
        ("d", "1".into()),
        ("s", listing.next_start.to_string()),
        ("l", PAGE_SIZE.to_string()),
        ("o[0][dataEntrega]", "asc".into()),
        ("cnpjFundo", request.cnpj.clone()),
        ("cnpj", request.cnpj.clone()),
        ("idCategoriaDocumento", "14".into()),
        ("situacao", "A".into()),
        ("paginaCertificados", "false".into()),
        (
            "dataInicial",
            NaiveDate::parse_from_str(&listing.from_date, "%Y-%m-%d")
                .unwrap()
                .format("%d/%m/%Y")
                .to_string(),
        ),
        (
            "dataFinal",
            NaiveDate::parse_from_str(&listing.through_date, "%Y-%m-%d")
                .unwrap()
                .format("%d/%m/%Y")
                .to_string(),
        ),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect()
}

fn normalize_requests(
    requests: &[HistoricalDividendRequest],
    now: DateTime<Utc>,
) -> Result<Vec<HistoricalDividendRequest>, String> {
    if requests.len() > MAX_FUNDS {
        return Err("Consulta histórica excedeu o limite de fundos.".into());
    }
    let mut result = BTreeMap::new();
    for original in requests {
        let mut request = original.clone();
        request.symbol = normalize_symbol(&request.symbol)?;
        if request.symbol.len() != 6 || !request.symbol.ends_with("11") {
            return Err("Consulte apenas a cota ordinária de um fundo listado.".into());
        }
        let digits: String = request.cnpj.chars().filter(char::is_ascii_digit).collect();
        if digits.len() != 14
            || request
                .cnpj
                .chars()
                .any(|c| !c.is_ascii_digit() && !"./-".contains(c))
        {
            return Err("CNPJ público do fundo inválido.".into());
        }
        crate::cvm::normalize_cnpj(&request.cnpj)?;
        request.cnpj = format!(
            "{}.{}.{}/{}-{}",
            &digits[..2],
            &digits[2..5],
            &digits[5..8],
            &digits[8..12],
            &digits[12..]
        );
        let start = NaiveDate::parse_from_str(&request.start_date, "%Y-%m-%d")
            .map_err(|_| "Data inicial do histórico inválida.".to_string())?;
        let end = NaiveDate::parse_from_str(&request.end_date, "%Y-%m-%d")
            .map_err(|_| "Data final do histórico inválida.".to_string())?;
        if start > end
            || start < NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()
            || end > now.date_naive()
            || end.signed_duration_since(start).num_days() > 15 * 366
        {
            return Err("Período histórico inválido ou fora do limite seguro.".into());
        }
        if request.isin.as_ref().is_some_and(|isin| {
            isin.len() != 12
                || !isin
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        }) {
            return Err("ISIN do fundo inválido.".into());
        }
        let key = identity(&request);
        if result.insert(key, request).is_some() {
            return Err("Identidade de fundo repetida na consulta histórica.".into());
        }
    }
    Ok(result.into_values().collect())
}
fn lookup_start(request: &HistoricalDividendRequest) -> Result<String, String> {
    let date = NaiveDate::parse_from_str(&request.start_date, "%Y-%m-%d")
        .map_err(|_| "Data inicial inválida.".to_string())?;
    Ok((date - ChronoDuration::days(120)).to_string())
}
fn identity(request: &HistoricalDividendRequest) -> String {
    format!(
        "{}:{}",
        request.symbol,
        request
            .cnpj
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
    )
}
fn normalize_text(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'ç' => 'c',
            'ã' | 'á' | 'à' | 'â' => 'a',
            'é' | 'ê' => 'e',
            'í' => 'i',
            'ó' | 'ô' | 'õ' => 'o',
            'ú' => 'u',
            other => other,
        })
        .filter(char::is_ascii_alphanumeric)
        .collect()
}
fn short_hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))[..24].into()
}
fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn reserve(
    path: &Path,
    cache: &mut Cache,
    gate: &mut RefreshGate,
    key: &str,
    calls: &mut usize,
) -> Result<Option<String>, String> {
    let now = Utc::now();
    if let Some(until) = cache
        .retry_after
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
    {
        if until > now {
            return Ok(Some(format!(
                "Fundos.NET: nova tentativa após {}.",
                until.to_rfc3339()
            )));
        }
    }
    cache.attempts.retain(|value| {
        DateTime::parse_from_rfc3339(value)
            .is_ok_and(|at| now - at.with_timezone(&Utc) < ChronoDuration::hours(24))
    });
    if cache.attempts.len() >= DAILY_LIMIT as usize {
        return Ok(Some("Limite local do Fundos.NET nas últimas 24 horas atingido. O histórico salvo foi preservado.".into()));
    }
    if let Some(reason) = gate.reserve(key, None, 0, now, Some(DAILY_LIMIT))? {
        return Ok(Some(reason));
    }
    cache.attempts.push(timestamp());
    write_file(path, cache)?;
    *calls += 1;
    Ok(None)
}

fn panel(
    cache: &Cache,
    requests: &[HistoricalDividendRequest],
    mut issues: Vec<String>,
) -> HistoricalDividendPanel {
    let mut result = HistoricalDividendPanel::default();
    for request in requests {
        let fund = cache.funds.get(&identity(request));
        let listing = fund.and_then(|fund| fund.listing.as_ref());
        let listing_valid = listing.is_some_and(|listing| {
            listing.from_date <= lookup_start(request).unwrap_or_default()
                && listing.through_date >= request.end_date
        });
        let listing_complete = listing_valid && listing.is_some_and(|listing| listing.complete);
        // A stale completed archive remains visible while its replacement index
        // is downloaded. Its coverage is pending until the new active set is
        // complete; a transient HTTP failure must not erase known income.
        let selected_listing = if listing_complete {
            listing
        } else {
            fund.and_then(|fund| fund.previous_listing.as_ref())
                .filter(|old| {
                    old.complete
                        && old.from_date <= request.end_date
                        && old.through_date >= request.start_date
                })
        };
        let documents = selected_listing.map(|listing| selected_documents(&listing.documents));
        let mut events = vec![];
        let mut pending = 0;
        let mut total = 0;
        if let Some(documents) = documents {
            match documents {
                Ok(documents) => {
                    total = documents.len();
                    for document in documents {
                        let saved = fund
                            .and_then(|fund| fund.documents.get(&document.id))
                            .filter(|saved| saved.version == document.version);
                        if let Some(saved) = saved {
                            if saved
                                .events
                                .iter()
                                .any(|event| event.date_precision != "exact")
                            {
                                pending += 1;
                                issues.push(format!("{}: documento {} possui data incompleta, sem pagamento presumido.",request.symbol,document.id));
                            }
                            events.extend(
                                saved
                                    .events
                                    .iter()
                                    .filter(|event| {
                                        event.payment_date.as_ref().is_none_or(|date| {
                                            date >= &request.start_date && date <= &request.end_date
                                        })
                                    })
                                    .cloned(),
                            );
                        } else {
                            pending += 1;
                            if let Some(error) = fund.and_then(|fund| fund.errors.get(&document.id))
                            {
                                issues.push(format!(
                                    "{}: documento {} pendente: {}",
                                    request.symbol, document.id, error
                                ));
                            }
                        }
                    }
                }
                Err(error) => {
                    issues.push(format!("{}: {error}", request.symbol));
                    pending = 1;
                }
            }
        } else {
            pending = 1;
        }
        if !listing_complete {
            pending = pending.max(1);
        }
        events.sort_by(|a, b| {
            a.payment_date
                .cmp(&b.payment_date)
                .then(a.event_id.cmp(&b.event_id))
        });
        events.dedup_by(|a, b| a.event_id == b.event_id);
        if listing_complete && total == 0 {
            pending = 1;
            issues.push(format!("{}: nenhum aviso de rendimentos foi encontrado. Uma lista vazia não comprova ausência de pagamentos.",request.symbol));
        }
        let complete = listing_complete && pending == 0;
        let fetched_at = selected_listing
            .or(listing)
            .map(|listing| listing.checked_at.clone());
        let observation=(!events.is_empty()).then(||DividendObservation{symbol:request.symbol.clone(),events,currency:"BRL".into(),fetched_at:fetched_at.clone().unwrap_or_default(),source:"fundosnet".into(),source_url:INDEX.into(),coverage_warning:if complete {"Avisos oficiais consultados no período; compras sem dia informado ainda podem deixar o direito ao primeiro pagamento pendente.".into()}else{"Histórico oficial em preenchimento; documentos ausentes ou conflitantes não são pagamentos de valor zero.".into()}});
        result.items.push(DividendItem {
            symbol: request.symbol.clone(),
            status: if complete {
                "available"
            } else if observation.is_some() {
                "cached"
            } else {
                "unavailable"
            }
            .into(),
            observation,
            error: (!complete).then(|| "Histórico de proventos ainda incompleto.".into()),
        });
        result.coverage.push(HistoricalDividendCoverage {
            symbol: request.symbol.clone(),
            start_date: request.start_date.clone(),
            end_date: request.end_date.clone(),
            listing_complete,
            documents_total: total,
            documents_pending: pending,
            complete,
            fetched_at,
        });
        result.pending_documents += pending;
    }
    issues.sort();
    issues.dedup();
    result.issues = issues;
    result
}

/// The core reconciles identical economic events across the recent B3 feed and
/// this independent historical source. A missing/conflicting date is preserved
/// as unresolved evidence instead of being silently discarded or paid twice.
pub fn income_events(panel: &HistoricalDividendPanel) -> Vec<ivestfii_core::income::IncomeEvent> {
    panel
        .items
        .iter()
        .filter_map(|item| item.observation.as_ref())
        .filter(|observation| observation.source == "fundosnet" && observation.source_url == INDEX)
        .flat_map(|observation| {
            observation
                .events
                .iter()
                .filter(move |event| {
                    event.symbol == observation.symbol
                        && event.kind == "income"
                        && event.verified == Some(true)
                        && ["exact", "incomplete"].contains(&event.date_precision.as_str())
                        && valid_document_url(&event.source_url)
                        && event
                            .isin_code
                            .as_ref()
                            .is_some_and(|isin| ordinary_isin(&event.symbol, isin))
                        && document_id_from_url(&event.source_url).is_some_and(|id| {
                            event.event_id == format!("fundosnet:{id}:{}:income", event.symbol)
                        })
                })
                .map(|event| ivestfii_core::income::IncomeEvent {
                    symbol: event.symbol.clone(),
                    amount_per_share: event.per_share.clone(),
                    record_date: if event.reconciliation_pending {
                        String::new()
                    } else {
                        event.record_date.clone().unwrap_or_default()
                    },
                    payment_date: event.payment_date.clone().unwrap_or_default(),
                    source_url: event.source_url.clone(),
                    source: HISTORICAL_DIVIDEND_SOURCE.into(),
                    id: event.event_id.clone(),
                })
        })
        .collect()
}
fn valid_document_url(value: &str) -> bool {
    value
        .strip_prefix(&format!("{BASE}exibirDocumento?id="))
        .is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix.len() <= 20
                && suffix.bytes().all(|b| b.is_ascii_digit())
                && suffix.parse::<u64>().is_ok_and(|id| id > 0)
        })
}
fn document_id_from_url(value: &str) -> Option<u64> {
    if !valid_document_url(value) {
        return None;
    }
    value.rsplit('=').next()?.parse().ok()
}
fn ordinary_isin(symbol: &str, isin: &str) -> bool {
    symbol.len() == 6
        && symbol.ends_with("11")
        && isin.len() == 12
        && isin.starts_with(&format!("BR{}CTF", &symbol[..4]))
        && isin.as_bytes()[9..].iter().all(u8::is_ascii_digit)
}
fn retry_seconds(value: Option<&str>, now: DateTime<Utc>) -> i64 {
    value
        .and_then(|value| {
            value.parse::<i64>().ok().or_else(|| {
                DateTime::parse_from_rfc2822(value)
                    .ok()
                    .map(|date| (date.with_timezone(&Utc) - now).num_seconds())
            })
        })
        .unwrap_or(900)
        .clamp(300, 30 * 86400)
}

fn validate_cache(cache: &Cache) -> Result<(), String> {
    if cache.version != 1
        || cache.funds.len() > MAX_FUNDS
        || cache.attempts.len() > DAILY_LIMIT as usize
    {
        return Err(
            "Arquivo histórico contém versão ou tamanho inválido; dados preservados.".into(),
        );
    }
    let now = Utc::now();
    if cache.attempts.iter().any(|value| {
        DateTime::parse_from_rfc3339(value).map_or(true, |at| at > now + ChronoDuration::minutes(5))
    }) {
        return Err("Datas da proteção histórica inválidas.".into());
    }
    if cache.retry_after.as_ref().is_some_and(|value| {
        DateTime::parse_from_rfc3339(value).map_or(true, |at| at > now + ChronoDuration::days(31))
    }) {
        return Err("Data de retomada histórica inválida.".into());
    }
    for (key, fund) in &cache.funds {
        normalize_requests(std::slice::from_ref(&fund.request), now)?;
        if *key != identity(&fund.request)
            || fund.documents.len() > MAX_DOCUMENTS
            || fund.errors.len() > MAX_DOCUMENTS
        {
            return Err("Identidade ou quantidade inválida no histórico de proventos.".into());
        }
        for listing in fund.listing.iter().chain(fund.previous_listing.iter()) {
            if listing.documents.len() > MAX_DOCUMENTS
                || listing.total > MAX_DOCUMENTS
                || listing.next_start > listing.total
                || (!listing.complete && listing.next_start == listing.total && listing.total != 0)
                || DateTime::parse_from_rfc3339(&listing.checked_at).is_err()
                || NaiveDate::parse_from_str(&listing.from_date, "%Y-%m-%d").is_err()
                || NaiveDate::parse_from_str(&listing.through_date, "%Y-%m-%d").is_err()
            {
                return Err("Índice histórico salvo inválido.".into());
            }
        }
        for (id, saved) in &fund.documents {
            if *id == 0
                || saved.version == 0
                || saved.events.len() > 100
                || DateTime::parse_from_rfc3339(&saved.fetched_at)
                    .map_or(true, |at| at > now + ChronoDuration::minutes(5))
                || saved.events.iter().any(|event| {
                    event.symbol != fund.request.symbol
                        || event.source_url != format!("{BASE}exibirDocumento?id={id}")
                        || event.kind != "income"
                        || event.verified != Some(true)
                        || event.event_id
                            != format!("fundosnet:{id}:{}:income", fund.request.symbol)
                        || event.isin_code.as_ref().is_none_or(|isin| {
                            !ordinary_isin(&fund.request.symbol, isin)
                                || fund
                                    .request
                                    .isin
                                    .as_ref()
                                    .is_some_and(|expected| expected != isin)
                        })
                        || event
                            .per_share
                            .parse::<rust_decimal::Decimal>()
                            .map_or(true, |value| value < rust_decimal::Decimal::ZERO)
                        || !["exact", "incomplete"].contains(&event.date_precision.as_str())
                        || event.payment_date.as_ref().is_some_and(|date| {
                            NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err()
                        })
                        || event.record_date.as_ref().is_some_and(|date| {
                            NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err()
                        })
                        || (event.date_precision == "exact"
                            && (event.payment_date.is_none() || event.record_date.is_none()))
                })
            {
                return Err("Evento histórico salvo inválido; arquivo preservado.".into());
            }
        }
    }
    Ok(())
}
fn read_file(path: &Path) -> Result<Cache, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Cache::default()),
        Err(_) => return Err("Não foi possível ler o histórico de proventos.".into()),
    };
    let mut bytes = vec![];
    file.take(MAX_CACHE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Não foi possível ler o histórico de proventos.".to_string())?;
    if bytes.len() > MAX_CACHE {
        return Err("Histórico de proventos excedeu o limite seguro.".into());
    }
    let cache: Cache = serde_json::from_slice(&bytes)
        .map_err(|_| "Histórico de proventos inválido; arquivo preservado.".to_string())?;
    validate_cache(&cache)?;
    Ok(cache)
}
fn write_file(path: &Path, cache: &Cache) -> Result<(), String> {
    validate_cache(cache)?;
    let bytes = serde_json::to_vec(cache)
        .map_err(|_| "Não foi possível serializar o histórico de proventos.".to_string())?;
    if bytes.len() > MAX_CACHE {
        return Err("Histórico de proventos excedeu o limite seguro; arquivo preservado.".into());
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|_| "Não foi possível preparar o histórico de proventos.".to_string())?;
    }
    let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(
            "Não foi possível salvar o histórico de proventos; arquivo anterior preservado.".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> HistoricalDividendRequest {
        HistoricalDividendRequest {
            symbol: "ABCD11".into(),
            cnpj: "12.345.678/0001-95".into(),
            isin: Some("BRABCDCTF001".into()),
            start_date: "2023-03-01".into(),
            end_date: "2026-09-30".into(),
        }
    }
    fn document(id: u64, version: u32) -> Document {
        Document {
            id,
            version,
            reference_date: "28/02/2023".into(),
            delivered_at: "28/02/2023 18:00".into(),
        }
    }
    fn event(id: u64) -> DividendEvent {
        DividendEvent {
            event_id: format!("fundosnet:{id}:ABCD11:income"),
            symbol: "ABCD11".into(),
            per_share: "0.123456789012".into(),
            kind: "income".into(),
            label: "Rendimento".into(),
            payment_date: Some("2023-03-15".into()),
            record_date: Some("2023-02-28".into()),
            ex_date: None,
            declared_date: Some("2023-02-28".into()),
            date_precision: "exact".into(),
            verified: Some(true),
            remarks: None,
            related_to: None,
            isin_code: Some("BRABCDCTF001".into()),
            source_url: format!("{BASE}exibirDocumento?id={id}"),
            last_observed_at: Some(timestamp()),
            reconciliation_pending: false,
        }
    }
    fn cache_with(documents: Vec<Document>) -> Cache {
        let request = request();
        let mut cache = Cache::default();
        let saved = documents
            .iter()
            .map(|document| {
                (
                    document.id,
                    SavedDocument {
                        version: document.version,
                        fetched_at: timestamp(),
                        events: vec![event(document.id)],
                    },
                )
            })
            .collect();
        cache.funds.insert(
            identity(&request),
            Fund {
                request,
                listing: Some(Listing {
                    from_date: "2022-11-01".into(),
                    through_date: "2026-10-20".into(),
                    checked_at: timestamp(),
                    complete: true,
                    next_start: documents.len(),
                    total: documents.len(),
                    documents,
                }),
                previous_listing: None,
                documents: saved,
                errors: BTreeMap::new(),
            },
        );
        cache
    }
    #[test]
    fn type_filter_is_omitted_and_only_public_identity_and_calendar_leave() {
        let request = request();
        let cache = cache_with(vec![]);
        let listing = cache
            .funds
            .values()
            .next()
            .unwrap()
            .listing
            .as_ref()
            .unwrap();
        let parameters = listing_parameters(&request, listing);
        assert!(!parameters.iter().any(|(key, _)| key == "tipoFundo"));
        assert!(parameters
            .iter()
            .any(|(key, value)| key == "cnpjFundo" && value == &request.cnpj));
        assert!(parameters
            .iter()
            .any(|(key, value)| key == "idCategoriaDocumento" && value == "14"));
        assert!(lookup_start(&request).unwrap() < request.start_date);
    }
    #[test]
    fn pagination_rejects_truncation_repeated_ids_and_changed_totals() {
        let row = serde_json::json!({"id":10,"versao":1,"dataReferencia":"28/02/2023","dataEntrega":"28/02/2023 18:00","categoriaDocumento":"Aviso aos Cotistas - Estruturado","tipoDocumento":"Rendimentos e Amortizações","status":"AC"});
        let parsed = parse_listing(
            serde_json::to_vec(&serde_json::json!({"data":[row.clone()],"recordsFiltered":2}))
                .unwrap()
                .as_slice(),
            0,
        )
        .unwrap();
        let mut listing = Listing {
            from_date: "2023-01-01".into(),
            through_date: "2026-09-30".into(),
            checked_at: timestamp(),
            complete: false,
            next_start: 0,
            total: 0,
            documents: vec![],
        };
        append_page(&mut listing, parsed).unwrap();
        assert!(!listing.complete);
        assert_eq!(listing.next_start, 1);
        assert!(append_page(
            &mut listing,
            ParsedPage {
                documents: vec![document(10, 1)],
                total: 2,
                received: 1
            }
        )
        .is_err());
        assert!(parse_listing(br#"{"data":[],"recordsFiltered":2}"#, 1).is_err());
        assert!(parse_listing(br#"{"data":[],"recordsFiltered":2001}"#, 0).is_err());
    }
    #[test]
    fn inactive_and_other_categories_are_never_downloaded_as_income() {
        let rows = vec![
            serde_json::json!({"id":10,"versao":1,"dataReferencia":"28/02/2023","dataEntrega":"28/02/2023 18:00","categoriaDocumento":"Aviso aos Cotistas - Estruturado","tipoDocumento":"Rendimentos e Amortizações","status":"IC"}),
            serde_json::json!({"id":11,"versao":1,"dataReferencia":"28/02/2023","dataEntrega":"28/02/2023 18:00","categoriaDocumento":"Informes Periódicos","tipoDocumento":"Informe Mensal","status":"AC"}),
        ];
        let bytes =
            serde_json::to_vec(&serde_json::json!({"data":rows,"recordsFiltered":2})).unwrap();
        let parsed = parse_listing(&bytes, 0).unwrap();
        assert_eq!(parsed.received, 2);
        assert!(parsed.documents.is_empty());
    }
    #[test]
    fn completed_active_index_removes_cancelled_event_and_requires_new_version() {
        let mut cache = cache_with(vec![document(10, 1), document(11, 1)]);
        assert_eq!(
            panel(&cache, &[request()], vec![]).items[0]
                .observation
                .as_ref()
                .unwrap()
                .events
                .len(),
            2
        );
        let fund = cache.funds.values_mut().next().unwrap();
        fund.listing.as_mut().unwrap().documents = vec![document(11, 1)];
        let result = panel(&cache, &[request()], vec![]);
        assert_eq!(
            result.items[0].observation.as_ref().unwrap().events[0].event_id,
            "fundosnet:11:ABCD11:income"
        );
        cache
            .funds
            .values_mut()
            .next()
            .unwrap()
            .listing
            .as_mut()
            .unwrap()
            .documents = vec![document(11, 2)];
        let result = panel(&cache, &[request()], vec![]);
        assert!(!result.coverage[0].complete);
        assert_eq!(result.pending_documents, 1);
        assert!(result.items[0].observation.is_none());
    }
    #[test]
    fn partial_listing_empty_index_and_incomplete_dates_never_prove_zero() {
        let mut cache = cache_with(vec![document(10, 1)]);
        cache
            .funds
            .values_mut()
            .next()
            .unwrap()
            .listing
            .as_mut()
            .unwrap()
            .complete = false;
        assert!(!panel(&cache, &[request()], vec![]).coverage[0].complete);
        assert!(panel(&cache, &[request()], vec![]).items[0]
            .observation
            .is_none());
        let empty = panel(&cache_with(vec![]), &[request()], vec![]);
        assert!(!empty.coverage[0].complete);
        assert_eq!(empty.pending_documents, 1);
        let mut cache = cache_with(vec![document(10, 1)]);
        let event = &mut cache
            .funds
            .values_mut()
            .next()
            .unwrap()
            .documents
            .get_mut(&10)
            .unwrap()
            .events[0];
        event.record_date = None;
        event.date_precision = "incomplete".into();
        let result = panel(&cache, &[request()], vec![]);
        assert!(!result.coverage[0].complete);
        assert_eq!(income_events(&result)[0].record_date, "");
    }
    #[test]
    fn distinct_receipt_filings_are_not_selected_before_ticker_validation() {
        let selected = selected_documents(&[document(10, 1), document(11, 2)]).unwrap();
        assert_eq!(selected.len(), 2);
        assert!(selected_documents(&[document(10, 1), document(10, 1)]).is_err());
    }
    #[test]
    fn read_only_cache_preserves_money_precision_and_rejects_source_injection() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("public-income.json");
        let mut cache = cache_with(vec![document(10, 1)]);
        write_file(&path, &cache).unwrap();
        let before = fs::read(&path).unwrap();
        let service = HistoricalDividendService::new(path.clone()).unwrap();
        let result = service.read_cache(&[request()]).unwrap();
        assert_eq!(income_events(&result)[0].amount_per_share, "0.123456789012");
        assert_eq!(fs::read(&path).unwrap(), before);
        cache
            .funds
            .values_mut()
            .next()
            .unwrap()
            .documents
            .get_mut(&10)
            .unwrap()
            .events[0]
            .source_url = "https://example.org/10".into();
        assert!(write_file(&path, &cache).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let missing = temp.path().join("absent").join("public.json");
        assert_eq!(
            HistoricalDividendService::new(missing.clone())
                .unwrap()
                .read_cache(&[request()])
                .unwrap()
                .pending_documents,
            1
        );
        assert!(!missing.parent().unwrap().exists());
    }
    #[test]
    fn malformed_cache_is_not_replaced_and_untrusted_inputs_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("public-income.json");
        fs::write(&path, b"broken").unwrap();
        assert!(HistoricalDividendService::new(path.clone())
            .unwrap()
            .read_cache(&[request()])
            .is_err());
        assert_eq!(fs::read(path).unwrap(), b"broken");
        let mut request = request();
        request.cnpj = "12.345.678/0001-90&admin=true".into();
        assert!(normalize_requests(&[request], Utc::now()).is_err());
        assert!(!valid_document_url(&format!(
            "{BASE}exibirDocumento?id=10&next=https://example.org"
        )));
    }
    #[tokio::test]
    async fn every_attempt_is_reserved_before_http_and_rolling_midnight_cannot_double_budget() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("public-income.json");
        let mut cache = Cache::default();
        let mut gate = RefreshGate::acquire(&path).await.unwrap();
        let mut calls = 0;
        assert!(
            reserve(&path, &mut cache, &mut gate, "test-document", &mut calls)
                .unwrap()
                .is_none()
        );
        assert_eq!(calls, 1);
        assert_eq!(read_file(&path).unwrap().attempts.len(), 1);
        assert!(
            reserve(&path, &mut cache, &mut gate, "test-document", &mut calls)
                .unwrap()
                .is_some()
        );
        assert_eq!(calls, 1);
        cache.attempts =
            vec![(Utc::now() - ChronoDuration::hours(1)).to_rfc3339(); DAILY_LIMIT as usize];
        assert!(
            reserve(&path, &mut cache, &mut gate, "other-document", &mut calls)
                .unwrap()
                .unwrap()
                .contains("24 horas")
        );
        assert_eq!(calls, 1);
    }
    #[test]
    fn public_csrf_is_normal_session_metadata_and_cannot_change_unrelated_headers() {
        let headers=csrf_headers("<meta name=\"_csrf\" content=\"public-session\"><meta name=\"_csrf_header\" content=\"X-CSRF-TOKEN\">").unwrap();
        assert_eq!(headers["x-csrf-token"], "public-session");
        assert!(csrf_headers("<meta name=\"_csrf\" content=\"x\"><meta name=\"_csrf_header\" content=\"Authorization\">").is_err());
        assert!(csrf_headers("<html></html>").unwrap().is_empty());
    }
    #[test]
    fn previous_completed_archive_remains_visible_until_new_active_index_is_complete() {
        let mut cache = cache_with(vec![document(10, 1)]);
        let fund = cache.funds.values_mut().next().unwrap();
        fund.previous_listing = fund.listing.take();
        fund.listing = Some(Listing {
            from_date: "2022-11-01".into(),
            through_date: "2026-10-21".into(),
            checked_at: timestamp(),
            complete: false,
            next_start: 0,
            total: 0,
            documents: vec![],
        });
        let result = panel(&cache, &[request()], vec![]);
        assert_eq!(income_events(&result).len(), 1);
        assert!(!result.coverage[0].complete);
        assert!(!result.coverage[0].listing_complete);
        let mut next_day = request();
        next_day.end_date = "2026-10-21".into();
        assert_eq!(income_events(&panel(&cache, &[next_day], vec![])).len(), 1);
        let mut earlier = request();
        earlier.start_date = "2022-01-01".into();
        assert_eq!(income_events(&panel(&cache, &[earlier], vec![])).len(), 1);
        let listing = cache
            .funds
            .values_mut()
            .next()
            .unwrap()
            .listing
            .as_mut()
            .unwrap();
        listing.complete = true;
        let result = panel(&cache, &[request()], vec![]);
        assert!(income_events(&result).is_empty());
        assert!(!result.coverage[0].complete);
    }
    #[tokio::test]
    async fn persisted_global_pause_blocks_every_following_http_and_http_date_is_respected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("public-income.json");
        let mut cache = Cache {
            retry_after: Some((Utc::now() + ChronoDuration::hours(2)).to_rfc3339()),
            ..Cache::default()
        };
        write_file(&path, &cache).unwrap();
        let mut gate = RefreshGate::acquire(&path).await.unwrap();
        let mut calls = 0;
        assert!(reserve(
            &path,
            &mut cache,
            &mut gate,
            "another-fund-document",
            &mut calls
        )
        .unwrap()
        .is_some());
        assert_eq!(calls, 0);
        assert!(read_file(&path).unwrap().attempts.is_empty());
        let now = DateTime::parse_from_rfc3339("2026-10-21T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            retry_seconds(Some("Wed, 21 Oct 2026 15:00:00 GMT"), now),
            10800
        );
        assert_eq!(retry_seconds(Some("3600"), now), 3600);
        assert_eq!(retry_seconds(None, now), 900);
    }
    #[test]
    fn cached_receipt_isin_and_unrelated_document_id_are_rejected() {
        let mut cache = cache_with(vec![document(10, 1)]);
        let saved = cache
            .funds
            .values_mut()
            .next()
            .unwrap()
            .documents
            .get_mut(&10)
            .unwrap();
        saved.events[0].isin_code = Some("BRABCDR01M01".into());
        assert!(validate_cache(&cache).is_err());
        let mut cache = cache_with(vec![document(10, 1)]);
        cache
            .funds
            .values_mut()
            .next()
            .unwrap()
            .documents
            .get_mut(&10)
            .unwrap()
            .events[0]
            .event_id = "fundosnet:11:ABCD11:income".into();
        assert!(validate_cache(&cache).is_err());
    }
}

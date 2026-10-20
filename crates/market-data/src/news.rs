use crate::refresh_policy::{RefreshGate, RefreshOutcome};
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::{redirect::Policy, Client, Url};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    time::Duration,
};
use uuid::Uuid;

pub const NEWS_SOURCE: &str = "Agência Brasil";
pub const NEWS_SOURCE_URL: &str = "https://agenciabrasil.ebc.com.br/rss/economia/feed.xml";
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_ITEMS: usize = 20;
const MAX_TITLE_CHARS: usize = 240;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NewsItem {
    pub id: String,
    pub title: String,
    pub url: String,
    pub published_at: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NewsSnapshot {
    pub items: Vec<NewsItem>,
    pub source: String,
    pub source_url: String,
    pub fetched_at: String,
}

pub struct NewsService {
    client: Client,
    cache_path: PathBuf,
}

impl NewsService {
    pub fn new(cache_path: PathBuf) -> Result<Self, String> {
        if cache_path.file_name().is_none() {
            return Err("Informe um arquivo para o cache de notícias.".into());
        }
        let client = Client::builder()
            .user_agent("ivestfii/0.1 (RSS reader)")
            .https_only(true)
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| format!("Não foi possível preparar o leitor de notícias: {error}"))?;
        Ok(Self { client, cache_path })
    }

    /// Reads previously downloaded headlines without making a network request.
    pub fn read_cache(&self) -> Result<Option<NewsSnapshot>, String> {
        let mut file = match File::open(&self.cache_path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("Não foi possível ler o cache de notícias: {error}")),
        };
        let mut bytes = Vec::new();
        (&mut file)
            .take((MAX_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Não foi possível ler o cache de notícias: {error}"))?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err("O cache de notícias excede o limite de tamanho.".into());
        }
        let snapshot: NewsSnapshot = serde_json::from_slice(&bytes)
            .map_err(|_| "O cache de notícias não contém JSON válido.".to_owned())?;
        validate_snapshot(&snapshot)?;
        Ok(Some(snapshot))
    }

    /// Downloads only the fixed, public economy feed. Errors leave the old cache intact.
    pub async fn refresh(&self) -> Result<NewsSnapshot, String> {
        let mut policy = RefreshGate::acquire(&self.cache_path).await?;
        let cached = self.read_cache().ok().flatten();
        if cached.as_ref().is_some_and(|item| {
            crate::refresh_policy::fresh(Some(&item.fetched_at), 3600, Utc::now())
        }) {
            return Ok(cached.expect("checked cache"));
        }
        if let Some(message) = policy.reserve("feed", None, 3600, Utc::now(), None)? {
            return Err(message);
        }
        self.fetch().await
    }

    pub async fn refresh_if_due(&self) -> RefreshOutcome<NewsSnapshot> {
        let before = self.read_cache().ok().flatten();
        match self.refresh().await {
            Ok(data)
                if before
                    .as_ref()
                    .is_some_and(|previous| previous.fetched_at == data.fetched_at) =>
            {
                RefreshOutcome::cached(data)
            }
            Ok(data) => RefreshOutcome::updated(data),
            Err(error) => RefreshOutcome::failed(self.read_cache().ok().flatten(), error),
        }
    }

    async fn fetch(&self) -> Result<NewsSnapshot, String> {
        let mut response = self
            .client
            .get(NEWS_SOURCE_URL)
            .header("Accept", "application/rss+xml, application/xml, text/xml")
            .send()
            .await
            .map_err(|error| format!("Não foi possível consultar a Agência Brasil: {error}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "A Agência Brasil respondeu com HTTP {}.",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err("O feed de notícias excede o limite de tamanho.".into());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("Não foi possível receber o feed de notícias: {error}"))?
        {
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err("O feed de notícias excede o limite de tamanho.".into());
            }
            body.extend_from_slice(&chunk);
        }
        self.cache_response(&body)
    }

    fn cache_response(&self, body: &[u8]) -> Result<NewsSnapshot, String> {
        let fetched_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let snapshot = parse_feed(body, fetched_at)?;
        self.write_cache(snapshot)
    }

    fn write_cache(&self, snapshot: NewsSnapshot) -> Result<NewsSnapshot, String> {
        let parent = self
            .cache_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        fs::create_dir_all(parent)
            .map_err(|error| format!("Não foi possível criar o cache de notícias: {error}"))?;
        let lock_path = self.cache_path.with_extension("news.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|error| format!("Não foi possível abrir o bloqueio do cache: {error}"))?;
        lock.lock()
            .map_err(|error| format!("Não foi possível bloquear o cache de notícias: {error}"))?;
        // Multiple app/MCP processes can refresh concurrently. Do not replace a newer fetch.
        if let Ok(Some(current)) = self.read_cache() {
            let current_date = DateTime::parse_from_rfc3339(&current.fetched_at)
                .map_err(|_| "A data do cache de notícias é inválida.".to_owned())?;
            let new_date = DateTime::parse_from_rfc3339(&snapshot.fetched_at)
                .map_err(|_| "A data de coleta de notícias é inválida.".to_owned())?;
            if current_date > new_date {
                return Ok(current);
            }
        }
        let temp_path = parent.join(format!(".news-{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let bytes = serde_json::to_vec(&snapshot).map_err(|error| {
                format!("Não foi possível preparar o cache de notícias: {error}")
            })?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .map_err(|error| format!("Não foi possível criar o cache temporário: {error}"))?;
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|error| format!("Não foi possível salvar o cache de notícias: {error}"))?;
            drop(file);
            fs::rename(&temp_path, &self.cache_path).map_err(|error| {
                format!("Não foi possível substituir o cache de notícias: {error}")
            })?;
            Ok(snapshot)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp_path);
        }
        result
    }
}

fn parse_feed(body: &[u8], fetched_at: String) -> Result<NewsSnapshot, String> {
    if body.len() > MAX_RESPONSE_BYTES {
        return Err("O feed de notícias excede o limite de tamanho.".into());
    }
    let xml = std::str::from_utf8(body)
        .map_err(|_| "O feed de notícias não usa texto UTF-8 válido.".to_owned())?;
    let document = roxmltree::Document::parse(xml)
        .map_err(|_| "A Agência Brasil retornou um feed XML inválido.".to_owned())?;
    let root = document.root_element();
    if !root.has_tag_name("rss") {
        return Err("A resposta da Agência Brasil não é um feed RSS.".into());
    }
    let channel = root
        .children()
        .find(|node| node.has_tag_name("channel"))
        .ok_or_else(|| "O feed de notícias não possui um canal RSS.".to_owned())?;
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for item in channel.children().filter(|node| node.has_tag_name("item")) {
        let field = |name| {
            item.children()
                .find(|node| node.has_tag_name(name))
                .map(|node| {
                    node.descendants()
                        .filter(|child| child.is_text())
                        .filter_map(|child| child.text())
                        .collect::<String>()
                })
        };
        let title = plain_title(&field("title").unwrap_or_default());
        let Some(url) = field("link").and_then(|link| headline_url(&link)) else {
            continue;
        };
        if title.is_empty() || !seen.insert(url.clone()) {
            continue;
        }
        let published_at = field("pubDate").and_then(|value| {
            DateTime::parse_from_rfc2822(value.trim()).ok().map(|date| {
                date.with_timezone(&Utc)
                    .to_rfc3339_opts(SecondsFormat::Secs, true)
            })
        });
        items.push(NewsItem {
            id: url.clone(),
            title,
            url,
            published_at,
            source: NEWS_SOURCE.into(),
        });
        if items.len() == MAX_ITEMS {
            break;
        }
    }
    if items.is_empty() {
        return Err("O feed da Agência Brasil não contém notícias válidas.".into());
    }
    let snapshot = NewsSnapshot {
        items,
        source: NEWS_SOURCE.into(),
        source_url: NEWS_SOURCE_URL.into(),
        fetched_at,
    };
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

fn headline_url(value: &str) -> Option<String> {
    if value.len() > 2048 {
        return None;
    }
    let mut url = Url::parse(value.trim()).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("agenciabrasil.ebc.com.br")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
    {
        return None;
    }
    url.set_fragment(None);
    Some(url.into())
}

fn plain_title(value: &str) -> String {
    let mut inside_tag = false;
    let text: String = value
        .chars()
        .filter(|character| match character {
            '<' => {
                inside_tag = true;
                false
            }
            '>' if inside_tag => {
                inside_tag = false;
                false
            }
            _ => !inside_tag && (!character.is_control() || character.is_whitespace()),
        })
        .collect();
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect()
}

fn validate_snapshot(snapshot: &NewsSnapshot) -> Result<(), String> {
    if snapshot.source != NEWS_SOURCE
        || snapshot.source_url != NEWS_SOURCE_URL
        || DateTime::parse_from_rfc3339(&snapshot.fetched_at).is_err()
        || snapshot.items.is_empty()
        || snapshot.items.len() > MAX_ITEMS
    {
        return Err("O cache de notícias contém metadados inválidos.".into());
    }
    let mut seen = HashSet::new();
    for item in &snapshot.items {
        if item.source != NEWS_SOURCE
            || item.id != item.url
            || headline_url(&item.url).as_deref() != Some(item.url.as_str())
            || item.title.is_empty()
            || plain_title(&item.title) != item.title
            || item
                .published_at
                .as_deref()
                .is_some_and(|date| DateTime::parse_from_rfc3339(date).is_err())
            || !seen.insert(&item.id)
        {
            return Err("O cache contém uma notícia inválida.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(items: &str) -> Vec<u8> {
        format!("<?xml version=\"1.0\"?><rss version=\"2.0\"><channel>{items}</channel></rss>")
            .into_bytes()
    }

    fn item(title: &str, url: &str, date: &str) -> String {
        format!("<item><title>{title}</title><link>{url}</link>{date}</item>")
    }

    fn fetch_time() -> String {
        "2026-10-16T06:00:00.000Z".into()
    }

    fn service() -> (NewsService, PathBuf) {
        let directory = std::env::temp_dir().join(format!("ivestfii-news-test-{}", Uuid::new_v4()));
        let service = NewsService::new(directory.join("news.json")).unwrap();
        (service, directory)
    }

    #[tokio::test]
    async fn repeated_automatic_and_manual_refresh_reuses_headlines_without_http_or_new_dates() {
        let (service, directory) = service();
        let saved = service
            .cache_response(&feed(&item(
                "Teste público",
                "https://agenciabrasil.ebc.com.br/economia/teste",
                "",
            )))
            .unwrap();
        let original = fs::read(&service.cache_path).unwrap();
        let outcome = service.refresh_if_due().await;
        assert_eq!(outcome.status, "cached");
        assert!(outcome.message.is_none());
        assert_eq!(outcome.data, Some(saved.clone()));
        assert_eq!(service.refresh().await.unwrap(), saved);
        assert_eq!(fs::read(&service.cache_path).unwrap(), original);
        assert!(!service
            .cache_path
            .with_extension("refresh-policy.json")
            .exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn reads_publication_date_as_utc_and_retains_only_headline_fields() {
        let content = item(
            "<![CDATA[  Economia <b>hoje</b> & amanhã  ]]>",
            "https://agenciabrasil.ebc.com.br/economia/noticia/exemplo",
            "<pubDate>Thu, 15 Oct 2026 20:49:00 -0300</pubDate><description>Artigo completo que não deve ser guardado</description>",
        );
        let snapshot = parse_feed(&feed(&content), fetch_time()).unwrap();
        assert_eq!(snapshot.items[0].title, "Economia hoje & amanhã");
        assert_eq!(
            snapshot.items[0].published_at.as_deref(),
            Some("2026-10-15T23:49:00Z")
        );
        assert!(!serde_json::to_string(&snapshot)
            .unwrap()
            .contains("Artigo completo"));
    }

    #[test]
    fn removes_duplicates_and_does_not_invent_missing_publication_dates() {
        let first = item(
            "Notícia válida",
            "https://agenciabrasil.ebc.com.br/economia/um",
            "",
        );
        let snapshot = parse_feed(&feed(&first.repeat(2)), fetch_time()).unwrap();
        assert_eq!(snapshot.items.len(), 1);
        assert!(snapshot.items[0].published_at.is_none());
        let invalid_date = first.replace("</item>", "<pubDate>data desconhecida</pubDate></item>");
        assert!(parse_feed(&feed(&invalid_date), fetch_time())
            .unwrap()
            .items[0]
            .published_at
            .is_none());
    }

    #[test]
    fn rejects_unsafe_links_and_invalid_xml_responses() {
        for url in [
            "http://agenciabrasil.ebc.com.br/economia/um",
            "javascript:alert(1)",
            "https://agenciabrasil.ebc.com.br.evil.test/economia/um",
            "https://agenciabrasil.ebc.com.br@evil.test/economia/um",
            "https://user:password@agenciabrasil.ebc.com.br/economia/um",
            "https://agenciabrasil.ebc.com.br:444/economia/um",
        ] {
            assert!(parse_feed(&feed(&item("Título", url, "")), fetch_time()).is_err());
        }
        for body in [
            b"<html><body>Unavailable</body></html>".as_slice(),
            b"<rss><channel><item>".as_slice(),
            b"<rss/>".as_slice(),
            b"<rss><channel/></rss>".as_slice(),
            b"<!DOCTYPE rss [<!ENTITY private SYSTEM 'file:///private'>]><rss><channel>&private;</channel></rss>".as_slice(),
        ] {
            assert!(parse_feed(body, fetch_time()).is_err());
        }
        assert!(parse_feed(&vec![b'x'; MAX_RESPONSE_BYTES + 1], fetch_time()).is_err());
    }

    #[test]
    fn bounds_unicode_titles_and_number_of_items() {
        let content: String = (0..25)
            .map(|index| {
                item(
                    &"ç".repeat(300),
                    &format!("https://agenciabrasil.ebc.com.br/economia/{index}"),
                    "",
                )
            })
            .collect();
        let snapshot = parse_feed(&feed(&content), fetch_time()).unwrap();
        assert_eq!(snapshot.items.len(), MAX_ITEMS);
        assert_eq!(snapshot.items[0].title.chars().count(), MAX_TITLE_CHARS);
    }

    #[test]
    fn malformed_refresh_preserves_original_cache_and_reference_dates() {
        let (service, directory) = service();
        assert!(service.read_cache().unwrap().is_none());
        let snapshot = service
            .cache_response(&feed(&item(
                "Notícia",
                "https://agenciabrasil.ebc.com.br/economia/um",
                "",
            )))
            .unwrap();
        let original = fs::read(&service.cache_path).unwrap();
        assert!(service.cache_response(b"unavailable").is_err());
        assert_eq!(fs::read(&service.cache_path).unwrap(), original);
        assert_eq!(service.read_cache().unwrap().unwrap(), snapshot);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn prevents_older_concurrent_refresh_from_replacing_newer_cache() {
        let (service, directory) = service();
        let body = feed(&item(
            "Notícia",
            "https://agenciabrasil.ebc.com.br/economia/um",
            "",
        ));
        let newer = parse_feed(&body, "2026-10-16T07:00:00Z".into()).unwrap();
        let older = parse_feed(&body, fetch_time()).unwrap();
        service.write_cache(newer.clone()).unwrap();
        assert_eq!(service.write_cache(older).unwrap(), newer);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_corrupted_or_oversized_cache() {
        let (service, directory) = service();
        fs::create_dir_all(&directory).unwrap();
        fs::write(&service.cache_path, "{broken").unwrap();
        assert!(service.read_cache().is_err());
        fs::write(&service.cache_path, vec![b'x'; MAX_RESPONSE_BYTES + 1]).unwrap();
        assert!(service.read_cache().is_err());
        service
            .cache_response(&feed(&item(
                "Notícia recuperada",
                "https://agenciabrasil.ebc.com.br/economia/recuperada",
                "",
            )))
            .unwrap();
        assert_eq!(
            service.read_cache().unwrap().unwrap().items[0].title,
            "Notícia recuperada"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    #[ignore = "Manual public RSS smoke test; requires network and uses only an isolated temp cache"]
    async fn live_public_feed_refreshes_and_round_trips_cache() {
        let (service, directory) = service();
        let snapshot = service.refresh().await.unwrap();
        assert!(!snapshot.items.is_empty());
        assert!(snapshot.items.len() <= MAX_ITEMS);
        assert_eq!(service.read_cache().unwrap().unwrap(), snapshot);
        println!(
            "Public RSS verified: {} headlines; fetched at {}",
            snapshot.items.len(),
            snapshot.fetched_at
        );
        fs::remove_dir_all(directory).unwrap();
    }
}

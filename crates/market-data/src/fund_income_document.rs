//! Exact cash distributions declared in public Fundos.NET structured documents.
//! The caller selects the active document/version from the official listing.
//! No amount is inferred from prices, yields, missing rows, or another month.

use std::collections::BTreeSet;
use std::str::FromStr;

use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, NaiveDate};
use roxmltree::Node;
use rust_decimal::Decimal;

use crate::{cvm::normalize_cnpj, dividends::DividendEvent, quotes::normalize_symbol};

const MAX_DOCUMENT: usize = 2 * 1024 * 1024;
const MAX_TABLES: usize = 128;

/// Parses the official transformed HTML, or the original XML/base64 download.
/// Missing dates remain incomplete; malformed dates and contradictory records
/// reject the document. Subscription receipts and capital refunds never return
/// as ordinary-share income. Listing status/version validation belongs to the
/// service, since those fields are not part of the structured document itself.
pub fn parse_fund_income_document(
    bytes: &[u8],
    symbol: &str,
    cnpj: &str,
    document_id: u64,
    fetched_at: &str,
) -> Result<Vec<DividendEvent>, String> {
    let symbol = normalize_symbol(symbol)?;
    if symbol.len() != 6 || !symbol.ends_with("11") || document_id == 0 {
        return Err("Documento de proventos sem identidade ordinária válida.".into());
    }
    let cnpj = normalize_cnpj(cnpj)?;
    DateTime::parse_from_rfc3339(fetched_at)
        .map_err(|_| "Data de coleta do documento de proventos inválida.")?;
    if bytes.len() > MAX_DOCUMENT {
        return Err("Documento de proventos excedeu 2 MiB.".into());
    }
    let decoded = decode_document(bytes)?;
    let raw = std::str::from_utf8(&decoded)
        .map_err(|_| "Documento de proventos não está em UTF-8.")?
        .trim_start_matches('\u{feff}')
        .trim();
    let payload = if raw.to_ascii_lowercase().contains("<html") {
        parse_html(raw, &symbol, &cnpj)?
    } else {
        parse_xml(raw, &symbol, &cnpj)?
    };
    let Some(payload) = payload else {
        return Ok(Vec::new());
    };
    let per_share = decimal(&payload.amount)?;
    let record_date = date(payload.record.as_deref())?;
    let payment_date = date(payload.payment.as_deref())?;
    let declared_date = date(payload.declared.as_deref())?;
    if let (Some(record), Some(payment)) = (&record_date, &payment_date) {
        if payment < record {
            return Err("O pagamento do provento precede a data-base no documento.".into());
        }
    }
    let precision = if record_date.is_some() && payment_date.is_some() {
        "exact"
    } else {
        "incomplete"
    };
    Ok(vec![DividendEvent {
        // A document has one ordinary-share income declaration; its rate and
        // dates are deliberately absent from the identity so revisions conflict.
        event_id: format!("fundosnet:{document_id}:{symbol}:income"),
        symbol,
        per_share,
        kind: "income".into(),
        label: "Rendimento".into(),
        payment_date,
        record_date,
        ex_date: None,
        declared_date,
        date_precision: precision.into(),
        verified: Some(true),
        remarks: None,
        related_to: Some(format!("Fundos.NET documento {document_id}")),
        isin_code: Some(payload.isin),
        source_url: format!(
            "https://fnet.bmfbovespa.com.br/fnet/publico/exibirDocumento?id={document_id}"
        ),
        last_observed_at: Some(fetched_at.into()),
        reconciliation_pending: false,
    }])
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Payload {
    isin: String,
    amount: String,
    record: Option<String>,
    payment: Option<String>,
    declared: Option<String>,
}

fn decode_document(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let raw = std::str::from_utf8(bytes)
        .map_err(|_| "Documento de proventos não está em UTF-8.")?
        .trim_start_matches('\u{feff}')
        .trim();
    if raw.starts_with('<') {
        return Ok(raw.as_bytes().to_vec());
    }
    let encoded = if raw.starts_with('"') {
        serde_json::from_str::<String>(raw)
            .map_err(|_| "Envelope do documento de proventos inválido.")?
    } else {
        raw.into()
    };
    let decoded = STANDARD
        .decode(encoded.as_bytes())
        .map_err(|_| "Documento de proventos não contém XML/HTML reconhecido.")?;
    if decoded.len() > MAX_DOCUMENT || !decoded.starts_with(b"<") {
        return Err("Documento de proventos decodificado inválido.".into());
    }
    Ok(decoded)
}

fn ordinary_isin(symbol: &str, isin: &str) -> bool {
    isin.len() == 12
        && isin.starts_with(&format!("BR{}CTF", &symbol[..4]))
        && isin.as_bytes()[9..].iter().all(u8::is_ascii_digit)
}

fn subscription_receipt(symbol: &str, candidate: &str, isin: &str) -> bool {
    // Receipt classes have their own ticker and ISIN, as in the official
    // multi-class XSL. Their cash must never be applied to ordinary holdings.
    candidate.len() == 6
        && candidate.starts_with(&symbol[..4])
        && candidate.as_bytes()[4..].iter().all(u8::is_ascii_digit)
        && candidate[4..].parse::<u8>().is_ok_and(|suffix| suffix > 11)
        && isin.len() == 12
        && isin.starts_with(&format!("BR{}R", &symbol[..4]))
        && isin.as_bytes()[7..9].iter().all(u8::is_ascii_digit)
        && isin.as_bytes()[9] == b'M'
        && isin.as_bytes()[10..].iter().all(u8::is_ascii_digit)
}

fn decimal(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty()
        || raw.len() > 64
        || raw.contains(['e', 'E', '+', '-'])
        || raw.matches(',').count() + raw.matches('.').count() > 1
        || !raw
            .chars()
            .all(|c| c.is_ascii_digit() || c == ',' || c == '.')
    {
        return Err("Valor por cota do documento de proventos inválido.".into());
    }
    let value = Decimal::from_str(&raw.replace(',', "."))
        .map_err(|_| "Valor por cota do documento de proventos inválido.")?;
    if value <= Decimal::ZERO || value > Decimal::from(1_000_000u32) {
        return Err("Valor por cota do documento de proventos deve ser positivo.".into());
    }
    Ok(value.normalize().to_string())
}

fn date(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let format = if raw.contains('/') {
        "%d/%m/%Y"
    } else {
        "%Y-%m-%d"
    };
    let value = NaiveDate::parse_from_str(raw, format)
        .map_err(|_| "Data do documento de proventos inválida.")?;
    if raw.len() != 10 || value.format(format).to_string() != raw {
        return Err("Data do documento de proventos ambígua.".into());
    }
    Ok(Some(value.format("%Y-%m-%d").to_string()))
}

fn key(raw: &str) -> String {
    raw.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            _ => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(':')
        .trim()
        .to_owned()
}

fn text(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn singleton(existing: &mut Option<String>, value: String) -> Result<(), String> {
    if existing.as_ref().is_some_and(|v| v != &value) {
        return Err("Documento de proventos contém dados contraditórios.".into());
    }
    *existing = Some(value);
    Ok(())
}

fn choose(existing: &mut Option<Payload>, value: Payload) -> Result<(), String> {
    if existing.as_ref().is_some_and(|v| v != &value) {
        return Err("Documento contém mais de um rendimento ordinário incompatível.".into());
    }
    *existing = Some(value);
    Ok(())
}

fn parse_html(raw: &str, symbol: &str, cnpj: &str) -> Result<Option<Payload>, String> {
    // The official XSL page uses HTML4 void elements outside the tables. Parse
    // only its bounded table fragments as XML, retaining empty cells/colspans;
    // scripts and display text never supply monetary evidence.
    let lower = raw.to_ascii_lowercase();
    let mut tables = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = lower[cursor..].find("<table") {
        let start = cursor + offset;
        let end = lower[start..]
            .find("</table>")
            .map(|n| start + n + 8)
            .ok_or("Tabela de proventos incompleta.")?;
        if lower[start + 6..end - 8].contains("<table") || tables.len() >= MAX_TABLES {
            return Err("Estrutura de tabelas de proventos não reconhecida.".into());
        }
        tables.push(html_entities(&raw[start..end])?);
        cursor = end;
    }
    if tables.is_empty() {
        return Err("Página não contém o informe estruturado de proventos.".into());
    }
    let mut declared = None;
    let mut actual_cnpjs = BTreeSet::new();
    let mut payload = None;
    let mut matched_symbol = false;
    let mut receipt_only = true;
    let mut receipts = 0;
    for table in &tables {
        let doc = roxmltree::Document::parse(table)
            .map_err(|_| "Tabela estruturada de proventos inválida.")?;
        let rows: Vec<Vec<String>> = doc
            .descendants()
            .filter(|n| n.has_tag_name("tr"))
            .map(|row| {
                row.children()
                    .filter(|n| n.has_tag_name("td") || n.has_tag_name("th"))
                    .map(text)
                    .collect()
            })
            .collect();
        if rows.len() > 1000 || rows.iter().flatten().any(|c| c.len() > 16_384) {
            return Err("Tabela de proventos excedeu o limite seguro.".into());
        }
        for row in &rows {
            for pair in row.windows(2) {
                match key(&pair[0]).as_str() {
                    "cnpj do fundo" | "cnpj da classe" => {
                        actual_cnpjs.insert(normalize_cnpj(&pair[1])?);
                    }
                    "data da informacao" => singleton(&mut declared, pair[1].clone())?,
                    _ => {}
                }
            }
        }
        let Some(header) = rows.iter().find(|row| {
            row.len() == 6
                && key(&row[0]) == "codigo isin"
                && key(&row[2]) == "codigo de negociacao"
        }) else {
            continue;
        };
        if header[3] != symbol {
            if subscription_receipt(symbol, &header[3], &header[1]) {
                receipts += 1;
            } else {
                receipt_only = false;
            }
            continue;
        }
        matched_symbol = true;
        if !ordinary_isin(symbol, &header[1])
            || key(&header[4]) != "rendimento"
            || key(&header[5]) != "amortizacao"
        {
            return Err(
                "Identidade ou colunas de proventos incompatíveis com a cota ordinária.".into(),
            );
        }
        let mut amount = None;
        let mut record = None;
        let mut payment = None;
        let mut amortization = None;
        for row in &rows {
            if row.len() < 2 || row.len() > 3 {
                continue;
            }
            let label = key(&row[0]);
            let target = if label.starts_with("valor do provento") {
                if let Some(value) = row.get(2) {
                    singleton(&mut amortization, value.clone())?;
                }
                Some(&mut amount)
            } else if label.starts_with("data-base") {
                Some(&mut record)
            } else if label == "data do pagamento" {
                Some(&mut payment)
            } else {
                None
            };
            if let Some(target) = target {
                singleton(target, row[1].clone())?;
            }
        }
        // Blank income with a populated amortization column is a capital refund.
        // Its absence is not converted to a zero cash distribution.
        if let Some(amount) = amount.filter(|s| !s.trim().is_empty()) {
            choose(
                &mut payload,
                Payload {
                    isin: header[1].clone(),
                    amount,
                    record,
                    payment,
                    declared: None,
                },
            )?;
        } else if let Some(value) = amortization.filter(|s| !s.trim().is_empty()) {
            decimal(&value)?;
        } else {
            return Err(
                "Documento não informa valor de rendimento nem amortização para a cota solicitada."
                    .into(),
            );
        }
    }
    if actual_cnpjs.len() != 1 || !actual_cnpjs.contains(cnpj) {
        return Err("O CNPJ do documento não corresponde ao fundo solicitado.".into());
    }
    if !matched_symbol {
        if receipt_only && receipts > 0 {
            // This says that this verified filing concerns receipts only. It
            // says nothing about ordinary income in the month or other filings.
            return Ok(None);
        }
        return Err("O documento não contém a cota ordinária solicitada.".into());
    }
    if let Some(value) = &mut payload {
        // The document's declared information date is independent of payment
        // and data-com. "Ato societário" is free text, never treated as a date.
        if value.declared.is_none() {
            value.declared = declared;
        }
    }
    Ok(payload)
}

fn parse_xml(raw: &str, symbol: &str, cnpj: &str) -> Result<Option<Payload>, String> {
    // This is the original single-class structured schema, not a heuristic
    // search over arbitrary XML/PDF text. Unknown layouts must remain pending.
    let doc =
        roxmltree::Document::parse(raw).map_err(|_| "XML do informe de proventos inválido.")?;
    let root = doc.root_element();
    if root.tag_name().name() != "DadosEconomicoFinanceiros" {
        return Err("Tipo de documento de proventos não reconhecido.".into());
    }
    let general =
        only_child(root, "DadosGerais")?.ok_or("XML de proventos sem identidade do fundo.")?;
    let actual = child_text(general, "CNPJFundo")?.ok_or("XML de proventos sem CNPJ do fundo.")?;
    if normalize_cnpj(&actual)? != cnpj {
        return Err("O CNPJ do documento não corresponde ao fundo solicitado.".into());
    }
    let actual_symbol =
        child_text(general, "CodNegociacaoCota")?.ok_or("XML de proventos sem código da cota.")?;
    let isin = child_text(general, "CodISINCota")?.ok_or("XML de proventos sem ISIN da cota.")?;
    if subscription_receipt(symbol, &actual_symbol, &isin) {
        return Ok(None);
    }
    if actual_symbol != symbol || !ordinary_isin(symbol, &isin) {
        return Err("O XML não corresponde à cota ordinária solicitada.".into());
    }
    let income = only_child(root, "InformeRendimentos")?
        .ok_or("XML sem informe de rendimentos estruturado.")?;
    let record = only_child(income, "Rendimento")?;
    let amount = record
        .map(|record| child_text(record, "ValorProventoCota"))
        .transpose()?
        .flatten()
        .unwrap_or_default();
    if amount.is_empty() {
        let amortization = only_child(income, "Amortizacao")?
            .map(|value| child_text(value, "ValorProventoCota"))
            .transpose()?
            .flatten();
        if let Some(value) = amortization.filter(|value| !value.is_empty()) {
            decimal(&value)?;
            return Ok(None);
        }
        return Err(
            "XML não informa valor de rendimento nem amortização para a cota solicitada.".into(),
        );
    }
    let record = record.ok_or("XML sem rendimento estruturado.")?;
    Ok(Some(Payload {
        isin,
        amount,
        record: child_text(record, "DataBase")?,
        payment: child_text(record, "DataPagamento")?,
        declared: child_text(record, "DataAprovacao")?,
    }))
}

fn only_child<'a, 'input>(
    node: Node<'a, 'input>,
    name: &str,
) -> Result<Option<Node<'a, 'input>>, String> {
    let mut children = node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name);
    let value = children.next();
    if children.next().is_some() {
        return Err("XML de proventos contém elementos duplicados ou ambíguos.".into());
    }
    Ok(value)
}

fn child_text(node: Node<'_, '_>, name: &str) -> Result<Option<String>, String> {
    only_child(node, name).map(|n| n.map(text))
}

fn html_entities(raw: &str) -> Result<String, String> {
    let mut result = String::with_capacity(raw.len());
    let mut cursor = 0;
    while let Some(offset) = raw[cursor..].find('&') {
        let start = cursor + offset;
        result.push_str(&raw[cursor..start]);
        let end = raw[start..]
            .find(';')
            .map(|n| start + n)
            .ok_or("Entidade HTML de proventos incompleta.")?;
        let entity = &raw[start + 1..end];
        let replacement = match entity {
            "nbsp" => Some(' '),
            "ldquo" | "rdquo" => Some('"'),
            "lsquo" | "rsquo" => Some('\''),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "ordm" => Some('º'),
            "ordf" => Some('ª'),
            "Agrave" => Some('À'),
            "Aacute" => Some('Á'),
            "Acirc" => Some('Â'),
            "Atilde" => Some('Ã'),
            "Auml" => Some('Ä'),
            "agrave" => Some('à'),
            "aacute" => Some('á'),
            "acirc" => Some('â'),
            "atilde" => Some('ã'),
            "auml" => Some('ä'),
            "Ccedil" => Some('Ç'),
            "ccedil" => Some('ç'),
            "Egrave" => Some('È'),
            "Eacute" => Some('É'),
            "Ecirc" => Some('Ê'),
            "Euml" => Some('Ë'),
            "egrave" => Some('è'),
            "eacute" => Some('é'),
            "ecirc" => Some('ê'),
            "euml" => Some('ë'),
            "Igrave" => Some('Ì'),
            "Iacute" => Some('Í'),
            "Icirc" => Some('Î'),
            "Iuml" => Some('Ï'),
            "igrave" => Some('ì'),
            "iacute" => Some('í'),
            "icirc" => Some('î'),
            "iuml" => Some('ï'),
            "Ograve" => Some('Ò'),
            "Oacute" => Some('Ó'),
            "Ocirc" => Some('Ô'),
            "Otilde" => Some('Õ'),
            "Ouml" => Some('Ö'),
            "ograve" => Some('ò'),
            "oacute" => Some('ó'),
            "ocirc" => Some('ô'),
            "otilde" => Some('õ'),
            "ouml" => Some('ö'),
            "Ugrave" => Some('Ù'),
            "Uacute" => Some('Ú'),
            "Ucirc" => Some('Û'),
            "Uuml" => Some('Ü'),
            "ugrave" => Some('ù'),
            "uacute" => Some('ú'),
            "ucirc" => Some('û'),
            "uuml" => Some('ü'),
            // XML handles its five standard entities and numeric code points.
            "amp" | "lt" | "gt" | "quot" | "apos" => None,
            value if value.starts_with('#') => None,
            _ => return Err("Entidade HTML de proventos não reconhecida.".into()),
        };
        if let Some(c) = replacement {
            // Quote entities may occur in text or an HTML attribute value.
            match c {
                '"' => result.push_str("&#34;"),
                '\'' => result.push_str("&#39;"),
                _ => result.push(c),
            }
        } else {
            result.push_str(&raw[start..=end]);
        }
        cursor = end + 1;
    }
    result.push_str(&raw[cursor..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CNPJ: &str = "00332266000131";
    const FETCHED: &str = "2026-10-21T12:00:00Z";

    fn header() -> String {
        "<table><tr><td>Nome do Fundo:</td><td>Fundo de exemplo</td><td>CNPJ do Fundo:</td><td>00.332.266/0001-31</td></tr><tr><td>Data da Informa&ccedil;&atilde;o:</td><td>30/04/2024</td><td>Ano:</td><td>2024</td></tr></table>".into()
    }

    fn table(symbol: &str, isin: &str, value: &str, record: &str, payment: &str) -> String {
        format!("<table><tr><td>C&oacute;digo ISIN:</td><td>{isin}</td><td>C&oacute;digo de negocia&ccedil;&atilde;o:</td><td>{symbol}</td><td>Rendimento</td><td>Amortiza&ccedil;&atilde;o</td></tr><tr><td colspan=\"4\">Ato societ&aacute;rio de aprova&ccedil;&atilde;o (se houver)</td><td></td><td></td></tr><tr><td colspan=\"4\">Data-base (&uacute;ltimo dia de negocia&ccedil;&atilde;o &ldquo;com&rdquo; direito ao provento)</td><td>{record}</td><td></td></tr><tr><td colspan=\"4\">Valor do provento (R$/unidade)</td><td>{value}</td><td></td></tr><tr><td colspan=\"4\">Data do pagamento</td><td>{payment}</td><td></td></tr></table>")
    }

    fn html(tables: &str) -> String {
        format!("<html><head><META charset=\"UTF-8\"><script>irrelevant</script></head><body>{}{tables}</body></html>", header())
    }

    fn parse(raw: &str) -> Result<Vec<DividendEvent>, String> {
        parse_fund_income_document(raw.as_bytes(), "ABCD11", CNPJ, 123, FETCHED)
    }

    fn sample() -> String {
        html(&table(
            "ABCD11",
            "BRABCDCTF001",
            "0,123456789",
            "30/04/2024",
            "15/05/2024",
        ))
    }

    #[test]
    fn html_preserves_exact_cash_dates_and_provenance_without_using_reference_month() {
        let result = parse(&sample()).unwrap();
        assert_eq!(result.len(), 1);
        let event = &result[0];
        assert_eq!(event.per_share, "0.123456789");
        assert_eq!(event.record_date.as_deref(), Some("2024-04-30"));
        assert_eq!(event.payment_date.as_deref(), Some("2024-05-15"));
        assert_eq!(event.declared_date.as_deref(), Some("2024-04-30"));
        assert_eq!(event.event_id, "fundosnet:123:ABCD11:income");
        assert_eq!(
            event.source_url,
            "https://fnet.bmfbovespa.com.br/fnet/publico/exibirDocumento?id=123"
        );
        assert_eq!(event.last_observed_at.as_deref(), Some(FETCHED));
    }

    #[test]
    fn ordinary_identity_excludes_subscription_receipts_from_the_same_document() {
        let raw = html(&format!(
            "{}{}{}",
            table(
                "ABCD13",
                "BRABCDR16M16",
                "0,987654321",
                "30/04/2024",
                "15/05/2024"
            ),
            table("ABCD11", "BRABCDCTF001", "0,12", "30/04/2024", "15/05/2024"),
            table("ABCD15", "BRABCDR17M15", "0,01", "30/04/2024", "15/05/2024")
        ));
        let result = parse(&raw).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].per_share, "0.12");
        assert_eq!(result[0].isin_code.as_deref(), Some("BRABCDCTF001"));
        assert!(parse(&sample().replace("BRABCDCTF001", "BRABCDR16M16")).is_err());
        assert!(parse(&sample().replace("ABCD11", "WXYZ11")).is_err());
        assert!(parse(&sample().replace("00.332.266/0001-31", "12.345.678/0001-95")).is_err());
    }

    #[test]
    fn verified_receipt_only_filing_is_checked_without_claiming_zero_ordinary_income() {
        let receipts = html(&format!(
            "{}{}",
            table(
                "ABCD13",
                "BRABCDR16M16",
                "0,987654321",
                "30/04/2024",
                "15/05/2024"
            ),
            table("ABCD15", "BRABCDR17M15", "0,01", "30/04/2024", "15/05/2024")
        ));
        assert!(parse(&receipts).unwrap().is_empty());
        assert!(parse(&receipts.replace("00.332.266/0001-31", "12.345.678/0001-95")).is_err());
        assert!(parse(&receipts.replace("ABCD15", "WXYZ15")).is_err());
        assert!(parse(&receipts.replace("BRABCDR16M16", "BRWXYZR16M16")).is_err());
        assert!(parse(&receipts.replace("BRABCDR16M16", "BRABCDCTF001")).is_err());
        assert!(parse(&html("")).is_err());
    }

    #[test]
    fn blank_or_missing_dates_stay_incomplete_and_invalid_dates_reject() {
        let raw = html(&table("ABCD11", "BRABCDCTF001", "0,12", "", "15/05/2024"));
        let event = &parse(&raw).unwrap()[0];
        assert_eq!(event.record_date, None);
        assert_eq!(event.date_precision, "incomplete");
        assert_eq!(event.payment_date.as_deref(), Some("2024-05-15"));
        for invalid in ["31/04/2024", "3/04/2024", "2024-4-30"] {
            assert!(parse(&sample().replace("30/04/2024", invalid)).is_err());
        }
        assert!(parse(&sample().replace("15/05/2024", "15/04/2024")).is_err());
    }

    #[test]
    fn refunds_and_empty_income_are_never_promoted_to_cash_income() {
        let raw = html(
            &table("ABCD11", "BRABCDCTF001", "", "30/04/2024", "15/05/2024").replace(
                "<td></td><td></td></tr><tr><td colspan=\"4\">Data do pagamento",
                "<td></td><td>1,23</td></tr><tr><td colspan=\"4\">Data do pagamento",
            ),
        );
        assert!(parse(&raw).unwrap().is_empty());
        assert!(parse(&sample().replace("Rendimento</td>", "Outro tipo</td>")).is_err());
    }

    #[test]
    fn exact_duplicates_collapse_but_conflicting_amount_or_dates_reject() {
        let ordinary = table("ABCD11", "BRABCDCTF001", "0,12", "30/04/2024", "15/05/2024");
        assert_eq!(
            parse(&html(&format!("{ordinary}{ordinary}")))
                .unwrap()
                .len(),
            1
        );
        for changed in [
            ordinary.replace("0,12", "0,13"),
            ordinary.replace("15/05/2024", "16/05/2024"),
        ] {
            assert!(parse(&html(&format!("{ordinary}{changed}"))).is_err());
        }
    }

    #[test]
    fn unsafe_or_unrecognized_values_and_layouts_fail_closed() {
        for value in ["0", "-0.1", "NaN", "1e-1", "1.234,56", "1000001", ""] {
            let raw = sample().replace("0,123456789", value);
            assert!(parse(&raw).is_err(), "{value}");
        }
        assert!(parse("<html>access denied</html>").is_err());
        assert!(parse(&sample().replace("&ldquo;", "&unknown;")).is_err());
        assert!(parse(&sample().replace("<table>", "<table><table>")).is_err());
        assert!(parse_fund_income_document(
            &vec![b' '; MAX_DOCUMENT + 1],
            "ABCD11",
            CNPJ,
            123,
            FETCHED
        )
        .is_err());
    }

    fn xml() -> String {
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><DadosEconomicoFinanceiros><DadosGerais><CNPJFundo>{CNPJ}</CNPJFundo><CodISINCota>BRABCDCTF001</CodISINCota><CodNegociacaoCota>ABCD11</CodNegociacaoCota></DadosGerais><InformeRendimentos><Rendimento><DataAprovacao>2024-04-30</DataAprovacao><DataBase>2024-04-30</DataBase><DataPagamento>2024-05-15</DataPagamento><ValorProventoCota>0.123456789</ValorProventoCota></Rendimento><Amortizacao tipo=\"\"/></InformeRendimentos></DadosEconomicoFinanceiros>")
    }

    #[test]
    fn original_xml_and_json_base64_download_keep_exact_same_event() {
        let xml = xml();
        let direct = parse(&xml).unwrap();
        let encoded = serde_json::to_string(&STANDARD.encode(&xml)).unwrap();
        assert_eq!(parse(&encoded).unwrap(), direct);
        assert_eq!(direct[0].per_share, "0.123456789");
        assert_eq!(direct[0].date_precision, "exact");
        assert!(parse(&xml.replace("CNPJFundo", "UnknownCNPJ")).is_err());
        assert!(parse(&xml.replace("ABCD11", "ABCD13")).is_err());
        let receipt = xml
            .replace("ABCD11", "ABCD13")
            .replace("BRABCDCTF001", "BRABCDR16M16");
        assert!(parse(&receipt).unwrap().is_empty());
    }

    #[test]
    fn xml_duplicate_identity_and_unsupported_multiclass_layouts_are_rejected() {
        let xml = xml();
        assert!(parse(&xml.replace(
            "</CNPJFundo>",
            "</CNPJFundo><CNPJFundo>00332266000131</CNPJFundo>"
        ))
        .is_err());
        assert!(parse(&xml.replace("InformeRendimentos", "UnknownIncomeLayout")).is_err());
        assert!(parse("<!DOCTYPE x [<!ENTITY x SYSTEM 'file:///untrusted'>]><x>&x;</x>").is_err());
    }

    #[test]
    #[ignore = "Manual check against an already-downloaded public Fundos.NET document"]
    fn public_structured_sample_preserves_the_declared_income() {
        let path = std::env::var("IVESTFII_INCOME_SAMPLE_PATH").expect("sample path");
        let symbol = std::env::var("IVESTFII_INCOME_SAMPLE_SYMBOL").expect("sample symbol");
        let cnpj = std::env::var("IVESTFII_INCOME_SAMPLE_CNPJ").expect("sample CNPJ");
        let document_id = std::env::var("IVESTFII_INCOME_SAMPLE_ID")
            .expect("sample document ID")
            .parse()
            .unwrap();
        let expected =
            std::env::var("IVESTFII_INCOME_SAMPLE_AMOUNT").expect("sample cash per share");
        let result = parse_fund_income_document(
            &std::fs::read(path).unwrap(),
            &symbol,
            &cnpj,
            document_id,
            FETCHED,
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].per_share, decimal(&expected).unwrap());
        assert_eq!(result[0].symbol, symbol);
        assert_eq!(result[0].kind, "income");
        assert_eq!(result[0].date_precision, "exact");
    }
}

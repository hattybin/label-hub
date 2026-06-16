//! Optional D365 F&O OData client using Entra ID app (client-credential) auth.
//! Provides PO lookups, product receipt queries, product descriptions, entity
//! auto-discovery, and field-inspection helpers — ported from the Node
//! prototype's d365Client.js.

use std::collections::HashMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{Duration, OffsetDateTime};

use crate::state::AppState;

// ── Entity names and fallback probe lists ────────────────────────────────────

const PO_HEADERS: &str = "PurchaseOrderHeadersV2";
const PO_LINES: &str = "PurchaseOrderLinesV2";

const RECEIPT_HEADER_CANDIDATES: &[&str] = &[
    "ProductReceiptHeaderV2",
    "ProductReceiptHeader",
    "VendorPackingSlipHeadersV2",
    "VendorPackingSlipHeaders",
    "ProductReceiptHeaders",
    "PurchPackingSlipJourEntity",
];

const RECEIPT_LINE_CANDIDATES: &[&str] = &[
    "ProductReceiptLine",
    "ProductReceiptLinesV2",
    "VendorPackingSlipLinesV2",
    "VendorPackingSlipLines",
    "ProductReceiptLines",
    "PurchPackingSlipTransEntity",
];

const DATE_FIELD_CANDIDATES: &[&str] = &[
    "ProductReceiptDate",
    "TransactionDate",
    "PackingSlipDate",
    "DeliveryDate",
    "PostingDate",
    "DocumentDate",
    "ReceiptDate",
];

// ── Wire types ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PoWithLines {
    pub header: Value,
    pub lines: Vec<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReceiptWithLines {
    pub header: Value,
    pub lines: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductDesc {
    pub desc: String,
    pub search_name: String,
    pub product_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InspectResult {
    pub entity: Option<String>,
    pub fields: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

// ── Token acquisition ────────────────────────────────────────────────────────

/// Acquire (and cache) an OData bearer token via the client-credentials flow.
pub async fn get_token(state: &AppState) -> Result<String, String> {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    {
        let cache = state.d365_token.lock().await;
        if let Some((tok, exp)) = cache.as_ref() {
            if now < exp - 300 {
                return Ok(tok.clone());
            }
        }
    }

    let cfg = &state.config;
    let tenant = cfg.azure_tenant_id.as_deref().ok_or("AZURE_TENANT_ID not set")?;
    let client_id = cfg.azure_client_id.as_deref().ok_or("AZURE_CLIENT_ID not set")?;
    let secret = cfg.azure_client_secret.as_deref().ok_or("AZURE_CLIENT_SECRET not set")?;
    let base = cfg.d365_base_url.as_deref().ok_or("D365_BASE_URL not set")?;

    let url = format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token");
    let scope = format!("{base}/.default");
    let params = [
        ("grant_type", "client_credentials"),
        ("client_id", client_id),
        ("client_secret", secret),
        ("scope", scope.as_str()),
    ];

    let resp = state
        .http
        .post(&url)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("token request failed: {e}"))?;

    if !resp.status().is_success() {
        let code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("token request {code}: {body}"));
    }

    let tr: TokenResponse =
        resp.json().await.map_err(|e| format!("could not parse token response: {e}"))?;

    let expires_at = now + if tr.expires_in > 0 { tr.expires_in } else { 3600 };
    *state.d365_token.lock().await = Some((tr.access_token.clone(), expires_at));
    Ok(tr.access_token)
}

// ── Core OData fetch helpers ─────────────────────────────────────────────────

/// OData GET returning parsed JSON. `query` is a pre-built query string
/// (e.g. `"$filter=...&$top=50&cross-company=true"`).
pub async fn odata_get(state: &AppState, entity: &str, query: &str) -> Result<Value, String> {
    let (status, body) = odata_raw(state, entity, query).await?;
    if status < 400 {
        serde_json::from_str(&body).map_err(|e| format!("parse error on {entity}: {e}"))
    } else {
        Err(format!("OData [{entity}] HTTP {status}: {body}"))
    }
}

/// OData GET returning the raw (status, body) — used for probing where we need
/// to inspect error messages rather than failing immediately.
async fn odata_raw(state: &AppState, entity: &str, query: &str) -> Result<(u16, String), String> {
    let token = get_token(state).await?;
    let base = state.config.d365_base_url.as_deref().ok_or("D365_BASE_URL not set")?;
    let sep = if query.is_empty() { "" } else { "?" };
    let url = format!("{base}/data/{entity}{sep}{query}");

    let resp = state
        .http
        .get(&url)
        .bearer_auth(token)
        .header("Accept", "application/json")
        .header("OData-MaxVersion", "4.0")
        .header("OData-Version", "4.0")
        .send()
        .await
        .map_err(|e| format!("OData request failed: {e}"))?;

    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    Ok((status, body))
}

// ── Utility ──────────────────────────────────────────────────────────────────

/// Escape a string value for use inside an OData single-quoted literal.
fn ode(s: &str) -> String {
    s.replace('\'', "''")
}

/// Percent-encode a query-parameter value (keeps OData filter syntax intact,
/// encodes spaces, single quotes, and other special chars).
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~' | b'$' | b',' => out.push(b as char),
            b' ' => out.push_str("%20"),
            b'\'' => out.push_str("%27"),
            b'=' => out.push_str("%3D"),
            _ => { let _ = write!(out, "%{:02X}", b); }
        }
    }
    out
}

/// Build `key=pct(value)&...` query string from a slice of pairs.
/// Keys are written verbatim (e.g. `$filter`) so OData params are not double-encoded.
fn qs(params: &[(&str, &str)]) -> String {
    params.iter().map(|(k, v)| format!("{}={}", k, pct(v))).collect::<Vec<_>>().join("&")
}

fn is_routing_error(body: &str) -> bool {
    body.contains("No route data was found") || body.contains("No HTTP resource was found")
}

fn is_property_error(body: &str) -> bool {
    body.to_ascii_lowercase().contains("could not find a property")
}

/// Pull the `value` array out of an OData response.
fn odata_array(v: Value) -> Vec<Value> {
    v.get("value").and_then(|a| a.as_array()).cloned().unwrap_or_default()
}

/// Strip OData metadata fields (keys starting with `@`) from a JSON object.
fn strip_meta(v: Value) -> Value {
    match v {
        Value::Object(m) => {
            Value::Object(m.into_iter().filter(|(k, _)| !k.starts_with('@')).collect())
        }
        other => other,
    }
}

fn fmt_date(dt: OffsetDateTime) -> String {
    format!("{:04}-{:02}-{:02}", dt.year(), u8::from(dt.month()), dt.day())
}

// ── Entity auto-discovery ────────────────────────────────────────────────────

/// Resolve a D365 entity name for `key` ("receiptHeaders" or "receiptLines").
///
/// Priority: `.env` override → in-memory cache → probe fallback candidates.
/// Caches the result so subsequent calls are instant.
pub async fn resolve_entity(state: &AppState, key: &str) -> Result<String, String> {
    // .env pin takes absolute priority
    let pinned = match key {
        "receiptHeaders" => state.config.receipt_header_entity.clone(),
        "receiptLines" => state.config.receipt_lines_entity.clone(),
        _ => None,
    };
    if let Some(name) = pinned {
        return Ok(name);
    }

    // Cache hit
    if let Some(name) = state.entity_cache.read().await.get(key).cloned() {
        return Ok(name);
    }

    let candidates: &[&str] = match key {
        "receiptHeaders" => RECEIPT_HEADER_CANDIDATES,
        "receiptLines" => RECEIPT_LINE_CANDIDATES,
        _ => return Err(format!("unknown entity key: {key}")),
    };

    for &name in candidates {
        // Pass 1: bare probe — only a 2xx with no routing-error body counts as found.
        match odata_raw(state, name, "$top=1&cross-company=true").await {
            Ok((status, body)) if (200..300).contains(&status) => {
                if !is_routing_error(&body) {
                    tracing::info!("entity '{}' → {} [{}]", key, name, status);
                    state.entity_cache.write().await.insert(key.to_string(), name.to_string());
                    return Ok(name.to_string());
                }
                continue;
            }
            Ok((404, _)) => {
                // Pass 2: some entities return 404 on bare $top=1 but 200 with a filter.
                // Only accept 2xx to avoid caching an entity that returns 400/4xx on the probe.
                let q2 = qs(&[("$filter", "PurchaseOrderNumber ne 'ZZZNOMATCH'"), ("$top", "1"), ("cross-company", "true")]);
                if let Ok((s2, body2)) = odata_raw(state, name, &q2).await {
                    if (200..300).contains(&s2) && !is_routing_error(&body2) {
                        tracing::info!("entity '{}' → {} [{}, filtered probe]", key, name, s2);
                        state.entity_cache.write().await.insert(key.to_string(), name.to_string());
                        return Ok(name.to_string());
                    }
                }
            }
            _ => continue,
        }
    }

    Err(format!(
        "could not resolve entity '{key}'. Tried: {}. \
         Set D365_RECEIPT_HEADER_ENTITY or D365_RECEIPT_LINES_ENTITY in .env to pin it.",
        candidates.join(", ")
    ))
}

/// Resolve the date field name for a receipt entity. Returns `None` if no
/// working field can be found (dashboard falls back to unfiltered load).
pub async fn resolve_receipt_date_field(state: &AppState, entity: &str) -> Option<String> {
    if let Some(f) = &state.config.receipt_date_field {
        return Some(f.clone());
    }

    if let Some(cached) = state.date_field_cache.read().await.get(entity).cloned() {
        return cached;
    }

    let today = fmt_date(OffsetDateTime::now_utc());

    for &field in DATE_FIELD_CANDIDATES {
        let q = qs(&[
            ("$filter", &format!("{field} ge {today}")),
            ("$top", "1"),
            ("cross-company", "true"),
        ]);
        match odata_raw(state, entity, &q).await {
            Ok((status, _body)) if status < 400 => {
                tracing::info!("date field for '{}' → {}", entity, field);
                state.date_field_cache.write().await.insert(entity.to_string(), Some(field.to_string()));
                return Some(field.to_string());
            }
            Ok((_, body)) => {
                if is_property_error(&body) { continue; }
                break;
            }
            _ => break,
        }
    }

    tracing::warn!("no date field resolved for '{}'; dashboard loads without date filter", entity);
    state.date_field_cache.write().await.insert(entity.to_string(), None);
    None
}

// ── PO queries ───────────────────────────────────────────────────────────────

pub async fn get_po_with_lines(state: &AppState, po_number: &str) -> Result<Option<PoWithLines>, String> {
    let safe = ode(po_number);
    let hq = qs(&[
        ("$filter", &format!("PurchaseOrderNumber eq '{safe}'")),
        ("$top", "1"),
        ("cross-company", "true"),
    ]);
    let hv = odata_get(state, PO_HEADERS, &hq).await?;
    let header = match odata_array(hv).into_iter().next() {
        Some(h) => h,
        None => return Ok(None),
    };

    let lq = qs(&[
        ("$filter", &format!("PurchaseOrderNumber eq '{safe}'")),
        ("$orderby", "LineNumber asc"),
        ("$top", "250"),
        ("cross-company", "true"),
    ]);
    let lv = odata_get(state, PO_LINES, &lq).await?;
    Ok(Some(PoWithLines { header, lines: odata_array(lv) }))
}

pub async fn get_pos_by_vendor(state: &AppState, vendor_account: &str) -> Result<Vec<Value>, String> {
    let safe = ode(vendor_account);
    for field in &["OrderVendorAccountNumber", "VendorAccountNumber", "InvoiceVendorAccountNumber"] {
        let q = qs(&[
            ("$filter", &format!("{field} eq '{safe}'")),
            ("$orderby", "PurchaseOrderNumber desc"),
            ("$top", "50"),
            ("cross-company", "true"),
        ]);
        match odata_get(state, PO_HEADERS, &q).await {
            Ok(v) => return Ok(odata_array(v)),
            Err(e) if e.contains("Could not find a property") => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(vec![])
}

// ── Receipt queries ──────────────────────────────────────────────────────────

pub async fn get_receipts_for_po(state: &AppState, po_number: &str) -> Result<Vec<Value>, String> {
    let entity = resolve_entity(state, "receiptHeaders").await?;
    let safe = ode(po_number);
    let q = qs(&[
        ("$filter", &format!("PurchaseOrderNumber eq '{safe}'")),
        ("$top", "100"),
        ("cross-company", "true"),
    ]);
    Ok(odata_array(odata_get(state, &entity, &q).await?))
}

pub async fn get_receipt_with_lines(
    state: &AppState,
    receipt_number: &str,
) -> Result<Option<ReceiptWithLines>, String> {
    let header_entity = resolve_entity(state, "receiptHeaders").await?;
    let lines_entity = resolve_entity(state, "receiptLines").await?;
    let safe = ode(receipt_number);

    let hq = qs(&[
        ("$filter", &format!("ProductReceiptNumber eq '{safe}'")),
        ("$top", "1"),
        ("cross-company", "true"),
    ]);
    let hv = odata_get(state, &header_entity, &hq).await?;
    let header = match odata_array(hv).into_iter().next() {
        Some(h) => h,
        None => return Ok(None),
    };

    let lq = qs(&[
        ("$filter", &format!("ProductReceiptNumber eq '{safe}'")),
        ("$top", "250"),
        ("cross-company", "true"),
    ]);
    let lv = odata_get(state, &lines_entity, &lq).await?;
    Ok(Some(ReceiptWithLines { header, lines: odata_array(lv) }))
}

/// Fetch receipts within a date window. `days_back`/`days_forward` are relative
/// to today (both 0 → today only).
pub async fn get_recent_receipts(
    state: &AppState,
    days_back: i64,
    days_forward: i64,
) -> Result<Vec<Value>, String> {
    let entity = resolve_entity(state, "receiptHeaders").await?;
    let date_field = resolve_receipt_date_field(state, &entity).await;

    let now = OffsetDateTime::now_utc();
    let start = now - Duration::days(days_back.max(0));
    let end = now + Duration::days(days_forward.max(0));

    let q = match &date_field {
        Some(f) => qs(&[
            ("$filter", &format!("{f} ge {} and {f} le {}", fmt_date(start), fmt_date(end))),
            ("$top", "500"),
            ("cross-company", "true"),
        ]),
        None => qs(&[("$top", "500"), ("cross-company", "true")]),
    };

    Ok(odata_array(odata_get(state, &entity, &q).await?))
}

// ── Product descriptions ─────────────────────────────────────────────────────

/// Batch-fetch product descriptions for the given item numbers.
/// Returns a map of `item_number → ProductDesc`.
pub async fn get_product_descriptions(
    state: &AppState,
    item_numbers: &[String],
) -> Result<HashMap<String, ProductDesc>, String> {
    if item_numbers.is_empty() {
        return Ok(HashMap::new());
    }
    let mut result: HashMap<String, ProductDesc> = HashMap::new();
    const BATCH: usize = 30;

    // Step 1: ReleasedProductsV2 (fallback: ReleasedProducts)
    for chunk in item_numbers.chunks(BATCH) {
        let filter = chunk
            .iter()
            .map(|n| format!("ItemNumber eq '{}'", ode(n)))
            .collect::<Vec<_>>()
            .join(" or ");
        let q = qs(&[
            ("$filter", &filter),
            ("$top", &(chunk.len() + 5).to_string()),
            ("cross-company", "true"),
        ]);
        for entity in &["ReleasedProductsV2", "ReleasedProducts"] {
            match odata_get(state, entity, &q).await {
                Ok(v) => {
                    for p in odata_array(v) {
                        let item = str_field(&p, "ItemNumber");
                        if item.is_empty() { continue; }
                        let mfr = str_field(&p, "BPPManufacturer").trim().to_string();
                        let model = str_field(&p, "BPPModelNumber").trim().to_string();
                        let product_name = match (mfr.is_empty(), model.is_empty()) {
                            (false, false) => format!("{mfr} | {model}"),
                            (true, false) => model,
                            (false, true) => mfr,
                            (true, true) => String::new(),
                        };
                        result.insert(item, ProductDesc {
                            desc: first_str(&p, &["ProductName", "ItemName", "Name"]),
                            search_name: first_str(&p, &["SearchName", "ProductSearchName"]),
                            product_name,
                        });
                    }
                    break;
                }
                Err(_) => continue,
            }
        }
    }

    // Step 2: EcoResProductName (translated canonical product name, keyed by ProductNumber)
    for chunk in item_numbers.chunks(BATCH) {
        let filter = chunk
            .iter()
            .map(|n| format!("ProductNumber eq '{}'", ode(n)))
            .collect::<Vec<_>>()
            .join(" or ");
        let q = qs(&[
            ("$filter", &filter),
            ("$top", &(chunk.len() + 5).to_string()),
            ("cross-company", "true"),
        ]);
        if let Ok(v) = odata_get(state, "EcoResProductName", &q).await {
            for p in odata_array(v) {
                let item = first_str(&p, &["ProductNumber", "ItemNumber"]);
                let name = str_field(&p, "Name");
                if item.is_empty() || name.is_empty() { continue; }
                result
                    .entry(item.clone())
                    .and_modify(|e| e.product_name = name.clone())
                    .or_insert(ProductDesc {
                        desc: String::new(),
                        search_name: String::new(),
                        product_name: name,
                    });
            }
        }
    }

    Ok(result)
}

// ── Entity / $metadata discovery ────────────────────────────────────────────

/// Search the D365 `$metadata` document for EntitySet names containing `pattern`.
pub async fn discover_entities(state: &AppState, pattern: &str) -> Result<Vec<String>, String> {
    let token = get_token(state).await?;
    let base = state.config.d365_base_url.as_deref().ok_or("D365_BASE_URL not set")?;

    let paths = [format!("{base}/data/$metadata"), format!("{base}/$metadata")];
    let mut xml = None;
    for url in &paths {
        if let Ok(r) = state
            .http
            .get(url)
            .bearer_auth(&token)
            .header("Accept", "application/xml")
            .header("OData-MaxVersion", "4.0")
            .header("OData-Version", "4.0")
            .send()
            .await
        {
            if r.status().is_success() {
                xml = r.text().await.ok();
                break;
            }
        }
    }

    let xml = xml.ok_or_else(|| {
        format!("could not fetch $metadata from D365 (tried {} paths)", paths.len())
    })?;

    let lower = pattern.to_ascii_lowercase();
    let mut names: Vec<String> = Vec::new();
    let mut pos = 0;
    while let Some(rel) = xml[pos..].find("EntitySet") {
        let start = pos + rel;
        let rest = &xml[start..];
        if let Some(na) = rest.find("Name=\"") {
            let content = &rest[na + 6..];
            if let Some(end) = content.find('"') {
                let name = &content[..end];
                if name.to_ascii_lowercase().contains(&lower) {
                    names.push(name.to_string());
                }
            }
        }
        pos = start + 9;
    }
    names.sort();
    names.dedup();
    Ok(names)
}

// ── Inspect / field-discovery helpers ────────────────────────────────────────

/// Return all fields from one receipt header record so callers can identify
/// field names (PO number field, date field, etc.) for their D365 instance.
pub async fn inspect_receipt_header(state: &AppState) -> Result<InspectResult, String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(e) = &state.config.receipt_header_entity { candidates.push(e.clone()); }
    for &f in RECEIPT_HEADER_CANDIDATES { candidates.push(f.to_string()); }
    candidates.dedup();

    for name in candidates {
        if let Ok(v) = odata_get(state, &name, "$top=1&cross-company=true").await {
            if let Some(rec) = odata_array(v).into_iter().next() {
                return Ok(InspectResult { entity: Some(name), fields: Some(strip_meta(rec)), note: None });
            }
        }
    }
    Ok(InspectResult {
        entity: None,
        fields: None,
        note: Some("no receipt header record found; all candidates failed or are empty".to_string()),
    })
}

pub async fn inspect_receipt_line(
    state: &AppState,
    receipt_number: &str,
) -> Result<InspectResult, String> {
    let entity = resolve_entity(state, "receiptLines").await?;
    let safe = ode(receipt_number);
    let q = qs(&[
        ("$filter", &format!("ProductReceiptNumber eq '{safe}'")),
        ("$top", "1"),
        ("cross-company", "true"),
    ]);
    let v = odata_get(state, &entity, &q).await?;
    match odata_array(v).into_iter().next() {
        Some(r) => Ok(InspectResult { entity: Some(entity), fields: Some(strip_meta(r)), note: None }),
        None => Ok(InspectResult {
            entity: Some(entity),
            fields: None,
            note: Some(format!("no lines found for receipt '{receipt_number}'")),
        }),
    }
}

pub async fn inspect_po_line(state: &AppState, item_number: &str) -> Result<InspectResult, String> {
    let safe = ode(item_number);
    let q = qs(&[
        ("$filter", &format!("ItemNumber eq '{safe}'")),
        ("$top", "1"),
        ("cross-company", "true"),
    ]);
    let v = odata_get(state, PO_LINES, &q).await?;
    match odata_array(v).into_iter().next() {
        Some(r) => Ok(InspectResult { entity: Some(PO_LINES.to_string()), fields: Some(strip_meta(r)), note: None }),
        None => Ok(InspectResult {
            entity: Some(PO_LINES.to_string()),
            fields: None,
            note: Some(format!("no PO line found for item '{item_number}'")),
        }),
    }
}

pub async fn inspect_product(state: &AppState, item_number: &str) -> Result<InspectResult, String> {
    let safe = ode(item_number);
    let q = qs(&[
        ("$filter", &format!("ItemNumber eq '{safe}'")),
        ("$top", "1"),
        ("cross-company", "true"),
    ]);
    for entity in &["ReleasedProductsV2", "ReleasedProducts"] {
        if let Ok(v) = odata_get(state, entity, &q).await {
            if let Some(r) = odata_array(v).into_iter().next() {
                return Ok(InspectResult {
                    entity: Some(entity.to_string()),
                    fields: Some(strip_meta(r)),
                    note: None,
                });
            }
        }
    }
    Ok(InspectResult {
        entity: None,
        fields: None,
        note: Some(format!("item '{item_number}' not found in ReleasedProductsV2 or ReleasedProducts")),
    })
}

// ── JSON field helpers ───────────────────────────────────────────────────────

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(|f| f.as_str()).unwrap_or("").to_string()
}

fn first_str(v: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|&k| v.get(k).and_then(|f| f.as_str()).filter(|s| !s.is_empty()))
        .unwrap_or("")
        .to_string()
}

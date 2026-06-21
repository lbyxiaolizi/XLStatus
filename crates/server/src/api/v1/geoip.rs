//! GeoIP lookup API and Agent IP-change events.

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::api::v1::settings;
use crate::auth::middleware::{AuthKind, AuthSession};
use crate::db::{AgentRepository, DatabaseBackend};
use crate::notifications::sender::{
    NotificationChannel, NotificationMessage, NotificationSender, NotificationSeverity,
};
use crate::security::{secure_reqwest_client_builder, validate_outbound_url_resolved};
use axum::{
    extract::{DefaultBodyLimit, Multipart, State},
    Json,
};
use chrono::Utc;
use maxminddb::{geoip2, Reader};
use reqwest::header::{HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use xlstatus_shared::AgentId;

const GEOIP_MMDB_MAX_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct GeoIpTestRequest {
    pub ip: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GeoIpLookupResponse {
    pub provider: String,
    pub ip: String,
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub isp: Option<String>,
    pub organization: Option<String>,
    pub timezone: Option<String>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentGeoLocation {
    pub source: String,
    pub provider: String,
    pub ip: String,
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub timezone: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GeoIpMaintenanceResponse {
    pub action: String,
    pub supported: bool,
    pub message: String,
    pub status: Option<GeoIpMmdbStatus>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GeoIpUpdateRequest {
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GeoIpMmdbStatus {
    pub configured: bool,
    pub path: String,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub database_type: Option<String>,
    pub build_epoch: Option<u64>,
    pub build_at: Option<String>,
    pub ip_version: Option<u16>,
    pub languages: Vec<String>,
    pub description: HashMap<String, String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentIpSnapshot {
    ipv4: Option<String>,
    ipv6: Option<String>,
}

pub async fn test_geoip(
    State(state): State<AppState>,
    auth: AuthSession,
    Json(req): Json<GeoIpTestRequest>,
) -> Result<Json<ApiResponse<GeoIpLookupResponse>>, AppError> {
    require_admin(&auth)?;
    let ip = req.ip.trim();
    if ip.is_empty() {
        return Err(AppError::BadRequest("ip is required".into()));
    }
    if ip.parse::<std::net::IpAddr>().is_err() {
        return Err(AppError::BadRequest("ip is invalid".into()));
    }
    let provider = req.provider.as_deref().map(str::trim);
    let token = req.token.as_deref();
    let result = lookup_ip_with_provider(&state.db, ip, provider, token).await?;
    Ok(Json(ApiResponse::success(result)))
}

pub async fn lookup_agent_geo_location(
    db: &DatabaseBackend,
    agent_id: AgentId,
) -> Option<AgentGeoLocation> {
    let snapshot = match latest_agent_ip(db, agent_id).await {
        Ok(snapshot) => snapshot,
        Err(err) => {
            tracing::debug!("agent GeoIP snapshot lookup failed: {}", err);
            return None;
        }
    };
    let ip = snapshot.ipv4.or(snapshot.ipv6)?;
    let lookup = match lookup_ip_with_provider(db, &ip, None, None).await {
        Ok(lookup) => lookup,
        Err(err) => {
            tracing::debug!("agent GeoIP lookup failed for {}: {:?}", agent_id.0, err);
            return None;
        }
    };
    if lookup.country.is_none()
        && lookup.region.is_none()
        && lookup.city.is_none()
        && lookup.latitude.is_none()
        && lookup.longitude.is_none()
    {
        return None;
    }
    Some(AgentGeoLocation {
        source: "geoip".into(),
        provider: lookup.provider,
        ip: lookup.ip,
        country: lookup.country,
        region: lookup.region,
        city: lookup.city,
        latitude: lookup.latitude,
        longitude: lookup.longitude,
        timezone: lookup.timezone,
    })
}

pub async fn geoip_status(
    State(_state): State<AppState>,
    auth: AuthSession,
) -> Result<Json<ApiResponse<GeoIpMmdbStatus>>, AppError> {
    require_admin_cookie_session(&auth)?;
    Ok(Json(ApiResponse::success(read_mmdb_status())))
}

pub async fn update_geoip_database(
    State(_state): State<AppState>,
    auth: AuthSession,
    payload: Option<Json<GeoIpUpdateRequest>>,
) -> Result<Json<ApiResponse<GeoIpMaintenanceResponse>>, AppError> {
    require_admin_cookie_session(&auth)?;
    let req = payload.map(|Json(req)| req).unwrap_or_default();
    let source_url = req
        .source_url
        .or_else(|| std::env::var("XLSTATUS_GEOIP_MMDB_URL").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let source_path = req
        .source_path
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if let Some(url) = source_url {
        let bytes = download_mmdb(&url).await?;
        let status = install_mmdb_bytes(&bytes)?;
        return Ok(Json(ApiResponse::success(GeoIpMaintenanceResponse {
            action: "geoip_update".into(),
            supported: true,
            message: "MMDB downloaded and installed".into(),
            status: Some(status),
        })));
    }

    if let Some(path) = source_path {
        let bytes = std::fs::read(&path)
            .map_err(|e| AppError::BadRequest(format!("failed to read MMDB source_path: {e}")))?;
        let status = install_mmdb_bytes(&bytes)?;
        return Ok(Json(ApiResponse::success(GeoIpMaintenanceResponse {
            action: "geoip_import".into(),
            supported: true,
            message: "MMDB imported and installed".into(),
            status: Some(status),
        })));
    }

    let status = read_mmdb_status();
    Ok(Json(ApiResponse::success(GeoIpMaintenanceResponse {
        action: "geoip_update".into(),
        supported: status.configured,
        message: if status.configured {
            "MMDB is already configured".into()
        } else {
            "MMDB source_url/source_path is not configured".into()
        },
        status: Some(status),
    })))
}

pub async fn upload_geoip_database(
    State(_state): State<AppState>,
    auth: AuthSession,
    mut multipart: Multipart,
) -> Result<Json<ApiResponse<GeoIpMaintenanceResponse>>, AppError> {
    require_admin_cookie_session(&auth)?;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid multipart upload: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if name != "file" && name != "database" && name != "mmdb" {
            continue;
        }
        let bytes = field
            .bytes()
            .await
            .map_err(|e| AppError::BadRequest(format!("failed to read MMDB upload: {e}")))?;
        let status = install_mmdb_bytes(&bytes)?;
        return Ok(Json(ApiResponse::success(GeoIpMaintenanceResponse {
            action: "geoip_upload".into(),
            supported: true,
            message: "MMDB uploaded and installed".into(),
            status: Some(status),
        })));
    }

    Err(AppError::BadRequest(
        "multipart field file/database/mmdb is required".into(),
    ))
}

pub fn geoip_upload_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(GEOIP_MMDB_MAX_BYTES)
}

pub async fn handle_agent_ip_report(
    db: &DatabaseBackend,
    agent_id: AgentId,
    ipv4: Option<&str>,
    ipv6: Option<&str>,
) -> anyhow::Result<()> {
    let next = AgentIpSnapshot {
        ipv4: clean_ip(ipv4),
        ipv6: clean_ip(ipv6),
    };
    if next.ipv4.is_none() && next.ipv6.is_none() {
        return Ok(());
    }
    let previous = latest_agent_ip(db, agent_id).await?;
    if previous == next {
        return Ok(());
    }
    insert_agent_ip_event(db, agent_id, &previous, &next).await?;
    if previous.ipv4.is_some() || previous.ipv6.is_some() {
        if let Err(err) = notify_agent_ip_change(db, agent_id, &previous, &next).await {
            tracing::warn!("Agent IP change notification failed: {}", err);
        }
    }
    Ok(())
}

async fn lookup_geojs(ip: &str) -> Result<GeoIpLookupResponse, AppError> {
    let url = format!("https://get.geojs.io/v1/ip/geo/{ip}.json");
    let raw = fetch_json(&url, None).await?;
    Ok(GeoIpLookupResponse {
        provider: "geojs".into(),
        ip: json_string(&raw, &["ip"]).unwrap_or_else(|| ip.to_string()),
        country: json_string(&raw, &["country"]),
        region: json_string(&raw, &["region"]),
        city: json_string(&raw, &["city"]),
        latitude: json_f64(&raw, &["latitude"]),
        longitude: json_f64(&raw, &["longitude"]),
        isp: None,
        organization: json_string(&raw, &["organization_name", "asn_organization"]),
        timezone: json_string(&raw, &["timezone"]),
        raw,
    })
}

async fn lookup_ip_with_provider(
    db: &DatabaseBackend,
    ip: &str,
    provider: Option<&str>,
    token: Option<&str>,
) -> Result<GeoIpLookupResponse, AppError> {
    let provider = provider
        .map(normalize_geoip_provider)
        .transpose()?
        .unwrap_or(settings::geoip_provider(db).await?);
    let configured_ipinfo_token = if provider == "ipinfo" && token.is_none() {
        settings::geoip_ipinfo_token(db).await?
    } else {
        None
    };
    let ipinfo_token = token.or(configured_ipinfo_token.as_deref());
    match provider.as_str() {
        "empty" => Ok(GeoIpLookupResponse {
            provider,
            ip: ip.to_string(),
            country: None,
            region: None,
            city: None,
            latitude: None,
            longitude: None,
            isp: None,
            organization: None,
            timezone: None,
            raw: serde_json::json!({ "ip": ip }),
        }),
        "geojs" => lookup_geojs(ip).await,
        "ip-api" | "ipapi" => lookup_ip_api(ip).await,
        "ipinfo" => lookup_ipinfo(ip, ipinfo_token).await,
        "mmdb" => lookup_mmdb(ip),
        _ => Err(AppError::BadRequest(
            "provider must be empty, geojs, ip-api, ipinfo, or mmdb".into(),
        )),
    }
}

async fn lookup_ip_api(ip: &str) -> Result<GeoIpLookupResponse, AppError> {
    let url = format!(
        "http://ip-api.com/json/{ip}?fields=status,message,country,regionName,city,lat,lon,isp,org,as,timezone,query"
    );
    let raw = fetch_json(&url, None).await?;
    if raw.get("status").and_then(|value| value.as_str()) == Some("fail") {
        let message =
            json_string(&raw, &["message"]).unwrap_or_else(|| "ip-api lookup failed".into());
        return Err(AppError::BadRequest(message));
    }
    Ok(GeoIpLookupResponse {
        provider: "ip-api".into(),
        ip: json_string(&raw, &["query"]).unwrap_or_else(|| ip.to_string()),
        country: json_string(&raw, &["country"]),
        region: json_string(&raw, &["regionName"]),
        city: json_string(&raw, &["city"]),
        latitude: json_f64(&raw, &["lat"]),
        longitude: json_f64(&raw, &["lon"]),
        isp: json_string(&raw, &["isp"]),
        organization: json_string(&raw, &["org", "as"]),
        timezone: json_string(&raw, &["timezone"]),
        raw,
    })
}

async fn lookup_ipinfo(ip: &str, token: Option<&str>) -> Result<GeoIpLookupResponse, AppError> {
    let url = ipinfo_lookup_url(ip)?;
    let raw = fetch_json(url.as_str(), token).await?;
    let (latitude, longitude) = raw
        .get("loc")
        .and_then(|value| value.as_str())
        .and_then(|loc| loc.split_once(','))
        .map(|(lat, lon)| (lat.parse::<f64>().ok(), lon.parse::<f64>().ok()))
        .unwrap_or((None, None));
    Ok(GeoIpLookupResponse {
        provider: "ipinfo".into(),
        ip: json_string(&raw, &["ip"]).unwrap_or_else(|| ip.to_string()),
        country: json_string(&raw, &["country"]),
        region: json_string(&raw, &["region"]),
        city: json_string(&raw, &["city"]),
        latitude,
        longitude,
        isp: None,
        organization: json_string(&raw, &["org"]),
        timezone: json_string(&raw, &["timezone"]),
        raw,
    })
}

async fn fetch_json(url: &str, bearer_token: Option<&str>) -> Result<serde_json::Value, AppError> {
    let validated = validate_outbound_url_resolved(url, "GeoIP lookup")
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let mut request = secure_reqwest_client_builder(&validated)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::BadRequest(format!("GeoIP client init failed: {e}")))?
        .get(validated.url.clone());
    if let Some(header) = geoip_bearer_auth_header(bearer_token)? {
        request = request.header(AUTHORIZATION, header);
    }
    let response = request
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("GeoIP lookup failed: {e}")))?;
    if !response.status().is_success() {
        return Err(AppError::BadRequest(format!(
            "GeoIP lookup failed with {}",
            response.status()
        )));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| AppError::BadRequest(format!("GeoIP response is invalid: {e}")))
}

fn ipinfo_lookup_url(ip: &str) -> Result<reqwest::Url, AppError> {
    reqwest::Url::parse(&format!("https://ipinfo.io/{ip}/json"))
        .map_err(|e| AppError::BadRequest(format!("invalid ipinfo URL: {e}")))
}

fn geoip_bearer_auth_header(token: Option<&str>) -> Result<Option<HeaderValue>, AppError> {
    let Some(token) = token.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    HeaderValue::from_str(&format!("Bearer {token}"))
        .map(Some)
        .map_err(|_| AppError::BadRequest("ipinfo token contains invalid header bytes".into()))
}

fn lookup_mmdb(ip: &str) -> Result<GeoIpLookupResponse, AppError> {
    let path = mmdb_path();
    let reader = Reader::open_readfile(&path).map_err(|e| {
        AppError::BadRequest(format!(
            "MMDB is not readable at {}: {e}",
            path.to_string_lossy()
        ))
    })?;
    let ip_addr = ip
        .parse::<std::net::IpAddr>()
        .map_err(|e| AppError::BadRequest(format!("ip is invalid: {e}")))?;
    let lookup = reader
        .lookup(ip_addr)
        .map_err(|e| AppError::BadRequest(format!("MMDB lookup failed: {e}")))?;

    if !lookup.has_data() {
        return Ok(GeoIpLookupResponse {
            provider: "mmdb".into(),
            ip: ip.to_string(),
            country: None,
            region: None,
            city: None,
            latitude: None,
            longitude: None,
            isp: None,
            organization: None,
            timezone: None,
            raw: serde_json::json!({
                "found": false,
                "database_type": reader.metadata.database_type,
            }),
        });
    }

    let city = lookup.decode::<geoip2::City>().ok().flatten();
    let country = lookup.decode::<geoip2::Country>().ok().flatten();
    let isp = lookup.decode::<geoip2::Isp>().ok().flatten();

    let country_name = city
        .as_ref()
        .and_then(|record| localized_name(&record.country.names))
        .or_else(|| {
            country
                .as_ref()
                .and_then(|record| localized_name(&record.country.names))
        })
        .or_else(|| {
            city.as_ref()
                .and_then(|record| record.country.iso_code.map(str::to_string))
        })
        .or_else(|| {
            country
                .as_ref()
                .and_then(|record| record.country.iso_code.map(str::to_string))
        });
    let region = city
        .as_ref()
        .and_then(|record| record.subdivisions.first())
        .and_then(|subdivision| {
            localized_name(&subdivision.names).or_else(|| subdivision.iso_code.map(str::to_string))
        });
    let city_name = city
        .as_ref()
        .and_then(|record| localized_name(&record.city.names));
    let latitude = city.as_ref().and_then(|record| record.location.latitude);
    let longitude = city.as_ref().and_then(|record| record.location.longitude);
    let timezone = city
        .as_ref()
        .and_then(|record| record.location.time_zone.map(str::to_string));
    let isp_name = isp
        .as_ref()
        .and_then(|record| record.isp.map(str::to_string));
    let organization = isp.as_ref().and_then(|record| {
        record
            .organization
            .or(record.autonomous_system_organization)
            .map(str::to_string)
    });

    Ok(GeoIpLookupResponse {
        provider: "mmdb".into(),
        ip: ip.to_string(),
        country: country_name,
        region,
        city: city_name,
        latitude,
        longitude,
        isp: isp_name,
        organization,
        timezone,
        raw: serde_json::json!({
            "found": true,
            "database_type": reader.metadata.database_type,
            "city": city,
            "country": country,
            "isp": isp,
        }),
    })
}

fn localized_name(names: &geoip2::Names<'_>) -> Option<String> {
    names
        .simplified_chinese
        .or(names.english)
        .or(names.japanese)
        .or(names.french)
        .or(names.spanish)
        .or(names.german)
        .or(names.brazilian_portuguese)
        .or(names.russian)
        .map(str::to_string)
}

fn read_mmdb_status() -> GeoIpMmdbStatus {
    let path = mmdb_path();
    let path_text = path.to_string_lossy().to_string();
    let metadata = std::fs::metadata(&path);
    let (size_bytes, modified_at) = match metadata {
        Ok(metadata) => (
            Some(metadata.len()),
            metadata.modified().ok().map(system_time_rfc3339),
        ),
        Err(_) => (None, None),
    };

    match Reader::open_readfile(&path) {
        Ok(reader) => GeoIpMmdbStatus {
            configured: true,
            path: path_text,
            size_bytes,
            modified_at,
            database_type: Some(reader.metadata.database_type),
            build_epoch: Some(reader.metadata.build_epoch),
            build_at: Some(unix_epoch_rfc3339(reader.metadata.build_epoch)),
            ip_version: Some(reader.metadata.ip_version),
            languages: reader.metadata.languages,
            description: reader.metadata.description.into_iter().collect(),
            error: None,
        },
        Err(err) => GeoIpMmdbStatus {
            configured: false,
            path: path_text,
            size_bytes,
            modified_at,
            database_type: None,
            build_epoch: None,
            build_at: None,
            ip_version: None,
            languages: Vec::new(),
            description: HashMap::new(),
            error: Some(err.to_string()),
        },
    }
}

fn install_mmdb_bytes(bytes: &[u8]) -> Result<GeoIpMmdbStatus, AppError> {
    if bytes.is_empty() {
        return Err(AppError::BadRequest("MMDB file is empty".into()));
    }
    if bytes.len() > GEOIP_MMDB_MAX_BYTES {
        return Err(AppError::BadRequest(format!(
            "MMDB file exceeds {} bytes",
            GEOIP_MMDB_MAX_BYTES
        )));
    }
    Reader::from_source(bytes.to_vec())
        .map_err(|e| AppError::BadRequest(format!("uploaded file is not a valid MMDB: {e}")))?;

    let path = mmdb_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::BadRequest(format!("failed to create GeoIP directory: {e}")))?;
    }
    let temp_path = temp_mmdb_path(&path);
    std::fs::write(&temp_path, bytes)
        .map_err(|e| AppError::BadRequest(format!("failed to write MMDB temp file: {e}")))?;
    std::fs::rename(&temp_path, &path)
        .map_err(|e| AppError::BadRequest(format!("failed to install MMDB file: {e}")))?;
    Ok(read_mmdb_status())
}

async fn download_mmdb(url: &str) -> Result<Vec<u8>, AppError> {
    let validated = validate_outbound_url_resolved(url, "GeoIP MMDB download")
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let response = secure_reqwest_client_builder(&validated)
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| AppError::BadRequest(format!("GeoIP download client init failed: {e}")))?
        .get(validated.url.clone())
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("GeoIP MMDB download failed: {e}")))?;
    if !response.status().is_success() {
        return Err(AppError::BadRequest(format!(
            "GeoIP MMDB download failed with {}",
            response.status()
        )));
    }
    if response
        .content_length()
        .map(|length| length > GEOIP_MMDB_MAX_BYTES as u64)
        .unwrap_or(false)
    {
        return Err(AppError::BadRequest(format!(
            "MMDB download exceeds {} bytes",
            GEOIP_MMDB_MAX_BYTES
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to read MMDB download: {e}")))?;
    if bytes.len() > GEOIP_MMDB_MAX_BYTES {
        return Err(AppError::BadRequest(format!(
            "MMDB download exceeds {} bytes",
            GEOIP_MMDB_MAX_BYTES
        )));
    }
    Ok(bytes.to_vec())
}

fn mmdb_path() -> PathBuf {
    if let Ok(path) = std::env::var("XLSTATUS_GEOIP_MMDB_PATH") {
        return PathBuf::from(path);
    }
    let data_dir = std::env::var("XLSTATUS_DATA_DIR").unwrap_or_else(|_| "data".into());
    PathBuf::from(data_dir)
        .join("geoip")
        .join("GeoLite2-City.mmdb")
}

fn temp_mmdb_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("GeoLite2-City.mmdb");
    path.with_file_name(format!("{file_name}.{}.tmp", uuid::Uuid::now_v7()))
}

fn system_time_rfc3339(value: std::time::SystemTime) -> String {
    chrono::DateTime::<Utc>::from(value).to_rfc3339()
}

fn unix_epoch_rfc3339(epoch: u64) -> String {
    let value = std::time::UNIX_EPOCH + std::time::Duration::from_secs(epoch);
    system_time_rfc3339(value)
}

async fn latest_agent_ip(
    db: &DatabaseBackend,
    agent_id: AgentId,
) -> anyhow::Result<AgentIpSnapshot> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT new_ipv4, new_ipv6 FROM agent_ip_events WHERE agent_id = ? ORDER BY created_at DESC LIMIT 1",
            )
            .bind(agent_id.0.to_string())
            .fetch_optional(pool)
            .await?;
            Ok(row
                .map(|(ipv4, ipv6)| AgentIpSnapshot { ipv4, ipv6 })
                .unwrap_or(AgentIpSnapshot {
                    ipv4: None,
                    ipv6: None,
                }))
        }
        DatabaseBackend::Postgres(pool) => {
            let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT new_ipv4, new_ipv6 FROM agent_ip_events WHERE agent_id = $1 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(agent_id.0)
            .fetch_optional(pool)
            .await?;
            Ok(row
                .map(|(ipv4, ipv6)| AgentIpSnapshot { ipv4, ipv6 })
                .unwrap_or(AgentIpSnapshot {
                    ipv4: None,
                    ipv6: None,
                }))
        }
    }
}

async fn insert_agent_ip_event(
    db: &DatabaseBackend,
    agent_id: AgentId,
    previous: &AgentIpSnapshot,
    next: &AgentIpSnapshot,
) -> anyhow::Result<()> {
    let id = uuid::Uuid::now_v7();
    let now = Utc::now();
    match db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO agent_ip_events (id, agent_id, old_ipv4, new_ipv4, old_ipv6, new_ipv6, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id.to_string())
            .bind(agent_id.0.to_string())
            .bind(&previous.ipv4)
            .bind(&next.ipv4)
            .bind(&previous.ipv6)
            .bind(&next.ipv6)
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO agent_ip_events (id, agent_id, old_ipv4, new_ipv4, old_ipv6, new_ipv6, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(id)
            .bind(agent_id.0)
            .bind(&previous.ipv4)
            .bind(&next.ipv4)
            .bind(&previous.ipv6)
            .bind(&next.ipv6)
            .bind(now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

async fn notify_agent_ip_change(
    db: &DatabaseBackend,
    agent_id: AgentId,
    previous: &AgentIpSnapshot,
    next: &AgentIpSnapshot,
) -> anyhow::Result<()> {
    if !settings::geoip_ip_change_enabled(db)
        .await
        .map_err(app_error_to_anyhow)?
    {
        return Ok(());
    }
    let Some(agent) = AgentRepository::new(db.clone())
        .find_by_id(agent_id)
        .await?
    else {
        return Ok(());
    };
    let scoped_server_ids = settings::geoip_ip_change_server_ids(db)
        .await
        .map_err(app_error_to_anyhow)?;
    if !scoped_server_ids.is_empty()
        && !scoped_server_ids
            .iter()
            .any(|server_id| server_id == &agent.id.0.to_string())
    {
        return Ok(());
    }
    let channels = if let Some(group_id) = settings::geoip_ip_change_notification_group_id(db)
        .await
        .map_err(app_error_to_anyhow)?
    {
        list_notification_channels_for_group(db, agent.owner_user_id, &group_id).await?
    } else {
        list_notification_channels_for_owner(db, agent.owner_user_id).await?
    };
    if channels.is_empty() {
        return Ok(());
    }
    let mut metadata = HashMap::new();
    metadata.insert("agent_id".into(), agent.id.0.to_string());
    metadata.insert("agent_name".into(), agent.name.clone());
    metadata.insert("old_ipv4".into(), previous.ipv4.clone().unwrap_or_default());
    metadata.insert("new_ipv4".into(), next.ipv4.clone().unwrap_or_default());
    metadata.insert("old_ipv6".into(), previous.ipv6.clone().unwrap_or_default());
    metadata.insert("new_ipv6".into(), next.ipv6.clone().unwrap_or_default());
    let message = NotificationMessage {
        title: "Agent IP changed".into(),
        message: format!(
            "{} IP changed: IPv4 {} -> {}, IPv6 {} -> {}",
            agent.name,
            previous.ipv4.as_deref().unwrap_or("-"),
            next.ipv4.as_deref().unwrap_or("-"),
            previous.ipv6.as_deref().unwrap_or("-"),
            next.ipv6.as_deref().unwrap_or("-")
        ),
        severity: notification_severity_from_setting(
            &settings::geoip_ip_change_severity(db)
                .await
                .map_err(app_error_to_anyhow)?,
        ),
        timestamp: Utc::now().to_rfc3339(),
        metadata,
    };
    let sender = NotificationSender::new();
    for channel in channels {
        if let Err(err) = sender.send(&channel, &message).await {
            tracing::warn!(
                "IP change notification channel {} failed: {}",
                channel.id,
                err
            );
        }
    }
    Ok(())
}

async fn list_notification_channels_for_group(
    db: &DatabaseBackend,
    owner_user_id: xlstatus_shared::UserId,
    group_id: &str,
) -> anyhow::Result<Vec<NotificationChannel>> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT n.id, n.name, n.url, n.request_method, n.request_type, n.headers_json, n.body_template, n.verify_tls
                FROM notifications n
                JOIN notification_group_members ngm ON ngm.notification_id = n.id
                JOIN notification_groups ng ON ng.id = ngm.group_id
                WHERE ng.id = ? AND ng.owner_user_id = ?
                ORDER BY n.created_at ASC
                "#,
            )
            .bind(group_id)
            .bind(owner_user_id.0.to_string())
            .fetch_all(pool)
            .await?;
            rows.into_iter().map(channel_from_sqlite_row).collect()
        }
        DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT n.id, n.name, n.url, n.request_method, n.request_type, n.headers_json, n.body_template, n.verify_tls
                FROM notifications n
                JOIN notification_group_members ngm ON ngm.notification_id = n.id
                JOIN notification_groups ng ON ng.id = ngm.group_id
                WHERE ng.id = $1 AND ng.owner_user_id = $2
                ORDER BY n.created_at ASC
                "#,
            )
            .bind(group_id)
            .bind(owner_user_id.0)
            .fetch_all(pool)
            .await?;
            rows.into_iter().map(channel_from_pg_row).collect()
        }
    }
}

async fn list_notification_channels_for_owner(
    db: &DatabaseBackend,
    owner_user_id: xlstatus_shared::UserId,
) -> anyhow::Result<Vec<NotificationChannel>> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT id, name, url, request_method, request_type, headers_json, body_template, verify_tls
                FROM notifications
                WHERE owner_user_id = ?
                ORDER BY created_at ASC
                "#,
            )
            .bind(owner_user_id.0.to_string())
            .fetch_all(pool)
            .await?;
            rows.into_iter().map(channel_from_sqlite_row).collect()
        }
        DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT id, name, url, request_method, request_type, headers_json, body_template, verify_tls
                FROM notifications
                WHERE owner_user_id = $1
                ORDER BY created_at ASC
                "#,
            )
            .bind(owner_user_id.0)
            .fetch_all(pool)
            .await?;
            rows.into_iter().map(channel_from_pg_row).collect()
        }
    }
}

fn channel_from_pg_row(row: sqlx::postgres::PgRow) -> anyhow::Result<NotificationChannel> {
    notification_channel_from_values(
        row.try_get("id")?,
        row.try_get("name")?,
        row.try_get("url")?,
        row.try_get("request_method")?,
        row.try_get("request_type")?,
        row.try_get("headers_json")?,
        row.try_get::<Option<String>, _>("body_template")?
            .unwrap_or_default(),
        row.try_get("verify_tls")?,
    )
}

fn channel_from_sqlite_row(row: sqlx::sqlite::SqliteRow) -> anyhow::Result<NotificationChannel> {
    notification_channel_from_values(
        row.try_get("id")?,
        row.try_get("name")?,
        row.try_get("url")?,
        row.try_get("request_method")?,
        row.try_get("request_type")?,
        row.try_get("headers_json")?,
        row.try_get::<Option<String>, _>("body_template")?
            .unwrap_or_default(),
        row.try_get::<i64, _>("verify_tls")? != 0,
    )
}

fn notification_channel_from_values(
    id: String,
    name: String,
    url: String,
    request_method: String,
    request_type: String,
    headers_json: Option<String>,
    body_template: String,
    verify_tls: bool,
) -> anyhow::Result<NotificationChannel> {
    let headers = headers_json
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(serde_json::from_str::<HashMap<String, String>>)
        .transpose()?
        .unwrap_or_default();
    Ok(NotificationChannel {
        id,
        name,
        url,
        request_method,
        request_type,
        headers,
        body_template,
        verify_tls,
    })
}

fn notification_severity_from_setting(value: &str) -> NotificationSeverity {
    match value {
        "critical" => NotificationSeverity::Critical,
        "error" => NotificationSeverity::Error,
        "warning" => NotificationSeverity::Warning,
        _ => NotificationSeverity::Info,
    }
}

fn app_error_to_anyhow(err: AppError) -> anyhow::Error {
    anyhow::anyhow!("{err:?}")
}

fn clean_ip(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| value.parse::<std::net::IpAddr>().is_ok())
        .map(ToString::to_string)
}

fn json_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|item| item.as_str()))
        .map(ToString::to_string)
}

fn json_f64(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|item| item.as_f64().or_else(|| item.as_str()?.parse().ok()))
    })
}

fn require_admin(auth: &AuthSession) -> Result<(), AppError> {
    if auth.role.is_admin() {
        Ok(())
    } else {
        Err(AppError::Forbidden("Admin role required".into()))
    }
}

fn require_admin_cookie_session(auth: &AuthSession) -> Result<(), AppError> {
    require_admin(auth)?;
    if matches!(auth.auth_kind, AuthKind::PersonalAccessToken) {
        return Err(AppError::Forbidden(
            "GeoIP maintenance requires an admin cookie session".into(),
        ));
    }
    Ok(())
}

fn normalize_geoip_provider(value: &str) -> Result<String, AppError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "empty" => Ok("empty".into()),
        "geojs" => Ok("geojs".into()),
        "ip-api" | "ipapi" => Ok("ip-api".into()),
        "ipinfo" => Ok("ipinfo".into()),
        "mmdb" => Ok("mmdb".into()),
        _ => Err(AppError::BadRequest(
            "provider must be empty, geojs, ip-api, ipinfo, or mmdb".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xlstatus_shared::{UserId, UserRole};

    #[test]
    fn cleans_valid_ip_only() {
        assert_eq!(clean_ip(Some(" 1.1.1.1 ")), Some("1.1.1.1".into()));
        assert_eq!(clean_ip(Some("not-ip")), None);
        assert_eq!(clean_ip(None), None);
    }

    #[test]
    fn extracts_json_numbers_from_strings() {
        let value = serde_json::json!({ "lat": "12.5", "lon": 35.0 });
        assert_eq!(json_f64(&value, &["lat"]), Some(12.5));
        assert_eq!(json_f64(&value, &["lon"]), Some(35.0));
    }

    #[test]
    fn ipinfo_lookup_does_not_put_token_in_url_query() {
        let url = ipinfo_lookup_url("1.1.1.1").expect("ipinfo url");

        assert_eq!(url.as_str(), "https://ipinfo.io/1.1.1.1/json");
        assert!(url.query().is_none());
    }

    #[test]
    fn ipinfo_token_uses_bearer_authorization_header() {
        let header = geoip_bearer_auth_header(Some(" token-value ")).expect("valid bearer header");

        assert_eq!(
            header.and_then(|value| value.to_str().ok().map(str::to_string)),
            Some("Bearer token-value".into())
        );
        assert!(geoip_bearer_auth_header(Some("")).unwrap().is_none());
    }

    #[test]
    fn ipinfo_token_rejects_invalid_header_bytes() {
        let err = geoip_bearer_auth_header(Some("bad\r\nvalue")).unwrap_err();

        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn geoip_maintenance_allows_admin_cookie_session() {
        let auth = auth_session(AuthKind::Session);

        assert!(require_admin_cookie_session(&auth).is_ok());
    }

    #[test]
    fn geoip_maintenance_rejects_admin_pat_session() {
        let auth = auth_session(AuthKind::PersonalAccessToken);

        assert!(matches!(
            require_admin_cookie_session(&auth),
            Err(AppError::Forbidden(_))
        ));
    }

    fn auth_session(auth_kind: AuthKind) -> AuthSession {
        AuthSession {
            session_id: "sess".into(),
            user_id: UserId(uuid::Uuid::from_bytes([1; 16])),
            username: "admin".into(),
            role: UserRole::Admin,
            csrf_token: "csrf".into(),
            auth_kind,
            scopes: vec!["admin:*".into()],
            server_ids: None,
            pat_id: None,
        }
    }
}

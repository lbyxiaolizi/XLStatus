use anyhow::Context;
use axum::{
    extract::connect_info::ConnectInfo,
    extract::Query,
    extract::Request,
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    middleware,
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::{Certificate, Identity, Server as TonicServer, ServerTlsConfig};
use tower_http::cors::{AllowOrigin, CorsLayer};
use xlstatus_proto_gen::xlstatus::v1::agent_service_server::AgentServiceServer;

mod alerts;
mod api;
mod auth;
mod config;
mod db;
mod ddns;
mod grpc;
mod mcp;
mod nat;
mod notifications;
mod realtime;
mod secrets;
mod security;
mod services;
mod tasks;

use crate::alerts::engine::AlertEngine;
use crate::db::{CreateUserInput, DatabaseBackend, UserRepository};
use crate::services::monitor::ServiceMonitor;
use api::v1::agent::{agent_auth_body_limit, create_enrollment_token, enroll};
use api::v1::agent_jwt::{get_agent_jwt, get_agent_jwt_challenge};
use api::v1::alerts::{
    alert_body_limit, create_alert_rule, delete_alert_rule, list_alert_events, list_alert_rules,
};
use api::v1::auth::{
    auth_body_limit, create_user, create_waf_bans, delete_session, delete_user, delete_waf_ban,
    disable_totp, enable_totp, get_totp_status, list_sessions, list_users, list_waf_bans, login,
    login_body_limit, logout, setup_totp, totp_body_limit, update_user, AppState,
};
use api::v1::cloudflared::cloudflared_token_body_limit;
use api::v1::ddns::{
    check_ddns_now, create_ddns_config, ddns_body_limit, delete_ddns_config, list_ddns_configs,
    list_ddns_history, reload_ddns_providers,
};
use api::v1::geoip::{
    geoip_status, geoip_test_body_limit, geoip_update_body_limit, geoip_upload_body_limit,
    test_geoip, update_geoip_database, upload_geoip_database,
};
use api::v1::maintenance::{
    compact_tsdb, download_archive, download_backup, maintenance_action_body_limit,
    maintenance_status, restore_backup, restore_body_limit, update_tsdb_retention, vacuum_sqlite,
};
use api::v1::mcp::{
    execute_mcp_tool, get_mcp_info, handle_mcp_jsonrpc, list_mcp_tools, mcp_body_limit,
};
use api::v1::nat::{
    create_nat_mapping, delete_nat_mapping, get_nat_mapping, list_all_nat_mappings,
    list_nat_mappings, nat_body_limit, update_nat_mapping,
};
use api::v1::notifications::{
    add_notification_group_member, create_notification, create_notification_group,
    delete_notification, delete_notification_group, delete_notification_group_member,
    list_notification_groups, list_notification_providers, list_notifications,
    notification_body_limit, test_notification, update_notification, update_notification_group,
};
use api::v1::oauth::{
    get_profile, list_oauth_bindings, list_oauth_providers, oauth_callback, start_oauth_bind,
    start_oauth_login, unbind_oauth_provider,
};
use api::v1::openapi::openapi_json;
use api::v1::pat::{create_pat, list_pats, pat_body_limit, revoke_pat};
use api::v1::server_ops::{
    apply_config, delete_file, download_url, force_update, get_config, list_files, read_file,
    server_ops_body_limit, upload_url, write_file,
};
use api::v1::servers::server_management_body_limit;
use api::v1::service_history::{get_service_history, get_service_uptime};
use api::v1::settings::{get_settings, settings_body_limit, update_settings};
use api::v1::terminal::{create_terminal_session, terminal_body_limit, ws_terminal};
use api::v1::themes::{
    delete_theme, import_theme, list_themes, select_theme, theme_body_limit, update_theme,
};
// M3: server list / detail / metrics routes are registered inline below
use api::v1::services::{
    create_service, delete_service, get_service, list_services, service_body_limit, test_probe,
    update_service,
};
use api::v1::tasks::{
    create_task, delete_task, get_task, get_task_runs, list_tasks, run_task, task_body_limit,
    update_task,
};
use api::v1::transfers::{
    list_temporary_transfers, revoke_temporary_transfer, temp_download, temp_upload,
    upload_body_limit,
};
use auth::middleware::session_middleware;
use xlstatus_shared::UserRole;

const GRPC_MESSAGE_LIMIT: usize = 256 * 1024 * 1024;
const HTTP_MAX_PATH_BYTES: usize = 4096;
const HTTP_MAX_QUERY_BYTES: usize = 16 * 1024;
const DEFAULT_AGENT_INSTALL_VERSION: &str = "v0.1.0-alpha.3";
const INSTALL_BOOTSTRAP_MAX_QUERY_BYTES: usize = 16 * 1024;
const INSTALL_BOOTSTRAP_MAX_HOST_BYTES: usize = 512;
const INSTALL_BOOTSTRAP_MAX_URL_BYTES: usize = 2048;
const INSTALL_BOOTSTRAP_MAX_TLS_PATH_BYTES: usize = 1024;
const INSTALL_BOOTSTRAP_MAX_TLS_DOMAIN_BYTES: usize = 253;
const INSTALL_BOOTSTRAP_MAX_ENROLLMENT_TOKEN_BYTES: usize = 128;
const INSTALL_BOOTSTRAP_MAX_AGENT_NAME_BYTES: usize = 255;

// M4: the alert engine is started in main() and then needs to be
// reachable from the gRPC `Session` task so HostState updates are
// observed for ServerResource / ServerOffline conditions. We use a
// `OnceLock` rather than threading the engine through every layer
// because the gRPC service is constructed independently of the
// main state.
static ALERT_ENGINE: std::sync::OnceLock<Arc<AlertEngine>> = std::sync::OnceLock::new();

/// Public accessor for the singleton alert engine. The gRPC layer
/// calls `current_alert_engine()` to grab a handle to push HostState
/// updates.
pub fn current_alert_engine() -> Option<Arc<AlertEngine>> {
    ALERT_ENGINE.get().cloned()
}

/// M5: shared registry of in-flight task dispatch requests. The
/// gRPC `session` loop delivers incoming `TaskResult` messages
/// to the registered waiters (the HTTP `run_task` handler).
pub fn current_task_response_registry() -> Arc<crate::grpc::TaskResponseRegistry> {
    static REG: std::sync::OnceLock<Arc<crate::grpc::TaskResponseRegistry>> =
        std::sync::OnceLock::new();
    REG.get_or_init(|| Arc::new(crate::grpc::TaskResponseRegistry::new()))
        .clone()
}

static DDNS_MANAGER: std::sync::OnceLock<Arc<crate::ddns::manager::DdnsManager>> =
    std::sync::OnceLock::new();
static NAT_MANAGER: std::sync::OnceLock<Arc<crate::nat::tunnel::NatTunnelManager>> =
    std::sync::OnceLock::new();

/// M6: shared handle to the running DDNS manager. The HTTP
/// reload endpoint uses this to refresh providers after an admin
/// adds a new config.
pub fn current_ddns_manager() -> Option<Arc<crate::ddns::manager::DdnsManager>> {
    DDNS_MANAGER.get().cloned()
}

pub fn set_ddns_manager(mgr: Arc<crate::ddns::manager::DdnsManager>) {
    let _ = DDNS_MANAGER.set(mgr);
}

pub fn current_nat_manager() -> Option<Arc<crate::nat::tunnel::NatTunnelManager>> {
    NAT_MANAGER.get().cloned()
}

pub fn set_nat_manager(mgr: Arc<crate::nat::tunnel::NatTunnelManager>) {
    let _ = NAT_MANAGER.set(mgr);
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    tracing::info!("Starting XLStatus server");

    // Load configuration
    let config = config::Config::load()?;
    secrets::init_secret_crypto(config.secret_encryption_key_material())?;
    tracing::info!("Configuration loaded");

    // Connect to database
    let db = db::DatabaseBackend::connect(&config.database.url, config.database.create_if_missing)
        .await?;
    tracing::info!("Connected to database: {}", config.database.url);

    // Run migrations
    db.run_migrations().await?;
    tracing::info!("Database migrations applied");
    match secrets::migrate_plaintext_secrets(&db).await {
        Ok(changed) if changed > 0 => {
            tracing::info!("Encrypted {} existing plaintext secret values", changed);
        }
        Ok(_) => {}
        Err(err) => {
            tracing::warn!("Secret migration failed: {}", err);
            return Err(err);
        }
    }

    seed_admin_user(&db).await?;

    // Build the live agent session registry before background jobs
    // that may dispatch work to connected agents.
    let session_registry = grpc::SessionRegistry::new();
    let io_registry = grpc::IoRegistry::new();
    let task_response_registry = current_task_response_registry();

    // M4: start service monitor + alert engine in the background.
    let monitor = Arc::new(ServiceMonitor::new(
        db.clone(),
        session_registry.clone(),
        task_response_registry.clone(),
    ));
    let monitor_clone = monitor.clone();
    tokio::spawn(async move {
        monitor_clone.start().await;
    });
    let alert_engine = Arc::new(AlertEngine::new(
        db.clone(),
        session_registry.clone(),
        task_response_registry.clone(),
    ));
    let alert_engine_clone = alert_engine.clone();
    tokio::spawn(async move {
        alert_engine_clone.start().await;
    });
    // Stash the engine + its "latest" handle into AppState so the
    // gRPC session loop can publish HostState to it.
    ALERT_ENGINE.set(alert_engine).ok();

    // M6: start the DDNS manager (loads providers from
    // ddns_configs, ticks every 60s, applies IP changes via the
    // configured provider, writes ddns_history rows).
    let ddns_manager = Arc::new(crate::ddns::manager::DdnsManager::new(db.clone()));
    if let Err(e) = ddns_manager.clone().start().await {
        tracing::warn!("DDNS manager failed to start: {}", e);
    } else {
        tracing::info!("DDNS manager started");
    }
    crate::set_ddns_manager(ddns_manager);

    // M6: start the NAT manager after the shared IO registry exists,
    // so reverse-tunnel listeners can forward new public connections
    // through the live agent IoStream bridge.
    let nat_manager = Arc::new(crate::nat::tunnel::NatTunnelManager::new(
        db.clone(),
        io_registry.clone(),
    ));
    crate::set_nat_manager(nat_manager.clone());
    let nat_manager_clone = nat_manager.clone();
    tokio::spawn(async move {
        if let Err(e) = nat_manager_clone.start().await {
            tracing::warn!("NAT manager failed to start: {}", e);
        } else {
            tracing::info!("NAT manager started");
        }
    });

    // M5: start the scheduled task dispatcher after the live-session
    // registry exists, so cron tasks use the same real gRPC dispatch
    // path as manual `/api/v1/tasks/:id/run` requests.
    let task_scheduler = Arc::new(crate::tasks::scheduler::TaskScheduler::new(
        db.clone(),
        session_registry.clone(),
        task_response_registry.clone(),
    ));
    let task_scheduler_clone = task_scheduler.clone();
    tokio::spawn(async move {
        task_scheduler_clone.start().await;
    });

    // Create app state. The session_registry is built up here so
    // the AppState handlers (M5 task dispatch) can reach the live
    // agent sessions registered by the gRPC service.
    let metrics = xlstatus_tsdb::MetricStore::in_memory();
    match api::v1::settings::tsdb_retention_days(&db).await {
        Ok(days) => {
            if let Err(err) = metrics.set_retention(chrono::Duration::days(days)) {
                tracing::warn!("failed to apply TSDB retention setting: {}", err);
            }
        }
        Err(err) => {
            tracing::warn!("failed to load TSDB retention setting: {:?}", err);
        }
    }
    let realtime = crate::realtime::BroadcastHub::new();
    let state = AppState {
        db: db.clone(),
        config: Arc::new(config.clone()),
        agent_jwt_challenges: Arc::new(RwLock::new(std::collections::HashMap::new())),
        metrics: metrics.clone(),
        realtime: realtime.clone(),
        session_registry: session_registry.clone(),
        terminal_sessions: api::v1::terminal::TerminalSessionRegistry::new(),
        io_registry: io_registry.clone(),
    };

    // Start HTTP server
    let http_bind = config.server.http_bind.clone();
    let http_handle = tokio::spawn(async move {
        async fn run_http_server(bind: String, state: AppState) -> anyhow::Result<()> {
            let cors = build_cors_layer(&state.config.server.cors_allowed_origins)?;
            let protected = Router::new()
                .route("/api/v1/auth/logout", post(logout))
                .route("/api/v1/auth/totp/status", get(get_totp_status))
                .route(
                    "/api/v1/auth/totp/setup",
                    post(setup_totp).layer(totp_body_limit()),
                )
                .route(
                    "/api/v1/auth/totp/enable",
                    post(enable_totp).layer(totp_body_limit()),
                )
                .route(
                    "/api/v1/auth/totp/disable",
                    post(disable_totp).layer(totp_body_limit()),
                )
                .route("/api/v1/profile", get(get_profile))
                .route("/api/v1/oauth2/bindings", get(list_oauth_bindings))
                .route("/api/v1/oauth2/:provider/bind", post(start_oauth_bind))
                .route(
                    "/api/v1/oauth2/:provider/unbind",
                    post(unbind_oauth_provider),
                )
                .route("/api/v1/users", get(list_users))
                .route("/api/v1/users", post(create_user).layer(auth_body_limit()))
                .route(
                    "/api/v1/users/:id",
                    post(update_user).layer(auth_body_limit()),
                )
                .route("/api/v1/users/:id", axum::routing::delete(delete_user))
                .route("/api/v1/sessions", get(list_sessions))
                .route(
                    "/api/v1/sessions/:id",
                    axum::routing::delete(delete_session),
                )
                .route("/api/v1/waf/bans", get(list_waf_bans))
                .route(
                    "/api/v1/waf/bans",
                    post(create_waf_bans).layer(auth_body_limit()),
                )
                .route(
                    "/api/v1/waf/bans/:id",
                    axum::routing::delete(delete_waf_ban),
                )
                .route("/api/v1/maintenance/status", get(maintenance_status))
                .route("/api/v1/maintenance/backup", post(download_backup))
                .route("/api/v1/maintenance/archive", post(download_archive))
                .route(
                    "/api/v1/maintenance/restore",
                    post(restore_backup).layer(restore_body_limit()),
                )
                .route("/api/v1/maintenance/sqlite-vacuum", post(vacuum_sqlite))
                .route("/api/v1/maintenance/tsdb-compact", post(compact_tsdb))
                .route(
                    "/api/v1/maintenance/tsdb-retention",
                    post(update_tsdb_retention).layer(maintenance_action_body_limit()),
                )
                .route(
                    "/api/v1/cloudflared/status",
                    get(api::v1::cloudflared::cloudflared_status),
                )
                .route(
                    "/api/v1/cloudflared/token",
                    post(api::v1::cloudflared::save_cloudflared_token)
                        .layer(cloudflared_token_body_limit()),
                )
                .route(
                    "/api/v1/cloudflared/start",
                    post(api::v1::cloudflared::start_cloudflared),
                )
                .route(
                    "/api/v1/cloudflared/stop",
                    post(api::v1::cloudflared::stop_cloudflared),
                )
                .route("/api/v1/geoip/status", get(geoip_status))
                .route(
                    "/api/v1/geoip/test",
                    post(test_geoip).layer(geoip_test_body_limit()),
                )
                .route(
                    "/api/v1/geoip/update",
                    post(update_geoip_database).layer(geoip_update_body_limit()),
                )
                .route(
                    "/api/v1/geoip/upload",
                    post(upload_geoip_database).layer(geoip_upload_body_limit()),
                )
                .route("/api/v1/settings", get(get_settings))
                .route(
                    "/api/v1/settings",
                    post(update_settings)
                        .patch(update_settings)
                        .layer(settings_body_limit()),
                )
                .route("/api/v1/themes", get(list_themes))
                .route(
                    "/api/v1/themes/import",
                    post(import_theme)
                        .put(import_theme)
                        .layer(theme_body_limit()),
                )
                .route(
                    "/api/v1/themes/:id",
                    post(update_theme)
                        .patch(update_theme)
                        .delete(delete_theme)
                        .layer(theme_body_limit()),
                )
                .route(
                    "/api/v1/themes/:id/select",
                    post(select_theme).layer(theme_body_limit()),
                )
                .route("/api/v1/tokens", post(create_pat).layer(pat_body_limit()))
                .route("/api/v1/tokens", get(list_pats))
                .route("/api/v1/tokens/:id", axum::routing::delete(revoke_pat))
                .route(
                    "/api/v1/enrollment-tokens",
                    post(create_enrollment_token).layer(agent_auth_body_limit()),
                )
                .route(
                    "/api/v1/agents/:id/revoke",
                    post(api::v1::agent::revoke_agent),
                )
                .route("/api/v1/mcp/tools", get(list_mcp_tools))
                .route(
                    "/api/v1/mcp/execute",
                    post(execute_mcp_tool).layer(mcp_body_limit()),
                )
                .route("/api/v1/mcp/info", get(get_mcp_info))
                .route("/mcp", post(handle_mcp_jsonrpc).layer(mcp_body_limit()))
                .route("/api/v1/services", get(list_services))
                .route(
                    "/api/v1/services",
                    post(create_service).layer(service_body_limit()),
                )
                .route(
                    "/api/v1/services/test-probe",
                    post(test_probe).layer(service_body_limit()),
                )
                .route("/api/v1/services/:id", get(get_service))
                .route(
                    "/api/v1/services/:id",
                    post(update_service).layer(service_body_limit()),
                )
                .route("/api/v1/services/:id", delete(delete_service))
                .route("/api/v1/services/:id/history", get(get_service_history))
                .route("/api/v1/services/:id/uptime", get(get_service_uptime))
                .route(
                    "/api/v1/alert-rules",
                    post(create_alert_rule).layer(alert_body_limit()),
                )
                .route("/api/v1/alert-rules", get(list_alert_rules))
                .route(
                    "/api/v1/alert-rules/:id",
                    axum::routing::delete(delete_alert_rule),
                )
                .route("/api/v1/alert-events", get(list_alert_events))
                // Notifications
                .route("/api/v1/notifications", get(list_notifications))
                .route(
                    "/api/v1/notifications",
                    post(create_notification).layer(notification_body_limit()),
                )
                .route(
                    "/api/v1/notifications/:id",
                    post(update_notification)
                        .patch(update_notification)
                        .layer(notification_body_limit()),
                )
                .route("/api/v1/notifications/:id", delete(delete_notification))
                .route("/api/v1/notifications/:id/test", post(test_notification))
                .route("/api/v1/notification-groups", get(list_notification_groups))
                .route(
                    "/api/v1/notification-groups",
                    post(create_notification_group).layer(notification_body_limit()),
                )
                .route(
                    "/api/v1/notification-groups/:id",
                    post(update_notification_group)
                        .patch(update_notification_group)
                        .layer(notification_body_limit()),
                )
                .route(
                    "/api/v1/notification-groups/:id",
                    delete(delete_notification_group),
                )
                .route(
                    "/api/v1/notification-groups/:id/members",
                    post(add_notification_group_member).layer(notification_body_limit()),
                )
                .route(
                    "/api/v1/notification-groups/:id/members/:notification_id",
                    delete(delete_notification_group_member),
                )
                .route(
                    "/api/v1/notification-providers",
                    get(list_notification_providers),
                )
                // M6: DDNS config + history endpoints
                .route(
                    "/api/v1/ddns/configs",
                    post(create_ddns_config).layer(ddns_body_limit()),
                )
                .route("/api/v1/ddns/configs", get(list_ddns_configs))
                .route(
                    "/api/v1/ddns/configs/:id",
                    axum::routing::delete(delete_ddns_config),
                )
                .route("/api/v1/ddns/configs/:id/history", get(list_ddns_history))
                .route(
                    "/api/v1/ddns/reload",
                    axum::routing::post(reload_ddns_providers),
                )
                .route(
                    "/api/v1/ddns/check-now",
                    axum::routing::post(check_ddns_now),
                )
                // M3 server listing and metrics endpoints
                .route("/api/v1/servers", get(api::v1::servers::list_servers))
                .route(
                    "/api/v1/servers/batch",
                    post(api::v1::servers::batch_update_servers)
                        .layer(server_management_body_limit()),
                )
                .route(
                    "/api/v1/server-transfers",
                    get(api::v1::servers::list_server_owner_transfers),
                )
                .route(
                    "/api/v1/server-transfers/:id/retry",
                    post(api::v1::servers::retry_server_owner_transfer),
                )
                .route(
                    "/api/v1/server-transfers/:id/cancel",
                    post(api::v1::servers::cancel_server_owner_transfer),
                )
                .route(
                    "/api/v1/transfers/temp/tokens",
                    get(list_temporary_transfers),
                )
                .route(
                    "/api/v1/transfers/temp/tokens/:id/revoke",
                    post(revoke_temporary_transfer),
                )
                .route(
                    "/api/v1/server-groups",
                    get(api::v1::servers::list_server_groups),
                )
                .route(
                    "/api/v1/server-groups",
                    post(api::v1::servers::create_server_group)
                        .layer(server_management_body_limit()),
                )
                .route(
                    "/api/v1/server-groups/:id",
                    post(api::v1::servers::update_server_group)
                        .patch(api::v1::servers::update_server_group)
                        .layer(server_management_body_limit()),
                )
                .route(
                    "/api/v1/server-groups/:id",
                    delete(api::v1::servers::delete_server_group),
                )
                .route(
                    "/api/v1/server-groups/:id/members",
                    post(api::v1::servers::add_server_group_members)
                        .layer(server_management_body_limit()),
                )
                .route(
                    "/api/v1/server-groups/:id/members/:server_id",
                    delete(api::v1::servers::delete_server_group_member),
                )
                .route("/api/v1/servers/:id", get(api::v1::servers::get_server))
                .route(
                    "/api/v1/servers/:id",
                    post(api::v1::servers::update_server).layer(server_management_body_limit()),
                )
                .route(
                    "/api/v1/servers/:id/metrics",
                    get(api::v1::servers::get_server_metrics),
                )
                .route(
                    "/api/v1/servers/:id/files",
                    post(list_files).layer(server_ops_body_limit()),
                )
                .route(
                    "/api/v1/servers/:id/files/read",
                    post(read_file).layer(server_ops_body_limit()),
                )
                .route(
                    "/api/v1/servers/:id/files/write",
                    post(write_file).layer(server_ops_body_limit()),
                )
                .route(
                    "/api/v1/servers/:id/files/delete",
                    post(delete_file).layer(server_ops_body_limit()),
                )
                .route(
                    "/api/v1/servers/:id/files/download-url",
                    post(download_url).layer(server_ops_body_limit()),
                )
                .route(
                    "/api/v1/servers/:id/files/upload-url",
                    post(upload_url).layer(server_ops_body_limit()),
                )
                .route("/api/v1/servers/:id/config", get(get_config))
                .route(
                    "/api/v1/servers/:id/config",
                    post(apply_config).layer(server_ops_body_limit()),
                )
                .route(
                    "/api/v1/servers/:id/force-update",
                    post(force_update).layer(server_ops_body_limit()),
                )
                .route("/ws/servers", get(api::v1::servers::ws_servers))
                // Tasks
                .route("/api/v1/tasks", post(create_task).layer(task_body_limit()))
                .route("/api/v1/tasks", get(list_tasks))
                .route("/api/v1/tasks/:id", get(get_task))
                .route(
                    "/api/v1/tasks/:id",
                    post(update_task).layer(task_body_limit()),
                )
                .route("/api/v1/tasks/:id", delete(delete_task))
                .route("/api/v1/tasks/:id/run", post(run_task))
                .route("/api/v1/tasks/:id/runs", get(get_task_runs))
                .route(
                    "/api/v1/terminal/sessions",
                    post(create_terminal_session).layer(terminal_body_limit()),
                )
                .route("/ws/terminal/:session_id", get(ws_terminal))
                // NAT
                .route(
                    "/api/v1/nat/mappings",
                    post(create_nat_mapping).layer(nat_body_limit()),
                )
                .route("/api/v1/nat/mappings/all", get(list_all_nat_mappings))
                .route(
                    "/api/v1/nat/mappings/agent/:agent_id",
                    get(list_nat_mappings),
                )
                .route("/api/v1/nat/mappings/:id", get(get_nat_mapping))
                .route(
                    "/api/v1/nat/mappings/:id",
                    post(update_nat_mapping).layer(nat_body_limit()),
                )
                .route("/api/v1/nat/mappings/:id", delete(delete_nat_mapping))
                .route_layer(middleware::from_fn_with_state(
                    state.clone(),
                    session_middleware,
                ));

            let app = Router::new()
                .route("/healthz", get(healthz))
                .route("/install-agent.sh", get(install_agent_script))
                .route("/api/v1/agents/install.sh", get(install_agent_script))
                .route("/api/v1/auth/login", post(login).layer(login_body_limit()))
                .route("/api/v1/openapi.json", get(openapi_json))
                .route("/api/v1/oauth2/providers", get(list_oauth_providers))
                .route("/api/v1/oauth2/:provider", get(start_oauth_login))
                .route("/api/v1/oauth2/callback", get(oauth_callback))
                .route("/api/v1/public/status", get(api::v1::public::public_status))
                .route(
                    "/api/v1/public/mjpeg",
                    get(api::v1::public::public_status_mjpeg),
                )
                .route(
                    "/api/v1/public/servers/:id",
                    get(api::v1::public::public_server_detail),
                )
                .route(
                    "/api/v1/agents/enroll",
                    post(enroll).layer(agent_auth_body_limit()),
                )
                .route("/api/v1/transfers/temp/download", get(temp_download))
                .route(
                    "/api/v1/transfers/temp/upload",
                    axum::routing::put(temp_upload).layer(upload_body_limit()),
                )
                .route(
                    "/api/v1/agents/jwt/challenge",
                    post(get_agent_jwt_challenge).layer(agent_auth_body_limit()),
                )
                .route(
                    "/api/v1/agents/jwt",
                    post(get_agent_jwt).layer(agent_auth_body_limit()),
                )
                .merge(protected)
                .with_state(state)
                .layer(middleware::from_fn(limit_http_request_target_length))
                .layer(middleware::from_fn(limit_http_query_length))
                .layer(cors);

            let addr: SocketAddr = bind.parse()?;
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .with_context(|| format!("failed to bind HTTP server to {addr}"))?;

            tracing::info!("HTTP server listening on {}", addr);
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("HTTP server error: {}", e))
        }

        run_http_server(http_bind, state).await
    });

    // Hand the (already built) registries to the agent
    // revoke handler so /api/v1/agents/:id/revoke can find the
    // matching live session and IO stream.
    api::v1::agent::set_revoke_registry(
        Arc::new(session_registry.clone()),
        Arc::new(io_registry.clone()),
    );

    // Start gRPC server
    let grpc_config = config.clone();
    let grpc_db = db.clone();
    let grpc_session_registry = session_registry.clone();
    let grpc_metrics = metrics.clone();
    let grpc_realtime = realtime.clone();
    let grpc_io_registry = io_registry.clone();
    let grpc_handle = tokio::spawn(async move {
        async fn run_grpc_server(
            config: config::Config,
            db: DatabaseBackend,
            session_registry: grpc::SessionRegistry,
            metrics: xlstatus_tsdb::MetricStore,
            realtime: crate::realtime::BroadcastHub,
            io_registry: grpc::IoRegistry,
        ) -> anyhow::Result<()> {
            let bind = config.server.grpc_bind.clone();
            let addr: SocketAddr = bind.parse()?;
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .with_context(|| format!("failed to bind gRPC server to {addr}"))?;

            let server_tls_config = build_grpc_server_tls_config(&config).await?;
            let agent_service = grpc::AgentServiceImpl::new(
                db,
                session_registry,
                config.security.session_secret,
                metrics,
                realtime,
                io_registry,
            );
            let reflection_service = if config.server.grpc_reflection_enabled {
                Some(
                    tonic_reflection::server::Builder::configure()
                        .register_encoded_file_descriptor_set(
                            xlstatus_proto_gen::xlstatus::v1::FILE_DESCRIPTOR_SET,
                        )
                        .build_v1()
                        .map_err(|e| {
                            anyhow::anyhow!("Failed to build reflection service: {}", e)
                        })?,
                )
            } else {
                None
            };

            let agent_service = AgentServiceServer::new(agent_service)
                .max_decoding_message_size(GRPC_MESSAGE_LIMIT)
                .max_encoding_message_size(GRPC_MESSAGE_LIMIT);

            tracing::info!("gRPC server listening on {}", addr);
            let mut server = TonicServer::builder();
            if let Some(tls_config) = server_tls_config {
                server = server
                    .tls_config(tls_config)
                    .context("failed to configure gRPC TLS")?;
                tracing::info!("gRPC TLS enabled");
            }
            server
                .add_service(agent_service)
                .add_optional_service(reflection_service)
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await
                .map_err(|e| anyhow::anyhow!("gRPC server error: {}", e))
        }

        run_grpc_server(
            grpc_config,
            grpc_db,
            grpc_session_registry,
            grpc_metrics,
            grpc_realtime,
            grpc_io_registry,
        )
        .await
    });

    // Both listeners are long-running. If either task returns, surface the
    // actual inner server error and stop the sibling task instead of silently
    // dropping back to the shell.
    let mut http_handle = http_handle;
    let mut grpc_handle = grpc_handle;
    let result = tokio::select! {
        res = &mut http_handle => {
            grpc_handle.abort();
            server_task_result("HTTP", res)
        }
        res = &mut grpc_handle => {
            http_handle.abort();
            server_task_result("gRPC", res)
        }
    };

    if let Err(e) = &result {
        tracing::error!("{:#}", e);
    }

    result
}

async fn healthz() -> &'static str {
    "OK"
}

async fn build_grpc_server_tls_config(
    config: &config::Config,
) -> anyhow::Result<Option<ServerTlsConfig>> {
    let cert_path = non_empty_config_path(&config.server.grpc_tls_cert_path);
    let key_path = non_empty_config_path(&config.server.grpc_tls_key_path);
    let client_ca_path = non_empty_config_path(&config.server.grpc_tls_client_ca_path);

    let (cert_path, key_path) = match (cert_path, key_path) {
        (None, None) => {
            if client_ca_path.is_some() {
                anyhow::bail!(
                    "GRPC_TLS_CLIENT_CA_PATH requires GRPC_TLS_CERT_PATH and GRPC_TLS_KEY_PATH"
                );
            }
            return Ok(None);
        }
        (Some(cert_path), Some(key_path)) => (cert_path, key_path),
        _ => {
            anyhow::bail!("GRPC_TLS_CERT_PATH and GRPC_TLS_KEY_PATH must be configured together");
        }
    };

    let cert = tokio::fs::read(cert_path)
        .await
        .with_context(|| format!("failed to read gRPC TLS certificate from {cert_path}"))?;
    let key = tokio::fs::read(key_path)
        .await
        .with_context(|| format!("failed to read gRPC TLS private key from {key_path}"))?;
    let mut tls_config = ServerTlsConfig::new().identity(Identity::from_pem(cert, key));

    if let Some(client_ca_path) = client_ca_path {
        let client_ca = tokio::fs::read(client_ca_path)
            .await
            .with_context(|| format!("failed to read gRPC mTLS client CA from {client_ca_path}"))?;
        tls_config = tls_config.client_ca_root(Certificate::from_pem(client_ca));
        tracing::info!("gRPC mTLS client certificate verification enabled");
    }

    Ok(Some(tls_config))
}

fn non_empty_config_path(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[derive(Debug, serde::Deserialize)]
struct AgentInstallQuery {
    server_url: Option<String>,
    grpc_server: Option<String>,
    grpc_tls_ca_path: Option<String>,
    grpc_tls_domain_name: Option<String>,
    grpc_tls_client_cert_path: Option<String>,
    grpc_tls_client_key_path: Option<String>,
    enrollment_token: Option<String>,
    agent_name: Option<String>,
    version: Option<String>,
}

async fn install_agent_script(
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let request_scheme =
        crate::security::forwarded_proto_from_headers_and_peer(&headers, Some(peer_addr))
            .unwrap_or("http");
    install_agent_script_response(
        parse_agent_install_query(&uri)
            .and_then(|query| build_install_agent_script(&headers, request_scheme, query)),
    )
}

fn install_agent_script_response(result: Result<String, String>) -> Response {
    match result {
        Ok(body) => (
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/x-shellscript; charset=utf-8"),
                ),
                (
                    header::CONTENT_DISPOSITION,
                    HeaderValue::from_static("attachment; filename=\"install-agent.sh\""),
                ),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
                (header::PRAGMA, HeaderValue::from_static("no-cache")),
                (header::EXPIRES, HeaderValue::from_static("0")),
            ],
            body,
        )
            .into_response(),
        Err(message) => (
            StatusCode::BAD_REQUEST,
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/plain; charset=utf-8"),
                ),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
                (header::PRAGMA, HeaderValue::from_static("no-cache")),
                (header::EXPIRES, HeaderValue::from_static("0")),
            ],
            message,
        )
            .into_response(),
    }
}

fn parse_agent_install_query(uri: &Uri) -> Result<AgentInstallQuery, String> {
    let raw_query = uri.query().unwrap_or_default();
    if raw_query.len() > INSTALL_BOOTSTRAP_MAX_QUERY_BYTES {
        return Err(format!(
            "install query must be at most {INSTALL_BOOTSTRAP_MAX_QUERY_BYTES} bytes"
        ));
    }
    Query::<AgentInstallQuery>::try_from_uri(uri)
        .map(|Query(query)| query)
        .map_err(|_| "install query is invalid".to_string())
}

fn build_install_agent_script(
    headers: &HeaderMap,
    request_scheme: &str,
    query: AgentInstallQuery,
) -> Result<String, String> {
    let requested_version = query
        .version
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DEFAULT_AGENT_INSTALL_VERSION);
    let version = if valid_release_version(requested_version) {
        requested_version
    } else {
        DEFAULT_AGENT_INSTALL_VERSION
    };
    let script_url = format!(
        "https://github.com/lbyxiaolizi/XLStatus/releases/download/{version}/install-agent.sh"
    );
    let request_authority = request_authority(headers)?;
    let request_host = request_authority.as_deref().and_then(authority_hostname);
    let server_url = normalize_install_control_url(
        query.server_url.as_deref(),
        request_host.as_deref(),
        request_authority.as_deref(),
        request_scheme,
        "server_url",
    )?;
    let grpc_server = normalize_install_grpc_url(
        query.grpc_server.as_deref(),
        &server_url,
        request_host.as_deref(),
    )?;
    let grpc_tls_ca_path = normalize_optional_install_text(
        query.grpc_tls_ca_path.as_deref(),
        INSTALL_BOOTSTRAP_MAX_TLS_PATH_BYTES,
        "grpc_tls_ca_path",
    )?;
    let grpc_tls_domain_name = normalize_optional_install_text(
        query.grpc_tls_domain_name.as_deref(),
        INSTALL_BOOTSTRAP_MAX_TLS_DOMAIN_BYTES,
        "grpc_tls_domain_name",
    )?;
    let grpc_tls_client_cert_path = normalize_optional_install_text(
        query.grpc_tls_client_cert_path.as_deref(),
        INSTALL_BOOTSTRAP_MAX_TLS_PATH_BYTES,
        "grpc_tls_client_cert_path",
    )?;
    let grpc_tls_client_key_path = normalize_optional_install_text(
        query.grpc_tls_client_key_path.as_deref(),
        INSTALL_BOOTSTRAP_MAX_TLS_PATH_BYTES,
        "grpc_tls_client_key_path",
    )?;
    let enrollment_token = normalize_optional_install_text(
        query.enrollment_token.as_deref(),
        INSTALL_BOOTSTRAP_MAX_ENROLLMENT_TOKEN_BYTES,
        "enrollment_token",
    )?
    .unwrap_or("");
    let agent_name = normalize_optional_install_text(
        query.agent_name.as_deref(),
        INSTALL_BOOTSTRAP_MAX_AGENT_NAME_BYTES,
        "agent_name",
    )?
    .unwrap_or("$(hostname)");
    let agent_name_line = if agent_name == "$(hostname)" {
        r#"export AGENT_NAME="$(hostname)""#.to_string()
    } else {
        format!("export AGENT_NAME={}", shell_quote(agent_name))
    };
    let script_url_block = if version == "latest" {
        r#"LATEST_RELEASE_API="https://api.github.com/repos/lbyxiaolizi/XLStatus/releases?per_page=20"
VERSION="$(curl -fsSL "$LATEST_RELEASE_API" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
if [ -z "$VERSION" ]; then
  echo "Failed to resolve latest XLStatus release" >&2
  exit 1
fi
SCRIPT_URL="https://github.com/lbyxiaolizi/XLStatus/releases/download/${VERSION}/install-agent.sh""#
            .to_string()
    } else {
        format!("SCRIPT_URL={}", shell_quote(&script_url))
    };
    Ok(format!(
        r#"#!/bin/bash
set -e

export VERSION={version}
export SERVER_URL={server_url}
export GRPC_SERVER={grpc_server}
{grpc_tls_ca_path_line}
{grpc_tls_domain_name_line}
{grpc_tls_client_cert_path_line}
{grpc_tls_client_key_path_line}
export ENROLLMENT_TOKEN={enrollment_token}
{agent_name_line}

if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required to download the XLStatus Agent installer" >&2
  exit 1
fi

{script_url_block}

curl -fsSL "$SCRIPT_URL" | bash
"#,
        version = shell_quote(version),
        server_url = shell_quote(&server_url),
        grpc_server = shell_quote(&grpc_server),
        grpc_tls_ca_path_line = optional_export_line("GRPC_TLS_CA_PATH", grpc_tls_ca_path),
        grpc_tls_domain_name_line =
            optional_export_line("GRPC_TLS_DOMAIN_NAME", grpc_tls_domain_name),
        grpc_tls_client_cert_path_line =
            optional_export_line("GRPC_TLS_CLIENT_CERT_PATH", grpc_tls_client_cert_path),
        grpc_tls_client_key_path_line =
            optional_export_line("GRPC_TLS_CLIENT_KEY_PATH", grpc_tls_client_key_path),
        enrollment_token = shell_quote(enrollment_token),
        agent_name_line = agent_name_line,
        script_url_block = script_url_block,
    ))
}

fn optional_export_line(name: &str, value: Option<&str>) -> String {
    value
        .map(|value| format!("export {name}={}", shell_quote(value)))
        .unwrap_or_default()
}

fn valid_release_version(value: &str) -> bool {
    value == "latest"
        || (!value.is_empty()
            && value.len() <= 80
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
}

fn request_authority(headers: &HeaderMap) -> Result<Option<String>, String> {
    let Some(value) = headers.get(header::HOST) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| "Host header is invalid".to_string())?
        .trim();
    if value.is_empty() {
        return Ok(None);
    }
    ensure_install_text_size(value, 1, INSTALL_BOOTSTRAP_MAX_HOST_BYTES, "Host header")?;
    if value.chars().any(char::is_control) {
        return Err("Host header must not contain control characters".to_string());
    }
    if value.contains('\\') {
        return Err("Host header must not contain backslashes".to_string());
    }
    let url =
        reqwest::Url::parse(&format!("http://{value}")).map_err(|_| "Host header is invalid")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err("Host header must be a plain authority".to_string());
    }
    Ok(Some(url_origin_authority(&url)))
}

fn normalize_install_control_url(
    value: Option<&str>,
    request_host: Option<&str>,
    request_authority: Option<&str>,
    request_scheme: &str,
    field: &str,
) -> Result<String, String> {
    if request_host.is_none()
        && value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
    {
        return Err(format!("{field} requires a Host header for validation"));
    }
    let raw = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| request_authority.map(|authority| format!("{request_scheme}://{authority}")))
        .unwrap_or_else(|| "http://localhost:8080".to_string());
    ensure_install_text_size(&raw, 1, INSTALL_BOOTSTRAP_MAX_URL_BYTES, field)?;
    let url = parse_install_endpoint_url(&raw, field)?;
    if let Some(host) = request_host {
        ensure_install_url_host(&url, host, field)?;
    }
    Ok(url_origin(&url))
}

fn normalize_install_grpc_url(
    value: Option<&str>,
    server_url: &str,
    request_host: Option<&str>,
) -> Result<String, String> {
    if request_host.is_none()
        && value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
    {
        return Err("grpc_server requires a Host header for validation".to_string());
    }
    let server = reqwest::Url::parse(server_url)
        .map_err(|e| format!("server_url is invalid after normalization: {e}"))?;
    let raw = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_grpc_endpoint(&server));
    ensure_install_text_size(&raw, 1, INSTALL_BOOTSTRAP_MAX_URL_BYTES, "grpc_server")?;
    let url = parse_install_endpoint_url(&raw, "grpc_server")?;
    if let Some(host) = request_host {
        ensure_install_url_host(&url, host, "grpc_server")?;
    } else if let Some(server_host) = server.host_str() {
        ensure_install_url_host(&url, server_host, "grpc_server")?;
    }
    Ok(url_origin(&url))
}

fn normalize_optional_install_text<'a>(
    value: Option<&'a str>,
    max_bytes: usize,
    field: &str,
) -> Result<Option<&'a str>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    ensure_install_text_size(value, 1, max_bytes, field)?;
    if value.chars().any(char::is_control) {
        return Err(format!("{field} must not contain control characters"));
    }
    Ok(Some(value))
}

fn ensure_install_text_size(
    value: &str,
    min_bytes: usize,
    max_bytes: usize,
    field: &str,
) -> Result<(), String> {
    let len = value.len();
    if len < min_bytes || len > max_bytes {
        return Err(format!(
            "{field} must be between {min_bytes} and {max_bytes} bytes"
        ));
    }
    Ok(())
}

fn parse_install_endpoint_url(value: &str, field: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|e| format!("{field} is invalid: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("{field} must use http or https"));
    }
    if url.host_str().is_none() {
        return Err(format!("{field} must include a host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{field} must not include userinfo"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(format!("{field} must not include query or fragment"));
    }
    if !matches!(url.path(), "" | "/") {
        return Err(format!("{field} must be an origin URL without a path"));
    }
    Ok(url)
}

fn ensure_install_url_host(
    url: &reqwest::Url,
    expected_host: &str,
    field: &str,
) -> Result<(), String> {
    let Some(host) = url.host_str() else {
        return Err(format!("{field} must include a host"));
    };
    if host.eq_ignore_ascii_case(expected_host) {
        Ok(())
    } else {
        Err(format!("{field} host must match this XLStatus server host"))
    }
}

fn authority_hostname(authority: &str) -> Option<String> {
    let url = reqwest::Url::parse(&format!("http://{authority}")).ok()?;
    url.host_str().map(|host| host.to_ascii_lowercase())
}

fn url_origin(url: &reqwest::Url) -> String {
    format!("{}://{}", url.scheme(), url_origin_authority(url))
}

fn url_origin_authority(url: &reqwest::Url) -> String {
    let host = url.host_str().expect("validated URL has host");
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

fn default_grpc_endpoint(server: &reqwest::Url) -> String {
    let mut url = server.clone();
    let _ = url.set_port(Some(50051));
    url_origin(&url)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn build_cors_layer(allowed_origins: &[String]) -> anyhow::Result<CorsLayer> {
    let origins = allowed_origins
        .iter()
        .map(|origin| {
            if origin == "*" {
                return Err(anyhow::anyhow!(
                    "CORS wildcard origins are not supported because cookie credentials are enabled"
                ));
            }
            HeaderValue::from_str(origin)
                .map_err(|e| anyhow::anyhow!("Invalid CORS origin '{}': {}", origin, e))
        })
        .collect::<Result<Vec<_>, _>>()?;

    tracing::info!("CORS allowed origins: {}", allowed_origins.join(", "));

    Ok(CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            HeaderName::from_static("x-csrf-token"),
        ])
        .allow_credentials(true))
}

async fn limit_http_query_length(request: Request, next: Next) -> Response {
    match ensure_http_query_length(request.uri()) {
        Ok(()) => next.run(request).await,
        Err(message) => (
            StatusCode::URI_TOO_LONG,
            [(header::CONTENT_TYPE, "application/json")],
            format!(r#"{{"success":false,"error":"{message}"}}"#),
        )
            .into_response(),
    }
}

fn ensure_http_query_length(uri: &Uri) -> Result<(), String> {
    let raw_query = uri.query().unwrap_or_default();
    if raw_query.len() > HTTP_MAX_QUERY_BYTES {
        return Err(format!(
            "query string must be at most {HTTP_MAX_QUERY_BYTES} bytes"
        ));
    }
    Ok(())
}

async fn limit_http_request_target_length(request: Request, next: Next) -> Response {
    match ensure_http_path_length(request.uri()) {
        Ok(()) => next.run(request).await,
        Err(message) => (
            StatusCode::URI_TOO_LONG,
            [(header::CONTENT_TYPE, "application/json")],
            format!(r#"{{"success":false,"error":"{message}"}}"#),
        )
            .into_response(),
    }
}

fn ensure_http_path_length(uri: &Uri) -> Result<(), String> {
    let raw_path = uri.path();
    if raw_path.len() > HTTP_MAX_PATH_BYTES {
        return Err(format!("path must be at most {HTTP_MAX_PATH_BYTES} bytes"));
    }
    Ok(())
}

fn server_task_result(
    name: &str,
    result: Result<anyhow::Result<()>, tokio::task::JoinError>,
) -> anyhow::Result<()> {
    match result {
        Ok(Ok(())) => Err(anyhow::anyhow!("{name} server exited unexpectedly")),
        Ok(Err(e)) => Err(e.context(format!("{name} server failed"))),
        Err(e) => Err(anyhow::anyhow!("{name} server task failed: {e}")),
    }
}

async fn seed_admin_user(db: &DatabaseBackend) -> anyhow::Result<()> {
    let username =
        std::env::var("XLSTATUS_SEED_ADMIN_USERNAME").or_else(|_| std::env::var("ADMIN_USERNAME"));
    let password = seed_admin_password_from_environment()?;

    let (Ok(username), Some(password)) = (username, password) else {
        return Ok(());
    };

    let repo = UserRepository::new(db.clone());
    if repo.find_by_username(&username).await?.is_some() {
        return Ok(());
    }

    repo.create(CreateUserInput {
        username: username.clone(),
        password,
        role: UserRole::Admin,
    })
    .await?;
    tracing::info!("Seeded admin user '{}'", username);
    Ok(())
}

fn seed_admin_password_from_environment() -> anyhow::Result<Option<String>> {
    match std::env::var("XLSTATUS_SEED_ADMIN_PASSWORD_FILE")
        .or_else(|_| std::env::var("ADMIN_PASSWORD_FILE"))
    {
        Ok(path) if !path.trim().is_empty() => {
            return read_seed_admin_password_file(path.trim()).map(Some);
        }
        Ok(_) | Err(std::env::VarError::NotPresent) => {}
        Err(err) => return Err(anyhow::anyhow!(err)),
    }

    match std::env::var("XLSTATUS_SEED_ADMIN_PASSWORD").or_else(|_| std::env::var("ADMIN_PASSWORD"))
    {
        Ok(password) => Ok(Some(password)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(err) => Err(anyhow::anyhow!(err)),
    }
}

fn read_seed_admin_password_file(path: &str) -> anyhow::Result<String> {
    let mut password = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read seed admin password file {path}"))?;

    while matches!(password.as_bytes().last(), Some(b'\n' | b'\r')) {
        password.pop();
    }

    if password.is_empty() {
        anyhow::bail!("seed admin password file is empty");
    }

    Ok(password)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_host(host: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        headers
    }

    fn install_query(server_url: Option<&str>, grpc_server: Option<&str>) -> AgentInstallQuery {
        AgentInstallQuery {
            server_url: server_url.map(str::to_string),
            grpc_server: grpc_server.map(str::to_string),
            grpc_tls_ca_path: None,
            grpc_tls_domain_name: None,
            grpc_tls_client_cert_path: None,
            grpc_tls_client_key_path: None,
            enrollment_token: Some(format!("xle_{}", "a".repeat(64))),
            agent_name: Some("$(hostname)".into()),
            version: Some(DEFAULT_AGENT_INSTALL_VERSION.into()),
        }
    }

    #[test]
    fn install_bootstrap_query_resource_budget_is_bounded() {
        assert_eq!(INSTALL_BOOTSTRAP_MAX_QUERY_BYTES, 16 * 1024);
        assert_eq!(INSTALL_BOOTSTRAP_MAX_HOST_BYTES, 512);
        assert_eq!(INSTALL_BOOTSTRAP_MAX_URL_BYTES, 2048);
        assert_eq!(INSTALL_BOOTSTRAP_MAX_TLS_PATH_BYTES, 1024);
        assert_eq!(INSTALL_BOOTSTRAP_MAX_TLS_DOMAIN_BYTES, 253);
        assert_eq!(INSTALL_BOOTSTRAP_MAX_ENROLLMENT_TOKEN_BYTES, 128);
        assert_eq!(INSTALL_BOOTSTRAP_MAX_AGENT_NAME_BYTES, 255);

        let uri: Uri = format!("/install-agent.sh?agent_name={}", "a".repeat(16 * 1024))
            .parse()
            .unwrap();
        let err = parse_agent_install_query(&uri).unwrap_err();
        assert!(err.contains("install query must be at most"));
    }

    #[test]
    fn global_http_query_resource_budget_is_bounded() {
        assert_eq!(HTTP_MAX_QUERY_BYTES, 16 * 1024);

        let uri: Uri = "/api/v1/servers?limit=50&offset=0".parse().unwrap();
        assert!(ensure_http_query_length(&uri).is_ok());

        let uri: Uri = format!("/api/v1/servers?x={}", "a".repeat(HTTP_MAX_QUERY_BYTES))
            .parse()
            .unwrap();
        let err = ensure_http_query_length(&uri).unwrap_err();
        assert!(err.contains("query string must be at most"));
    }

    #[test]
    fn global_http_path_resource_budget_is_bounded() {
        assert_eq!(HTTP_MAX_PATH_BYTES, 4096);

        let uri: Uri = "/api/v1/servers/00000000-0000-0000-0000-000000000001"
            .parse()
            .unwrap();
        assert!(ensure_http_path_length(&uri).is_ok());

        let uri: Uri = format!("/api/v1/servers/{}", "a".repeat(HTTP_MAX_PATH_BYTES))
            .parse()
            .unwrap();
        let err = ensure_http_path_length(&uri).unwrap_err();
        assert!(err.contains("path must be at most"));
    }

    #[test]
    fn install_bootstrap_response_is_not_cacheable() {
        let response = install_agent_script_response(Ok("body".into()));
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
        assert_eq!(response.headers().get(header::PRAGMA).unwrap(), "no-cache");
        assert_eq!(response.headers().get(header::EXPIRES).unwrap(), "0");
    }

    #[test]
    fn install_bootstrap_rejects_cross_host_control_urls() {
        let headers = headers_with_host("status.example.com");

        let err = build_install_agent_script(
            &headers,
            "http",
            install_query(Some("https://evil.example.com"), None),
        )
        .unwrap_err();
        assert!(err.contains("server_url host must match"));

        let err = build_install_agent_script(
            &headers,
            "http",
            install_query(
                Some("https://status.example.com"),
                Some("https://evil.example.com:50051"),
            ),
        )
        .unwrap_err();
        assert!(err.contains("grpc_server host must match"));
    }

    #[test]
    fn install_bootstrap_requires_host_header_for_explicit_urls() {
        let headers = HeaderMap::new();
        let err = build_install_agent_script(
            &headers,
            "http",
            install_query(Some("https://evil.example.com"), None),
        )
        .unwrap_err();
        assert!(err.contains("server_url requires a Host header"));
    }

    #[test]
    fn install_bootstrap_defaults_to_request_host() {
        let headers = headers_with_host("status.example.com");
        let body = build_install_agent_script(&headers, "http", install_query(None, None)).unwrap();

        assert!(body.contains("export SERVER_URL='http://status.example.com'"));
        assert!(body.contains("export GRPC_SERVER='http://status.example.com:50051'"));
    }

    #[test]
    fn install_bootstrap_default_server_url_preserves_request_host_port() {
        let headers = headers_with_host("status.example.com:8080");
        let body = build_install_agent_script(&headers, "http", install_query(None, None)).unwrap();

        assert!(body.contains("export SERVER_URL='http://status.example.com:8080'"));
        assert!(body.contains("export GRPC_SERVER='http://status.example.com:50051'"));
    }

    #[test]
    fn install_bootstrap_defaults_to_forwarded_https_request_scheme() {
        let headers = headers_with_host("status.example.com");
        let body =
            build_install_agent_script(&headers, "https", install_query(None, None)).unwrap();

        assert!(body.contains("export SERVER_URL='https://status.example.com'"));
        assert!(body.contains("export GRPC_SERVER='https://status.example.com:50051'"));
    }

    #[test]
    fn install_bootstrap_rejects_non_authority_host_header() {
        for host in [
            "user@status.example.com",
            "status.example.com/path",
            "status.example.com?x=1",
            "status.example.com#fragment",
            "status.example.com\\@evil.example",
        ] {
            let headers = headers_with_host(host);
            let err = build_install_agent_script(&headers, "http", install_query(None, None))
                .unwrap_err();
            assert!(
                err.contains("Host header"),
                "{host} should be rejected as invalid Host authority"
            );
        }
    }

    #[test]
    fn install_bootstrap_allows_same_host_different_control_ports() {
        let headers = headers_with_host("status.example.com");
        let body = build_install_agent_script(
            &headers,
            "http",
            install_query(
                Some("https://status.example.com:8443"),
                Some("https://status.example.com:50051"),
            ),
        )
        .unwrap();

        assert!(body.contains("export SERVER_URL='https://status.example.com:8443'"));
        assert!(body.contains("export GRPC_SERVER='https://status.example.com:50051'"));
    }

    #[test]
    fn install_bootstrap_rejects_url_paths_and_userinfo() {
        let headers = headers_with_host("status.example.com");

        let err = build_install_agent_script(
            &headers,
            "http",
            install_query(Some("https://status.example.com/path"), None),
        )
        .unwrap_err();
        assert!(err.contains("without a path"));

        let err = build_install_agent_script(
            &headers,
            "http",
            install_query(Some("https://user:pass@status.example.com"), None),
        )
        .unwrap_err();
        assert!(err.contains("must not include userinfo"));
    }

    #[test]
    fn install_bootstrap_rejects_oversized_reflected_fields() {
        let headers = headers_with_host("status.example.com");

        let mut query = install_query(None, None);
        query.enrollment_token = Some("x".repeat(INSTALL_BOOTSTRAP_MAX_ENROLLMENT_TOKEN_BYTES + 1));
        let err = build_install_agent_script(&headers, "http", query).unwrap_err();
        assert!(err.contains("enrollment_token must be between"));

        let mut query = install_query(None, None);
        query.agent_name = Some("a".repeat(INSTALL_BOOTSTRAP_MAX_AGENT_NAME_BYTES + 1));
        let err = build_install_agent_script(&headers, "http", query).unwrap_err();
        assert!(err.contains("agent_name must be between"));

        let mut query = install_query(None, None);
        query.grpc_tls_ca_path = Some("a".repeat(INSTALL_BOOTSTRAP_MAX_TLS_PATH_BYTES + 1));
        let err = build_install_agent_script(&headers, "http", query).unwrap_err();
        assert!(err.contains("grpc_tls_ca_path must be between"));

        let mut query = install_query(None, None);
        query.grpc_tls_domain_name = Some("a".repeat(INSTALL_BOOTSTRAP_MAX_TLS_DOMAIN_BYTES + 1));
        let err = build_install_agent_script(&headers, "http", query).unwrap_err();
        assert!(err.contains("grpc_tls_domain_name must be between"));
    }

    #[test]
    fn install_bootstrap_rejects_control_characters_in_reflected_fields() {
        let headers = headers_with_host("status.example.com");
        let mut query = install_query(None, None);
        query.agent_name = Some("agent\nname".into());

        let err = build_install_agent_script(&headers, "http", query).unwrap_err();
        assert!(err.contains("agent_name must not contain control characters"));
    }

    #[test]
    fn install_bootstrap_rejects_oversized_control_urls() {
        let headers = headers_with_host("status.example.com");
        let long_host = format!(
            "https://{}.status.example.com",
            "a".repeat(INSTALL_BOOTSTRAP_MAX_URL_BYTES)
        );

        let err =
            build_install_agent_script(&headers, "http", install_query(Some(&long_host), None))
                .unwrap_err();
        assert!(err.contains("server_url must be between"));
    }

    #[test]
    fn install_bootstrap_rejects_oversized_host_header() {
        let headers = headers_with_host(&format!(
            "{}.example.com",
            "a".repeat(INSTALL_BOOTSTRAP_MAX_HOST_BYTES)
        ));

        let err =
            build_install_agent_script(&headers, "http", install_query(None, None)).unwrap_err();
        assert!(err.contains("Host header must be between"));
    }

    #[test]
    fn server_install_script_does_not_persist_seed_admin_password_in_systemd_unit() {
        let script = include_str!("../../../deploy/install.sh");

        assert!(script.contains("BOOTSTRAP_ENV_FILE=\"/run/xlstatus/bootstrap.env\""));
        assert!(script.contains("EnvironmentFile=-$BOOTSTRAP_ENV_FILE"));
        assert!(script.contains("write_bootstrap_env_file"));
        assert!(script.contains("clear_bootstrap_env_after_seed"));
        assert!(script.contains("wait_for_healthz"));
        assert!(script.contains("before clearing secrets"));
        assert!(script.contains("systemctl restart xlstatus"));
        assert!(!script.contains("Environment=\"XLSTATUS_SEED_ADMIN_PASSWORD="));
        assert!(!script.contains("ADMIN_PASSWORD_SYSTEMD"));
    }

    #[test]
    fn seed_admin_password_file_trims_trailing_newlines() {
        let path = std::env::temp_dir().join(format!(
            "xlstatus-seed-admin-password-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        std::fs::write(&path, "  strong password  \r\n").unwrap();
        let password = read_seed_admin_password_file(path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(password, "  strong password  ");
    }

    #[test]
    fn seed_admin_password_file_rejects_empty_secret() {
        let path = std::env::temp_dir().join(format!(
            "xlstatus-empty-seed-admin-password-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        std::fs::write(&path, "\n").unwrap();
        let err = read_seed_admin_password_file(path.to_str().unwrap()).unwrap_err();
        let _ = std::fs::remove_file(&path);

        assert!(err.to_string().contains("password file is empty"));
    }

    #[test]
    fn compose_seed_admin_password_uses_secret_file() {
        for (name, compose) in [
            (
                "docker-compose.yml",
                include_str!("../../../docker-compose.yml"),
            ),
            (
                "docker-compose.pg.yml",
                include_str!("../../../docker-compose.pg.yml"),
            ),
            (
                "docker-compose.simple.yml",
                include_str!("../../../docker-compose.simple.yml"),
            ),
        ] {
            assert!(
                compose.contains(
                    "XLSTATUS_SEED_ADMIN_PASSWORD_FILE=/run/secrets/xlstatus_seed_admin_password"
                ),
                "{name} must point the server at the mounted seed admin password secret"
            );
            assert!(
                !compose.contains("XLSTATUS_SEED_ADMIN_PASSWORD=${"),
                "{name} must not persist the seed admin password in the container environment"
            );
            assert!(
                compose.contains("xlstatus_seed_admin_password:"),
                "{name} must define the seed admin password secret"
            );
            assert!(
                compose.contains("file: ./.secrets/xlstatus_seed_admin_password"),
                "{name} must load the seed admin password from the local secrets directory"
            );
        }

        let gitignore = include_str!("../../../.gitignore");
        assert!(gitignore.contains(".secrets/"));
    }

    #[test]
    fn postgres_compose_does_not_publish_database_publicly() {
        let compose = include_str!("../../../docker-compose.pg.yml");

        assert!(compose.contains("\"127.0.0.1:5432:5432\""));
        assert!(!compose.contains("\"5432:5432\""));
        assert!(!compose.contains("- 5432:5432"));
    }
}

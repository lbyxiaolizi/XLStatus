use axum::{
    middleware,
    routing::{delete, get, post},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::transport::Server as TonicServer;
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
mod security;
mod services;
mod tasks;

use crate::alerts::engine::AlertEngine;
use crate::db::{CreateUserInput, DatabaseBackend, UserRepository};
use crate::services::monitor::ServiceMonitor;
use api::v1::agent::{create_enrollment_token, enroll};
use api::v1::agent_jwt::{get_agent_jwt, get_agent_jwt_challenge};
use api::v1::alerts::{create_alert_rule, delete_alert_rule, list_alert_events, list_alert_rules};
use api::v1::auth::{create_user, login, logout, AppState};
use api::v1::ddns::{
    check_ddns_now, create_ddns_config, delete_ddns_config, list_ddns_configs, list_ddns_history,
    reload_ddns_providers,
};
use api::v1::mcp::{execute_mcp_tool, get_mcp_info, handle_mcp_jsonrpc, list_mcp_tools};
use api::v1::nat::{
    create_nat_mapping, delete_nat_mapping, get_nat_mapping, list_all_nat_mappings,
    list_nat_mappings, update_nat_mapping,
};
use api::v1::pat::{create_pat, list_pats, revoke_pat};
use api::v1::server_ops::{
    apply_config, delete_file, download_url, force_update, get_config, list_files, read_file,
    upload_url, write_file,
};
use api::v1::service_history::{get_service_history, get_service_uptime};
use api::v1::terminal::{create_terminal_session, ws_terminal};
// M3: server list / detail / metrics routes are registered inline below
use api::v1::services::{
    create_service, delete_service, get_service, list_services, test_probe, update_service,
};
use api::v1::tasks::{
    create_task, delete_task, get_task, get_task_runs, list_tasks, run_task, update_task,
};
use api::v1::transfers::{temp_download, temp_upload, upload_body_limit};
use auth::middleware::session_middleware;
use xlstatus_shared::UserRole;

const GRPC_MESSAGE_LIMIT: usize = 256 * 1024 * 1024;

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
    tracing::info!("Configuration loaded");

    // Connect to database
    let db = db::DatabaseBackend::connect(&config.database.url, config.database.create_if_missing)
        .await?;
    tracing::info!("Connected to database: {}", config.database.url);

    // Run migrations
    db.run_migrations().await?;
    tracing::info!("Database migrations applied");

    seed_admin_user(&db).await?;

    // M4: start service monitor + alert engine in the background.
    let monitor = Arc::new(ServiceMonitor::new(db.clone()));
    let monitor_clone = monitor.clone();
    tokio::spawn(async move {
        monitor_clone.start().await;
    });
    let alert_engine = Arc::new(AlertEngine::new(db.clone()));
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

    // Build the live agent session registry first so the HTTP
    // AppState (M5 task dispatch) can reach it.
    let session_registry = grpc::SessionRegistry::new();
    let io_registry = grpc::IoRegistry::new();
    let task_response_registry = current_task_response_registry();

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
        task_response_registry,
    ));
    let task_scheduler_clone = task_scheduler.clone();
    tokio::spawn(async move {
        task_scheduler_clone.start().await;
    });

    // Create app state. The session_registry is built up here so
    // the AppState handlers (M5 task dispatch) can reach the live
    // agent sessions registered by the gRPC service.
    let metrics = xlstatus_tsdb::MetricStore::in_memory();
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
            let protected = Router::new()
                .route("/api/v1/auth/logout", post(logout))
                .route("/api/v1/users", post(create_user))
                .route("/api/v1/tokens", post(create_pat))
                .route("/api/v1/tokens", get(list_pats))
                .route("/api/v1/tokens/:id", axum::routing::delete(revoke_pat))
                .route("/api/v1/enrollment-tokens", post(create_enrollment_token))
                .route(
                    "/api/v1/agents/:id/revoke",
                    post(api::v1::agent::revoke_agent),
                )
                .route("/api/v1/mcp/tools", get(list_mcp_tools))
                .route("/api/v1/mcp/execute", post(execute_mcp_tool))
                .route("/api/v1/mcp/info", get(get_mcp_info))
                .route("/mcp", post(handle_mcp_jsonrpc))
                .route("/api/v1/services", get(list_services))
                .route("/api/v1/services", post(create_service))
                .route("/api/v1/services/test-probe", post(test_probe))
                .route("/api/v1/services/:id", get(get_service))
                .route("/api/v1/services/:id", post(update_service))
                .route("/api/v1/services/:id", delete(delete_service))
                .route("/api/v1/services/:id/history", get(get_service_history))
                .route("/api/v1/services/:id/uptime", get(get_service_uptime))
                .route("/api/v1/alert-rules", post(create_alert_rule))
                .route("/api/v1/alert-rules", get(list_alert_rules))
                .route(
                    "/api/v1/alert-rules/:id",
                    axum::routing::delete(delete_alert_rule),
                )
                .route("/api/v1/alert-events", get(list_alert_events))
                // M6: DDNS config + history endpoints
                .route("/api/v1/ddns/configs", post(create_ddns_config))
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
                .route("/api/v1/servers/:id", get(api::v1::servers::get_server))
                .route(
                    "/api/v1/servers/:id/metrics",
                    get(api::v1::servers::get_server_metrics),
                )
                .route("/api/v1/servers/:id/files", get(list_files))
                .route("/api/v1/servers/:id/files/read", get(read_file))
                .route("/api/v1/servers/:id/files/write", post(write_file))
                .route("/api/v1/servers/:id/files/delete", post(delete_file))
                .route("/api/v1/servers/:id/files/download-url", get(download_url))
                .route("/api/v1/servers/:id/files/upload-url", get(upload_url))
                .route("/api/v1/servers/:id/config", get(get_config))
                .route("/api/v1/servers/:id/config", post(apply_config))
                .route("/api/v1/servers/:id/force-update", post(force_update))
                .route("/ws/servers", get(api::v1::servers::ws_servers))
                // Tasks
                .route("/api/v1/tasks", post(create_task))
                .route("/api/v1/tasks", get(list_tasks))
                .route("/api/v1/tasks/:id", get(get_task))
                .route("/api/v1/tasks/:id", post(update_task))
                .route("/api/v1/tasks/:id", delete(delete_task))
                .route("/api/v1/tasks/:id/run", post(run_task))
                .route("/api/v1/tasks/:id/runs", get(get_task_runs))
                .route("/api/v1/terminal/sessions", post(create_terminal_session))
                .route("/ws/terminal/:session_id", get(ws_terminal))
                // NAT
                .route("/api/v1/nat/mappings", post(create_nat_mapping))
                .route("/api/v1/nat/mappings/all", get(list_all_nat_mappings))
                .route(
                    "/api/v1/nat/mappings/agent/:agent_id",
                    get(list_nat_mappings),
                )
                .route("/api/v1/nat/mappings/:id", get(get_nat_mapping))
                .route("/api/v1/nat/mappings/:id", post(update_nat_mapping))
                .route("/api/v1/nat/mappings/:id", delete(delete_nat_mapping))
                .route_layer(middleware::from_fn_with_state(
                    state.clone(),
                    session_middleware,
                ));

            let app = Router::new()
                .route("/healthz", get(healthz))
                .route("/api/v1/auth/login", post(login))
                .route("/api/v1/public/status", get(api::v1::public::public_status))
                .route("/api/v1/agents/enroll", post(enroll))
                .route("/api/v1/transfers/temp/download", get(temp_download))
                .route(
                    "/api/v1/transfers/temp/upload",
                    axum::routing::put(temp_upload).layer(upload_body_limit()),
                )
                .route(
                    "/api/v1/agents/jwt/challenge",
                    post(get_agent_jwt_challenge),
                )
                .route("/api/v1/agents/jwt", post(get_agent_jwt))
                .merge(protected)
                .with_state(state);

            let addr: SocketAddr = bind.parse()?;
            tracing::info!("HTTP server listening on {}", addr);

            let listener = tokio::net::TcpListener::bind(addr).await?;
            axum::serve(listener, app)
                .await
                .map_err(|e| anyhow::anyhow!("HTTP server error: {}", e))
        }

        run_http_server(http_bind, state).await
    });

    // Hand the (already built) session registry to the agent
    // revoke handler so /api/v1/agents/:id/revoke can find the
    // matching live session.
    api::v1::agent::set_revoke_registry(Arc::new(session_registry.clone()));

    // Start gRPC server
    let grpc_bind = config.server.grpc_bind.clone();
    let grpc_db = db.clone();
    let grpc_session_registry = session_registry.clone();
    let grpc_metrics = metrics.clone();
    let grpc_realtime = realtime.clone();
    let grpc_io_registry = io_registry.clone();
    let grpc_handle = tokio::spawn(async move {
        async fn run_grpc_server(
            bind: String,
            db: DatabaseBackend,
            session_registry: grpc::SessionRegistry,
            metrics: xlstatus_tsdb::MetricStore,
            realtime: crate::realtime::BroadcastHub,
            io_registry: grpc::IoRegistry,
        ) -> anyhow::Result<()> {
            let addr: SocketAddr = bind.parse()?;
            tracing::info!("gRPC server listening on {}", addr);

            let config = config::Config::load()?;
            let agent_service = grpc::AgentServiceImpl::new(
                db,
                session_registry,
                config.security.session_secret,
                metrics,
                realtime,
                io_registry,
            );
            let reflection_service = tonic_reflection::server::Builder::configure()
                .register_encoded_file_descriptor_set(
                    xlstatus_proto_gen::xlstatus::v1::FILE_DESCRIPTOR_SET,
                )
                .build_v1()
                .map_err(|e| anyhow::anyhow!("Failed to build reflection service: {}", e))?;

            let agent_service = AgentServiceServer::new(agent_service)
                .max_decoding_message_size(GRPC_MESSAGE_LIMIT)
                .max_encoding_message_size(GRPC_MESSAGE_LIMIT);

            TonicServer::builder()
                .add_service(agent_service)
                .add_service(reflection_service)
                .serve(addr)
                .await
                .map_err(|e| anyhow::anyhow!("gRPC server error: {}", e))
        }

        run_grpc_server(
            grpc_bind,
            grpc_db,
            grpc_session_registry,
            grpc_metrics,
            grpc_realtime,
            grpc_io_registry,
        )
        .await
    });

    // Wait for both servers
    tokio::select! {
        res = http_handle => {
            if let Err(e) = res {
                tracing::error!("HTTP server error: {}", e);
            }
        }
        res = grpc_handle => {
            if let Err(e) = res {
                tracing::error!("gRPC server error: {}", e);
            }
        }
    }

    Ok(())
}

async fn healthz() -> &'static str {
    "OK"
}

async fn seed_admin_user(db: &DatabaseBackend) -> anyhow::Result<()> {
    let username =
        std::env::var("XLSTATUS_SEED_ADMIN_USERNAME").or_else(|_| std::env::var("ADMIN_USERNAME"));
    let password =
        std::env::var("XLSTATUS_SEED_ADMIN_PASSWORD").or_else(|_| std::env::var("ADMIN_PASSWORD"));

    let (Ok(username), Ok(password)) = (username, password) else {
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

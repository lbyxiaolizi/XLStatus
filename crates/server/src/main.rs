use axum::{routing::get, routing::post, Router};
use std::net::SocketAddr;
use std::sync::Arc;
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
mod services;
mod tasks;

use crate::db::{DatabaseBackend, UserRepository};
use api::v1::agent::{create_enrollment_token, enroll};
use api::v1::agent_jwt::get_agent_jwt;
use api::v1::auth::{create_user, login, logout, AppState};
use api::v1::pat::{create_pat, list_pats, revoke_pat};
use api::v1::services::test_probe;

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
    let db = db::DatabaseBackend::connect(&config.database.url).await?;
    tracing::info!("Connected to database: {}", config.database.url);

    // Run migrations
    db.run_migrations().await?;
    tracing::info!("Database migrations applied");

    // Create app state
    let state = AppState {
        db: db.clone(),
        config: Arc::new(config.clone()),
    };

    // Start HTTP server
    let http_bind = config.server.http_bind.clone();
    let http_handle = tokio::spawn(async move {
        async fn run_http_server(bind: String, state: AppState) -> anyhow::Result<()> {
            let app = Router::new()
                .route("/healthz", get(healthz))
                .route("/api/v1/auth/login", post(login))
                .route("/api/v1/auth/logout", post(logout))
                .route("/api/v1/users", post(create_user))
                .route("/api/v1/tokens", post(create_pat))
                .route("/api/v1/tokens", get(list_pats))
                .route("/api/v1/tokens/:id", axum::routing::delete(revoke_pat))
                .route("/api/v1/enrollment-tokens", post(create_enrollment_token))
                .route("/api/v1/agents/enroll", post(enroll))
                .route("/api/v1/agents/jwt", post(get_agent_jwt))
                .route("/api/v1/services/test-probe", post(test_probe))
                .with_state(state);

            let addr: SocketAddr = bind.parse()?;
            tracing::info!("HTTP server listening on {}", addr);

            let listener = tokio::net::TcpListener::bind(addr).await?;
            axum::serve(listener, app).await.map_err(|e| anyhow::anyhow!("HTTP server error: {}", e))
        }

        run_http_server(http_bind, state).await
    });

    // Create session registry
    let session_registry = grpc::SessionRegistry::new();

    // Start gRPC server
    let grpc_bind = config.server.grpc_bind.clone();
    let grpc_db = db.clone();
    let grpc_session_registry = session_registry.clone();
    let grpc_handle = tokio::spawn(async move {
        async fn run_grpc_server(
            bind: String,
            db: DatabaseBackend,
            session_registry: grpc::SessionRegistry,
        ) -> anyhow::Result<()> {
            let addr: SocketAddr = bind.parse()?;
            tracing::info!("gRPC server listening on {}", addr);

            let agent_service = grpc::AgentServiceImpl::new(db, session_registry);
            let reflection_service = tonic_reflection::server::Builder::configure()
                .register_encoded_file_descriptor_set(xlstatus_proto_gen::xlstatus::v1::FILE_DESCRIPTOR_SET)
                .build_v1()
                .map_err(|e| anyhow::anyhow!("Failed to build reflection service: {}", e))?;

            TonicServer::builder()
                .add_service(AgentServiceServer::new(agent_service))
                .add_service(reflection_service)
                .serve(addr)
                .await
                .map_err(|e| anyhow::anyhow!("gRPC server error: {}", e))
        }

        run_grpc_server(grpc_bind, grpc_db, grpc_session_registry).await
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

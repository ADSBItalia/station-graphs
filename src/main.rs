mod telemetry;

use axum::{
    http::{header, HeaderValue},
    routing::get,
    Router,
};
use std::net::SocketAddr;
use std::path::Path;
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    services::ServeDir,
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

async fn health_check() -> &'static str {
    "ADSBItalia Station Graphs OK"
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cfg = telemetry::CONFIG.read().unwrap().clone();
    tracing::info!("Starting ADSBItalia Station Graphs Daemon");
    tracing::info!("Station Name: {}", cfg.name);
    tracing::info!("Station Location: {}, {}", cfg.lat, cfg.lon);
    tracing::info!("Readsb JSON Path: {}", cfg.aircraft_json);

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Locate web directory
    let web_dir = if Path::new("./web").exists() {
        "./web".to_string()
    } else if Path::new("/opt/adsbitalia-station-graphs/web").exists() {
        "/opt/adsbitalia-station-graphs/web".to_string()
    } else if Path::new("/var/www/graphs").exists() {
        "/var/www/graphs".to_string()
    } else {
        "./web".to_string()
    };
    tracing::info!("Serving web assets from: {}", web_dir);

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/api/config", get(telemetry::handle_config))
        .route("/api/live", get(telemetry::handle_live))
        .route("/api/history", get(telemetry::handle_history))
        .route("/api/coverage", get(telemetry::handle_coverage))
        .route("/graphs/api/live", get(telemetry::handle_live))
        .route("/graphs/api/history", get(telemetry::handle_history))
        .route("/graphs/api/coverage", get(telemetry::handle_coverage))
        .fallback_service(ServeDir::new(web_dir))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("x-backend"),
            HeaderValue::from_static("adsbitalia-station-graphs"),
        ))
        .layer(cors)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http());

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8085);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap_or_else(|e| {
        panic!("Failed to bind to {}: {}", addr, e);
    });

    axum::serve(listener, app).await.unwrap();
}

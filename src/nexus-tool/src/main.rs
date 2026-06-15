use anyhow::Result;
use axum::{routing::get, routing::post, Router};
use nautilus_enclave::EnclaveKeyPair;
use nexus_tool::common::{
    get_attestation, get_logs, health_check, invoke,
    walrus_upload_json, walrus_read_json, walrus_verify_blob,
};
use nexus_tool::{AppState, LogBuffer};
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

struct LogBufferLayer {
    buffer: Arc<LogBuffer>,
}

impl<S> tracing_subscriber::Layer<S> for LogBufferLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = LogVisitor::default();
        event.record(&mut visitor);
        let level = event.metadata().level();
        let target = event.metadata().target();
        let line = format!(
            "{} {:>5} {} {}",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
            level,
            target,
            visitor.message
        );
        self.buffer.push(line);
    }
}

#[derive(Default)]
struct LogVisitor {
    message: String,
}

impl tracing::field::Visit for LogVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        } else if !self.message.is_empty() {
            self.message
                .push_str(&format!(" {}={:?}", field.name(), value));
        } else {
            self.message = format!("{}={:?}", field.name(), value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else if !self.message.is_empty() {
            self.message
                .push_str(&format!(" {}={}", field.name(), value));
        } else {
            self.message = format!("{}={}", field.name(), value);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let log_buffer = Arc::new(LogBuffer::new(1000));

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(LogBufferLayer {
            buffer: log_buffer.clone(),
        })
        .init();

    // Generate Ed25519 keypair — NSM entropy in enclave, OsRng locally
    let eph_kp = EnclaveKeyPair::generate();

    let state = Arc::new(AppState {
        eph_kp,
        logs: log_buffer,
    });

    info!("Starting nexus-tee-tool...");

    run_api_server(state).await
}

async fn run_api_server(state: Arc<AppState>) -> Result<()> {
    use tower_http::cors::{Any, CorsLayer};

    let cors = CorsLayer::new()
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_origin(Any);

    let app = Router::new()
        .route("/", get(ping))
        .route("/health", get(health_check))
        .route("/get_attestation", get(get_attestation))
        .route("/tee/demo/invoke", post(invoke))
        .route("/walrus/upload-json", post(walrus_upload_json))
        .route("/walrus/read-json", post(walrus_read_json))
        .route("/walrus/verify-blob", post(walrus_verify_blob))
        .route("/logs", get(get_logs))
        .with_state(state)
        .layer(cors);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:4000").await?;
    info!("nexus-tee-tool listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app)
        .await
        .map_err(|e| anyhow::anyhow!("Server error: {}", e))
}

async fn ping() -> &'static str {
    "Nexus TEE Tool Ready!"
}

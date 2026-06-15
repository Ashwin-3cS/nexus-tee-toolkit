use crate::AppState;
use crate::EnclaveError;
use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tracing::info;
use nautilus_enclave;

// ── Helpers ──────────────────────────────────────────────────────────

fn sha256_hex(data: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(data)))
}

// ── GET /get_attestation ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct GetAttestationResponse {
    pub attestation: String,
    pub public_key: String,
}

pub async fn get_attestation(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GetAttestationResponse>, EnclaveError> {
    info!("get_attestation called");

    let pk_bytes = state.eph_kp.public_key_bytes();
    let doc = nautilus_enclave::get_attestation(&pk_bytes, &[])
        .map_err(|e| EnclaveError::GenericError(format!("attestation failed: {}", e)))?;

    Ok(Json(GetAttestationResponse {
        attestation: doc.raw_cbor_hex,
        public_key: hex::encode(&pk_bytes),
    }))
}

// ── GET /health ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthCheckResponse {
    pub public_key: String,
    pub status: String,
    pub tool_fqn: String,
}

pub async fn health_check(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HealthCheckResponse>, EnclaveError> {
    let pk = state.eph_kp.public_key_bytes();
    Ok(Json(HealthCheckResponse {
        public_key: hex::encode(pk),
        status: "ok".to_string(),
        tool_fqn: "xyz.ashwin.tee.demo@1".to_string(),
    }))
}

// ── POST /tee/demo/invoke ─────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct InvokeRequest {
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AttestationDoc {
    /// Raw COSE_Sign1 attestation document from the Nitro Security Module, hex-encoded.
    pub raw_cbor_hex: String,
    /// SHA-384 PCR0 — identity fingerprint of the running enclave binary.
    pub pcr0: Option<String>,
    /// Enclave ephemeral public key (Ed25519), hex-encoded.
    pub public_key: String,
    /// SHA-256 of the serialized input — binds this attestation to the specific invocation.
    pub input_hash: String,
    /// SHA-256 of the serialized output — proves output wasn't tampered post-enclave.
    pub output_hash: String,
    /// Tool FQN this attestation was produced for.
    pub tool_fqn: String,
    /// Timestamp from inside the enclave.
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InvokeResponse {
    pub result: String,
    pub attestation: AttestationDoc,
}

pub async fn invoke(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InvokeRequest>,
) -> Result<Json<serde_json::Value>, EnclaveError> {
    info!("invoke called with message: {}", req.message);

    let input_hash = sha256_hex(req.message.as_bytes());
    let result = format!("Processed inside enclave: {}", req.message);
    let output_hash = sha256_hex(result.as_bytes());
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    // user_data embeds input/output hashes so they're cryptographically bound
    // to this specific invocation — prevents attestation replay attacks.
    let user_data = serde_json::to_vec(&serde_json::json!({
        "input_hash": &input_hash,
        "output_hash": &output_hash,
        "tool_fqn": "xyz.ashwin.tee.demo@1",
        "timestamp": &timestamp,
    }))
    .unwrap();

    let pk_bytes = state.eph_kp.public_key_bytes();
    let doc = nautilus_enclave::get_attestation(&pk_bytes, &user_data)
        .map_err(|e| EnclaveError::GenericError(format!("attestation failed: {}", e)))?;

    let attestation = AttestationDoc {
        raw_cbor_hex: doc.raw_cbor_hex,
        pcr0: Some(doc.pcr0),
        public_key: hex::encode(&pk_bytes),
        input_hash,
        output_hash,
        tool_fqn: "xyz.ashwin.tee.demo@1".to_string(),
        timestamp,
    };

    // Nexus-compatible envelope: { "ok": { ... } }
    Ok(Json(serde_json::json!({
        "ok": {
            "result": result,
            "attestation": attestation,
        }
    })))
}

// ── GET /logs ────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LogsQueryParams {
    pub lines: Option<usize>,
}

#[derive(Serialize)]
pub struct LogsResponse {
    pub lines: Vec<String>,
    pub count: usize,
}

pub async fn get_logs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<LogsQueryParams>,
) -> Result<Json<LogsResponse>, EnclaveError> {
    let n = params.lines.unwrap_or(100).min(1000);
    let lines = state.logs.recent(n);
    Ok(Json(LogsResponse {
        count: lines.len(),
        lines,
    }))
}

// ── Walrus handlers ───────────────────────────────────────────────────
//
// Each handler runs the Walrus operation, then wraps the result with a
// TEE attestation that cryptographically binds the specific input and
// output to the running enclave binary (PCR0).

async fn walrus_attest<T: serde::Serialize>(
    state: &Arc<AppState>,
    input_hash: &str,
    output: &T,
    op: &str,
) -> serde_json::Value {
    let output_json = serde_json::to_value(output).unwrap_or(serde_json::Value::Null);
    let output_hash = sha256_hex(output_json.to_string().as_bytes());
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let user_data = serde_json::to_vec(&serde_json::json!({
        "op": op,
        "input_hash": input_hash,
        "output_hash": &output_hash,
        "tool_fqn": "xyz.ashwin.tee.walrus@1",
        "timestamp": &timestamp,
    }))
    .unwrap();

    let pk_bytes = state.eph_kp.public_key_bytes();
    let attest = nautilus_enclave::get_attestation(&pk_bytes, &user_data).ok();

    serde_json::json!({
        "result": output_json,
        "attestation": {
            "pcr0": attest.as_ref().map(|a| a.pcr0.clone()),
            "raw_cbor_hex": attest.as_ref().map(|a| a.raw_cbor_hex.clone()).unwrap_or_default(),
            "public_key": hex::encode(&pk_bytes),
            "input_hash": input_hash,
            "output_hash": output_hash,
            "tool_fqn": "xyz.ashwin.tee.walrus@1",
            "timestamp": timestamp,
        }
    })
}

// POST /walrus/upload-json
pub async fn walrus_upload_json(
    State(state): State<Arc<AppState>>,
    Json(req): Json<crate::walrus::upload_json::Input>,
) -> Result<Json<serde_json::Value>, EnclaveError> {
    info!("walrus/upload-json called");
    let input_hash = sha256_hex(req.json.as_bytes());
    let output = crate::walrus::upload_json::run(req).await;
    Ok(Json(walrus_attest(&state, &input_hash, &output, "upload-json").await))
}

// POST /walrus/read-json
pub async fn walrus_read_json(
    State(state): State<Arc<AppState>>,
    Json(req): Json<crate::walrus::read_json::Input>,
) -> Result<Json<serde_json::Value>, EnclaveError> {
    info!("walrus/read-json called for blob: {}", req.blob_id);
    let input_hash = sha256_hex(req.blob_id.as_bytes());
    let output = crate::walrus::read_json::run(req).await;
    Ok(Json(walrus_attest(&state, &input_hash, &output, "read-json").await))
}

// POST /walrus/verify-blob
pub async fn walrus_verify_blob(
    State(state): State<Arc<AppState>>,
    Json(req): Json<crate::walrus::verify_blob::Input>,
) -> Result<Json<serde_json::Value>, EnclaveError> {
    info!("walrus/verify-blob called for blob: {}", req.blob_id);
    let input_hash = sha256_hex(req.blob_id.as_bytes());
    let output = crate::walrus::verify_blob::run(req).await;
    Ok(Json(walrus_attest(&state, &input_hash, &output, "verify-blob").await))
}

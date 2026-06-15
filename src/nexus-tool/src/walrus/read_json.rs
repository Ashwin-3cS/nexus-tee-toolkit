use {
    crate::walrus::client::WalrusConfig,
    nexus_sdk::walrus::WalrusError,
    serde::{Deserialize, Serialize},
    serde_json::Value,
    thiserror::Error,
};

#[derive(Error, Debug)]
pub enum ReadJsonError {
    #[error("Failed to read JSON: {0}")]
    ReadError(#[from] WalrusError),
    #[error("Invalid JSON data: {0}")]
    InvalidJson(String),
}

#[derive(Debug, Deserialize)]
pub struct Input {
    pub blob_id: String,
    #[serde(default, deserialize_with = "crate::walrus::utils::validation::deserialize_url_opt")]
    pub aggregator_url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    Ok { json: Value },
    Err { reason: String, kind: ReadErrorKind, #[serde(skip_serializing_if = "Option::is_none")] status_code: Option<u16> },
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReadErrorKind { Network, Validation }

pub async fn run(input: Input) -> Output {
    let client = WalrusConfig::new()
        .with_aggregator_url(input.aggregator_url)
        .build()
        .await;

    match client.read_json::<String>(&input.blob_id).await {
        Ok(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(json) => Output::Ok { json },
            Err(e) => Output::Err {
                reason: ReadJsonError::InvalidJson(e.to_string()).to_string(),
                kind: ReadErrorKind::Validation,
                status_code: None,
            },
        },
        Err(e) => {
            let status_code = match &e {
                WalrusError::ApiError { status_code, .. } => Some(*status_code),
                _ => None,
            };
            Output::Err { reason: ReadJsonError::ReadError(e).to_string(), kind: ReadErrorKind::Network, status_code }
        }
    }
}

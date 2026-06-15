use {
    crate::walrus::client::WalrusConfig,
    nexus_sdk::walrus::WalrusError,
    serde::{Deserialize, Serialize},
    thiserror::Error,
};

#[derive(Error, Debug)]
pub enum VerifyBlobError {
    #[error("Failed to verify blob: {0}")]
    VerificationError(#[from] WalrusError),
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
    Verified { blob_id: String },
    Unverified { blob_id: String },
    Err { reason: String, #[serde(skip_serializing_if = "Option::is_none")] status_code: Option<u16> },
}

pub async fn run(input: Input) -> Output {
    let blob_id = input.blob_id.clone();
    let client = WalrusConfig::new()
        .with_aggregator_url(input.aggregator_url)
        .build()
        .await;

    match client.verify_blob(&input.blob_id).await {
        Ok(true)  => Output::Verified { blob_id },
        Ok(false) => Output::Unverified { blob_id },
        Err(e) => {
            let status_code = match &e {
                WalrusError::ApiError { status_code, .. } => Some(*status_code),
                _ => None,
            };
            Output::Err { reason: e.to_string(), status_code }
        }
    }
}

use {
    crate::walrus::client::{with_publisher_retry, WalrusConfig},
    nexus_sdk::walrus::{StorageInfo, WalrusError},
    serde::{Deserialize, Serialize},
    thiserror::Error,
};

#[derive(Error, Debug)]
pub enum UploadJsonError {
    #[error("Failed to upload JSON: {0}")]
    UploadError(#[from] WalrusError),
    #[error("Invalid JSON data: {0}")]
    InvalidJson(String),
}

#[derive(Debug, Deserialize)]
pub struct Input {
    pub json: String,
    #[serde(default, deserialize_with = "crate::walrus::utils::validation::deserialize_url_opt")]
    pub publisher_url: Option<String>,
    #[serde(default, deserialize_with = "crate::walrus::utils::validation::deserialize_url_opt")]
    pub aggregator_url: Option<String>,
    #[serde(default = "default_epochs")]
    pub epochs: u8,
    #[serde(default)]
    pub send_to_address: Option<String>,
}

fn default_epochs() -> u8 { 1 }

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    AlreadyCertified { blob_id: String, end_epoch: u64, tx_digest: String },
    NewlyCreated { blob_id: String, end_epoch: u64, sui_object_id: String },
    Err { reason: String, kind: UploadErrorKind, #[serde(skip_serializing_if = "Option::is_none")] status_code: Option<u16> },
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UploadErrorKind { Network, Validation }

pub async fn run(input: Input) -> Output {
    match upload(input).await {
        Ok(info) => to_output(info),
        Err(e) => {
            let (kind, status_code) = match &e {
                UploadJsonError::InvalidJson(_) => (UploadErrorKind::Validation, None),
                UploadJsonError::UploadError(WalrusError::ApiError { status_code, .. }) => {
                    (UploadErrorKind::Network, Some(*status_code))
                }
                UploadJsonError::UploadError(_) => (UploadErrorKind::Network, None),
            };
            Output::Err { reason: e.to_string(), kind, status_code }
        }
    }
}

async fn upload(input: Input) -> Result<StorageInfo, UploadJsonError> {
    serde_json::from_str::<serde_json::Value>(&input.json)
        .map_err(|e| UploadJsonError::InvalidJson(e.to_string()))?;

    let client = WalrusConfig::new()
        .with_publisher_url(input.publisher_url)
        .with_aggregator_url(input.aggregator_url)
        .build()
        .await;

    Ok(with_publisher_retry(|| {
        client.upload_json(&input.json, input.epochs, input.send_to_address.clone())
    }).await?)
}

fn to_output(info: StorageInfo) -> Output {
    if let Some(ac) = info.already_certified {
        Output::AlreadyCertified { blob_id: ac.blob_id, end_epoch: ac.end_epoch, tx_digest: ac.event.tx_digest }
    } else {
        let nc = info.newly_created.unwrap();
        Output::NewlyCreated {
            blob_id: nc.blob_object.blob_id,
            end_epoch: nc.blob_object.storage.end_epoch,
            sui_object_id: nc.blob_object.id,
        }
    }
}

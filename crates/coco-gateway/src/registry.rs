//! Lightweight token registry with blake3 hashing and file persistence.

use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use subtle::ConstantTimeEq;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TokenStatus {
    Active,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRecord {
    pub id: Uuid,
    pub name: String,
    pub scope: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub status: TokenStatus,
    pub token_hash: String,
}

impl TokenRecord {
    pub fn allows_route(&self, route: &str) -> bool {
        self.is_unrestricted() || self.scope.iter().any(|scoped_route| scoped_route == route)
    }

    pub fn is_unrestricted(&self) -> bool {
        self.scope.is_empty()
    }
}

pub struct TokenRegistry {
    tokens: RwLock<Vec<TokenRecord>>,
    file_path: PathBuf,
}

impl TokenRegistry {
    pub async fn load_or_create(path: PathBuf) -> std::io::Result<Self> {
        let tokens = if path.exists() {
            let data = tokio::fs::read_to_string(&path).await?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(Self {
            tokens: RwLock::new(tokens),
            file_path: path,
        })
    }

    pub async fn create_token(&self, name: String, scope: Vec<String>) -> (TokenRecord, String) {
        let mut raw = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut raw);
        let token_value = format!("ccgw_{}", hex::encode(raw));
        let hash = hex::encode(blake3::hash(token_value.as_bytes()).as_bytes());

        let record = TokenRecord {
            id: Uuid::new_v4(),
            name,
            scope,
            created_at: Utc::now(),
            status: TokenStatus::Active,
            token_hash: hash,
        };

        self.tokens.write().await.push(record.clone());
        self.persist().await;

        (record, token_value)
    }

    pub async fn validate(&self, token: &str) -> Option<TokenRecord> {
        let hash = blake3::hash(token.as_bytes());
        let hash_bytes = hash.as_bytes();
        let tokens = self.tokens.read().await;
        tokens
            .iter()
            .find(|r| {
                r.status == TokenStatus::Active
                    && hex::decode(&r.token_hash)
                        .map(|h| h.ct_eq(hash_bytes).into())
                        .unwrap_or(false)
            })
            .cloned()
    }

    pub async fn list_tokens(&self) -> Vec<TokenRecord> {
        self.tokens.read().await.clone()
    }

    pub async fn revoke_token(&self, id: Uuid) -> bool {
        let mut tokens = self.tokens.write().await;
        if let Some(record) = tokens.iter_mut().find(|r| r.id == id) {
            record.status = TokenStatus::Revoked;
            drop(tokens);
            self.persist().await;
            true
        } else {
            false
        }
    }

    async fn persist(&self) {
        if let Some(parent) = self.file_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let tokens = self.tokens.read().await;
        let data = serde_json::to_string_pretty(&*tokens).unwrap_or_default();
        let tmp_path = self.file_path.with_extension("tmp");
        if tokio::fs::write(&tmp_path, &data).await.is_ok() {
            let _ = tokio::fs::rename(&tmp_path, &self.file_path).await;
        }
    }
}

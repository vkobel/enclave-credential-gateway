//! Lightweight token registry with blake3 hashing and file persistence.

use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;
use subtle::ConstantTimeEq;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug)]
pub enum TokenCreateError {
    DuplicateName { name: String },
    Persist { source: io::Error },
}

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
    #[serde(default)]
    pub all_routes: bool,
    pub created_at: DateTime<Utc>,
    pub status: TokenStatus,
    pub token_hash: String,
}

impl TokenRecord {
    pub fn allows_route(&self, route: &str) -> bool {
        self.all_routes || self.scope.iter().any(|scoped_route| scoped_route == route)
    }

    pub fn is_all_routes(&self) -> bool {
        self.all_routes
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
            let mut records: Vec<TokenRecord> = serde_json::from_str(&data).unwrap_or_default();
            for record in &mut records {
                if record.scope.is_empty() && !record.all_routes {
                    record.all_routes = true;
                }
            }
            records
        } else {
            Vec::new()
        };
        Ok(Self {
            tokens: RwLock::new(tokens),
            file_path: path,
        })
    }

    pub async fn create_token(
        &self,
        name: String,
        scope: Vec<String>,
        all_routes: bool,
    ) -> Result<(TokenRecord, String), TokenCreateError> {
        let mut tokens = self.tokens.write().await;
        if tokens.iter().any(|record| record.name == name) {
            return Err(TokenCreateError::DuplicateName { name });
        }

        let mut raw = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut raw);
        let token_value = format!("gate_{}", hex::encode(raw));
        let hash = hex::encode(blake3::hash(token_value.as_bytes()).as_bytes());

        let record = TokenRecord {
            id: Uuid::new_v4(),
            name,
            scope,
            all_routes,
            created_at: Utc::now(),
            status: TokenStatus::Active,
            token_hash: hash,
        };

        tokens.push(record.clone());
        drop(tokens);
        if let Err(source) = self.persist().await {
            let mut tokens = self.tokens.write().await;
            tokens.retain(|token| token.id != record.id);
            return Err(TokenCreateError::Persist { source });
        }

        Ok((record, token_value))
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

    pub async fn revoke_token(&self, id: Uuid) -> io::Result<bool> {
        let mut tokens = self.tokens.write().await;
        if let Some(record) = tokens.iter_mut().find(|r| r.id == id) {
            let previous_status = record.status.clone();
            record.status = TokenStatus::Revoked;
            drop(tokens);
            if let Err(error) = self.persist().await {
                let mut tokens = self.tokens.write().await;
                if let Some(record) = tokens.iter_mut().find(|r| r.id == id) {
                    record.status = previous_status;
                }
                return Err(error);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn persist(&self) -> io::Result<()> {
        if let Some(parent) = self.file_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tokens = self.tokens.read().await;
        let data = serde_json::to_string_pretty(&*tokens).map_err(io::Error::other)?;
        let tmp_path = self.file_path.with_extension("tmp");
        tokio::fs::write(&tmp_path, &data).await?;
        tokio::fs::rename(&tmp_path, &self.file_path).await?;
        Ok(())
    }
}

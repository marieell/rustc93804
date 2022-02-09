use async_trait::async_trait;
use core::fmt::Debug;
use thiserror::Error;

#[derive(Error, Clone, Debug, PartialEq)]
pub enum ClientError {
    #[error("{0}")]
    Generic(String),
}

#[async_trait]
pub trait Client: Debug {
    async fn get(&self, url: &str) -> Result<String, ClientError>;
}

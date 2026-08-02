//! Probe Backend Trait and Implementations

pub mod curl;
pub mod dnsx;
pub mod ffuf;
pub mod httpx;
pub mod lychee;
pub mod native_reqwest;
pub mod nmap;
pub mod openssl;
pub mod ping;
pub mod wget;

use crate::health::models::{BackendObservation, ProbeContext, ProbeTarget};
use async_trait::async_trait;

#[async_trait]
pub trait ProbeBackend: Send + Sync {
    fn backend_id(&self) -> &'static str;

    async fn probe(
        &self,
        target: &ProbeTarget,
        context: &ProbeContext,
    ) -> Result<BackendObservation, String>;
}

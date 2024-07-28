//! mev-share api implementation and helpers.

use jsonrpsee::core::RpcResult;
use jsonrpsee_types::ErrorObjectOwned;
use reth_primitives::keccak256;
use reth_rpc_api::MevApiServer;
use reth_rpc_server_types::result::rpc_error_with_code;
use reth_rpc_types::mev::{
    SendBundleRequest, SendBundleResponse, SimBundleOverrides, SimBundleResponse,
};
use reth_transaction_pool::TransactionPoolBundleExt;
use std::sync::Arc;

/// [`MevApi`] implementation to receive MEV Share bundles.
pub struct MevApi<Pool> {
    /// An interface to interact with the pool
    pool: Arc<Pool>,
}

impl<Pool> MevApi<Pool> {
    /// Create a new [`MevApi`] instance.
    pub fn new(pool: Pool) -> Self {
        Self { pool: Arc::new(pool) }
    }
}

impl<Pool> MevApi<Pool>
where
    Pool: TransactionPoolBundleExt<Bundle = SendBundleRequest> + 'static,
{
    /// Accepts an mev-share bundle and attempts to insert it into the mev share bundle pool.
    pub async fn send_bundle(
        &self,
        bundle: SendBundleRequest,
    ) -> Result<SendBundleResponse, MevError> {
        // TODO: Check if this is the correct way to serialize bundle contents
        let bytes = bincode::serialize(&bundle.bundle_body).unwrap();
        let bundle_hash = keccak256(bytes);

        // Validation is performed by the pool
        self.pool.add_bundle(bundle).map_err(MevError::InsertionError)?;

        Ok(SendBundleResponse { bundle_hash })
    }
}

#[async_trait::async_trait]
impl<Pool> MevApiServer for MevApi<Pool>
where
    Pool: TransactionPoolBundleExt<Bundle = SendBundleRequest> + 'static,
{
    async fn send_bundle(&self, bundle: SendBundleRequest) -> RpcResult<SendBundleResponse> {
        Ok(Self::send_bundle(self, bundle).await?)
    }

    async fn sim_bundle(
        &self,
        _request: SendBundleRequest,
        _sim_overrides: SimBundleOverrides,
    ) -> RpcResult<SimBundleResponse> {
        // Out of scope for work task.
        todo!()
    }
}

impl<Pool> std::fmt::Debug for MevApi<Pool> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MevApi").finish_non_exhaustive()
    }
}

impl<Pool> Clone for MevApi<Pool> {
    fn clone(&self) -> Self {
        Self { pool: Arc::clone(&self.pool) }
    }
}

/// [`Mev`] specific errors.
#[derive(Debug)]
pub enum MevError {
    /// Thrown if the bundle body has no entries
    EmptyBundleBody,
    /// Inserting the bundle into the pool failed
    InsertionError(String),
}

impl From<MevError> for ErrorObjectOwned {
    fn from(err: MevError) -> Self {
        match err {
            MevError::EmptyBundleBody => rpc_error_with_code(
                jsonrpsee_types::error::INVALID_PARAMS_CODE,
                "Bundle body must not be empty",
            ),
            MevError::InsertionError(reason) => rpc_error_with_code(
                jsonrpsee_types::error::INVALID_PARAMS_CODE,
                format!("Failed to insert bundle into pool: {}", reason),
            ),
        }
    }
}

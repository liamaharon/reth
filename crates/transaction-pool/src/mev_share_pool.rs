//! [`TransactionPoolBundleExt`] implementation for the MEV Share bundle type.

use crate::{
    AllPoolTransactions, AllTransactionsEvents, BestTransactions, BestTransactionsAttributes,
    BlobStore, BlobStoreError, BlockInfo, CanonicalStateUpdate, ChangedAccount,
    GetPooledTransactionLimit, NewBlobSidecar, NewTransactionEvent, Pool, PoolConfig, PoolResult,
    PoolSize, PropagatedTransactions, TransactionEvents, TransactionListenerKind,
    TransactionOrdering, TransactionOrigin, TransactionPool, TransactionPoolBundleExt,
    TransactionPoolExt, TransactionValidator, ValidPoolTransaction,
};
use parking_lot::{RwLock, RwLockReadGuard};
use reth_eth_wire_types::HandleMempoolData;
use reth_primitives::{Address, PooledTransactionsElement, TxHash};
use reth_rpc_types::{mev::SendBundleRequest, BlobTransactionSidecar};
use std::{collections::HashSet, sync::Arc};
use tokio::sync::mpsc::Receiver;

/// [`TransactionPoolBundleExt`] implementation for the MEV Share bundle type.
#[derive(Debug)]
pub struct MevSharePool<V, T: TransactionOrdering, S> {
    /// Arc'ed instance of the tx pool internals
    tx_pool: Arc<Pool<V, T, S>>,
    /// Arc'ed instance of the sbundle pool internals
    sbundle_pool: Arc<SBundlePool>,
}

impl<V, T, S> MevSharePool<V, T, S>
where
    V: TransactionValidator,
    T: TransactionOrdering<Transaction = <V as TransactionValidator>::Transaction>,
    S: BlobStore,
{
    /// Create a new transaction pool instance.
    pub fn new(validator: V, ordering: T, blob_store: S, config: PoolConfig) -> Self {
        Self {
            tx_pool: Arc::new(Pool::<V, T, S>::new(validator, ordering, blob_store, config)),
            sbundle_pool: Arc::new(SBundlePool::new(SBundlePoolConfig::default())),
        }
    }
}

impl<V, T, S> TransactionPoolBundleExt for MevSharePool<V, T, S>
where
    V: TransactionValidator,
    T: TransactionOrdering<Transaction = <V as TransactionValidator>::Transaction>,
    S: BlobStore,
{
    type Bundle = SendBundleRequest;

    fn add_bundle(&self, bundle: SendBundleRequest) -> Result<(), String> {
        self.sbundle_pool.add_bundle(bundle)
    }
    fn get_bundles(&self) -> RwLockReadGuard<'_, Vec<SendBundleRequest>> {
        self.sbundle_pool.get_bundles()
    }
}

/// Configuration for [`SBundlePoolInner`].
#[derive(Debug)]
pub struct SBundlePoolConfig {
    /// Maximum number of bundles allowed in the pool.
    pub max_bundles: u32,
}

impl Default for SBundlePoolConfig {
    fn default() -> Self {
        Self { max_bundles: 42 }
    }
}

/// Inner implementation for [`SBundlePool`].
#[derive(Debug, Default)]
struct SBundlePool {
    pending: RwLock<Vec<SendBundleRequest>>,
    config: SBundlePoolConfig,
}

impl SBundlePool {
    /// Initialize a new [`SBundlePool`].
    pub(crate) fn new(config: SBundlePoolConfig) -> Self {
        Self { pending: RwLock::new(Vec::new()), config }
    }

    /// Add a new pending bundle
    /// TODO: Improve error handling
    pub(crate) fn add_bundle(&self, bundle: SendBundleRequest) -> Result<(), String> {
        if self.pending.read().len() as u32 >= self.config.max_bundles {
            return Err("Pending pool is full!".to_owned());
        }

        match self.validate_sbundle(&bundle) {
            Ok(_) => {}
            Err(e) => return Err(format!("Bundle validation failed: {}", e)),
        }

        self.pending.write().push(bundle);

        Ok(())
    }

    /// Get pending bundles
    pub(crate) fn get_bundles(&self) -> RwLockReadGuard<'_, Vec<SendBundleRequest>> {
        self.pending.read()
    }

    /// Validate a bundle is eligible for inclusion
    pub(crate) fn validate_sbundle(&self, _bundle: &SendBundleRequest) -> Result<(), String> {
        // TODO: Implement validation
        Ok(())
    }
}

/// [`TransactionPool`] requires implementors to be [`Clone`].
impl<V, T: TransactionOrdering, S> Clone for MevSharePool<V, T, S> {
    fn clone(&self) -> Self {
        Self { tx_pool: Arc::clone(&self.tx_pool), sbundle_pool: Arc::clone(&self.sbundle_pool) }
    }
}

/// Implements the [`TransactionPool`] interface by delegating to the inner `tx_pool`.
/// TODO: Use a crate like `delegate!` or `ambassador` to automate this.
impl<V, T, S> TransactionPool for MevSharePool<V, T, S>
where
    V: TransactionValidator,
    T: TransactionOrdering<Transaction = <V as TransactionValidator>::Transaction>,
    S: BlobStore,
{
    type Transaction = T::Transaction;

    fn pool_size(&self) -> PoolSize {
        self.tx_pool.pool_size()
    }

    fn block_info(&self) -> BlockInfo {
        self.tx_pool.block_info()
    }

    async fn add_transaction_and_subscribe(
        &self,
        origin: TransactionOrigin,
        transaction: Self::Transaction,
    ) -> PoolResult<TransactionEvents> {
        self.tx_pool.add_transaction_and_subscribe(origin, transaction).await
    }

    async fn add_transaction(
        &self,
        origin: TransactionOrigin,
        transaction: Self::Transaction,
    ) -> PoolResult<TxHash> {
        self.tx_pool.add_transaction(origin, transaction).await
    }

    async fn add_transactions(
        &self,
        origin: TransactionOrigin,
        transactions: Vec<Self::Transaction>,
    ) -> Vec<PoolResult<TxHash>> {
        self.tx_pool.add_transactions(origin, transactions).await
    }

    fn transaction_event_listener(&self, tx_hash: TxHash) -> Option<TransactionEvents> {
        self.tx_pool.transaction_event_listener(tx_hash)
    }

    fn all_transactions_event_listener(&self) -> AllTransactionsEvents<Self::Transaction> {
        self.tx_pool.all_transactions_event_listener()
    }

    fn pending_transactions_listener_for(&self, kind: TransactionListenerKind) -> Receiver<TxHash> {
        self.tx_pool.pending_transactions_listener_for(kind)
    }

    fn blob_transaction_sidecars_listener(&self) -> Receiver<NewBlobSidecar> {
        self.tx_pool.blob_transaction_sidecars_listener()
    }

    fn new_transactions_listener_for(
        &self,
        kind: TransactionListenerKind,
    ) -> Receiver<NewTransactionEvent<Self::Transaction>> {
        self.tx_pool.new_transactions_listener_for(kind)
    }

    fn pooled_transaction_hashes(&self) -> Vec<TxHash> {
        self.tx_pool.pooled_transaction_hashes()
    }

    fn pooled_transaction_hashes_max(&self, max: usize) -> Vec<TxHash> {
        self.tx_pool.pooled_transaction_hashes_max(max)
    }

    fn pooled_transactions(&self) -> Vec<Arc<ValidPoolTransaction<Self::Transaction>>> {
        self.tx_pool.pooled_transactions()
    }

    fn pooled_transactions_max(
        &self,
        max: usize,
    ) -> Vec<Arc<ValidPoolTransaction<Self::Transaction>>> {
        self.tx_pool.pooled_transactions_max(max)
    }

    fn get_pooled_transaction_elements(
        &self,
        tx_hashes: Vec<TxHash>,
        limit: GetPooledTransactionLimit,
    ) -> Vec<PooledTransactionsElement> {
        self.tx_pool.get_pooled_transaction_elements(tx_hashes, limit)
    }

    fn get_pooled_transaction_element(&self, tx_hash: TxHash) -> Option<PooledTransactionsElement> {
        self.tx_pool.get_pooled_transaction_element(tx_hash)
    }

    fn best_transactions(
        &self,
    ) -> Box<dyn BestTransactions<Item = Arc<ValidPoolTransaction<Self::Transaction>>>> {
        self.tx_pool.best_transactions()
    }

    #[allow(deprecated)]
    fn best_transactions_with_base_fee(
        &self,
        base_fee: u64,
    ) -> Box<dyn BestTransactions<Item = Arc<ValidPoolTransaction<Self::Transaction>>>> {
        self.tx_pool.best_transactions_with_base_fee(base_fee)
    }

    fn best_transactions_with_attributes(
        &self,
        best_transactions_attributes: BestTransactionsAttributes,
    ) -> Box<dyn BestTransactions<Item = Arc<ValidPoolTransaction<Self::Transaction>>>> {
        self.tx_pool.best_transactions_with_attributes(best_transactions_attributes)
    }

    fn pending_transactions(&self) -> Vec<Arc<ValidPoolTransaction<Self::Transaction>>> {
        self.tx_pool.pending_transactions()
    }

    fn queued_transactions(&self) -> Vec<Arc<ValidPoolTransaction<Self::Transaction>>> {
        self.tx_pool.queued_transactions()
    }

    fn all_transactions(&self) -> AllPoolTransactions<Self::Transaction> {
        self.tx_pool.all_transactions()
    }

    fn remove_transactions(
        &self,
        hashes: Vec<TxHash>,
    ) -> Vec<Arc<ValidPoolTransaction<Self::Transaction>>> {
        self.tx_pool.remove_transactions(hashes)
    }

    fn retain_unknown<A>(&self, announcement: &mut A)
    where
        A: HandleMempoolData,
    {
        self.tx_pool.retain_unknown(announcement)
    }

    fn get(&self, tx_hash: &TxHash) -> Option<Arc<ValidPoolTransaction<Self::Transaction>>> {
        self.tx_pool.get(tx_hash)
    }

    fn get_all(&self, txs: Vec<TxHash>) -> Vec<Arc<ValidPoolTransaction<Self::Transaction>>> {
        self.tx_pool.get_all(txs)
    }

    fn on_propagated(&self, txs: PropagatedTransactions) {
        self.tx_pool.on_propagated(txs)
    }

    fn get_transactions_by_sender(
        &self,
        sender: Address,
    ) -> Vec<Arc<ValidPoolTransaction<Self::Transaction>>> {
        self.tx_pool.get_transactions_by_sender(sender)
    }

    fn get_transactions_by_sender_and_nonce(
        &self,
        sender: Address,
        nonce: u64,
    ) -> Option<Arc<ValidPoolTransaction<Self::Transaction>>> {
        self.tx_pool.get_transactions_by_sender_and_nonce(sender, nonce)
    }

    fn get_transactions_by_origin(
        &self,
        origin: TransactionOrigin,
    ) -> Vec<Arc<ValidPoolTransaction<Self::Transaction>>> {
        self.tx_pool.get_transactions_by_origin(origin)
    }

    fn unique_senders(&self) -> HashSet<Address> {
        self.tx_pool.unique_senders()
    }

    fn get_blob(&self, tx_hash: TxHash) -> Result<Option<BlobTransactionSidecar>, BlobStoreError> {
        self.tx_pool.get_blob(tx_hash)
    }

    fn get_all_blobs(
        &self,
        tx_hashes: Vec<TxHash>,
    ) -> Result<Vec<(TxHash, BlobTransactionSidecar)>, BlobStoreError> {
        self.tx_pool.get_all_blobs(tx_hashes)
    }

    fn get_all_blobs_exact(
        &self,
        tx_hashes: Vec<TxHash>,
    ) -> Result<Vec<BlobTransactionSidecar>, BlobStoreError> {
        self.tx_pool.get_all_blobs_exact(tx_hashes)
    }
}

/// TODO: Use something like `delegate!` to automate this.
impl<V, T, S> TransactionPoolExt for MevSharePool<V, T, S>
where
    V: TransactionValidator,
    T: TransactionOrdering<Transaction = <V as TransactionValidator>::Transaction>,
    S: BlobStore,
{
    fn set_block_info(&self, info: BlockInfo) {
        self.tx_pool.set_block_info(info)
    }

    fn on_canonical_state_change(&self, update: CanonicalStateUpdate<'_>) {
        self.tx_pool.on_canonical_state_change(update);
    }

    fn update_accounts(&self, accounts: Vec<ChangedAccount>) {
        self.tx_pool.update_accounts(accounts);
    }

    fn delete_blob(&self, tx: TxHash) {
        self.tx_pool.delete_blob(tx)
    }

    fn delete_blobs(&self, txs: Vec<TxHash>) {
        self.tx_pool.delete_blobs(txs)
    }

    fn cleanup_blobs(&self) {
        self.tx_pool.cleanup_blobs()
    }
}

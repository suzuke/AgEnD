//! Shared operator read receipts; viewing never resolves an attention item.
use alloc::{string::String, vec::Vec};
use core::future::Future;
pub trait AttentionReadStore: Sync {
    type Error: Send;
    fn attention_read_keys(&self) -> impl Future<Output = Result<Vec<String>, Self::Error>> + Send;
    fn mark_attention_read<'a>(
        &'a self,
        key: &'a str,
        now: u64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
}

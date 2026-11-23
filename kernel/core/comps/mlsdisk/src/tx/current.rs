use core::sync::atomic::Ordering::{Acquire, Release};

use super::{Tx, TxData, TxId, TxProvider, TxStatus};
use crate::{os::CurrentThread, prelude::*};

#[derive(Clone)]
pub(crate) struct CurrentTx<'a> {
    provider: &'a TxProvider,
}

impl !Send for CurrentTx<'_> {}
impl !Sync for CurrentTx<'_> {}

impl<'a> CurrentTx<'a> {
    pub(super) fn new(provider: &'a TxProvider) -> Self {
        Self { provider }
    }

    pub(crate) fn context<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let tx_table = self.provider.tx_table.lock();
        let tid = CurrentThread::id();
        if !tx_table.contains_key(&tid) {
            panic!("there should be one Tx exited on the current thread");
        }

        assert!(tx_table.get(&tid).unwrap().status() == TxStatus::Ongoing);
        drop(tx_table);

        f()
    }

    pub(crate) fn commit(&self) -> Result<()> {
        let mut tx_status = self
            .provider
            .tx_table
            .lock()
            .get(&CurrentThread::id())
            .expect("there should be one Tx exited on the current thread")
            .status();
        debug_assert!(tx_status == TxStatus::Ongoing);

        let res = self.provider.call_precommit_handlers();
        if res.is_ok() {
            self.provider.call_commit_handlers();
            tx_status = TxStatus::Committed;
        } else {
            self.provider.call_abort_handlers();
            tx_status = TxStatus::Aborted;
        }

        let mut tx = self
            .provider
            .tx_table
            .lock()
            .remove(&CurrentThread::id())
            .unwrap();
        tx.set_status(tx_status);
        res
    }

    pub(crate) fn abort(&self) {
        let tx_status = self
            .provider
            .tx_table
            .lock()
            .get(&CurrentThread::id())
            .expect("there should be one Tx exited on the current thread")
            .status();
        debug_assert!(tx_status == TxStatus::Ongoing);

        self.provider.call_abort_handlers();
        let mut tx = self
            .provider
            .tx_table
            .lock()
            .remove(&CurrentThread::id())
            .unwrap();
        tx.set_status(TxStatus::Aborted);
    }

    pub(crate) fn id(&self) -> TxId {
        self.get_current_mut_with(|tx| tx.id())
    }

    pub(crate) fn data_with<T: TxData, F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        self.get_current_mut_with(|tx| {
            let data = tx.data::<T>();
            f(data)
        })
    }

    pub(crate) fn data_mut_with<T: TxData, F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        self.get_current_mut_with(|tx| {
            let data = tx.data_mut::<T>();
            f(data)
        })
    }

    fn get_current_mut_with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Tx) -> R,
    {
        let mut tx_table = self.provider.tx_table.lock();
        let Some(tx) = tx_table.get_mut(&CurrentThread::id()) else {
            panic!("there should be one Tx exited on the current thread");
        };

        if tx.is_accessing_data.swap(true, Acquire) {
            panic!("get_current_mut_with must not be called recursively");
        }

        let retval: R = f(tx);

        tx.is_accessing_data.store(false, Release);

        retval
    }
}

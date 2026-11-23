mod current;

use core::{
    any::{Any, TypeId},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

pub(crate) use self::current::CurrentTx;
use crate::{
    os::{CurrentThread, HashMap, Mutex, RwLock, Tid},
    prelude::*,
};

#[expect(clippy::type_complexity)]
pub(crate) struct TxProvider {
    id: u64,
    initializer_map: RwLock<HashMap<TypeId, Box<dyn Any + Send + Sync>>>,
    precommit_handlers: RwLock<Vec<Box<dyn Fn(CurrentTx<'_>) -> Result<()> + Send + Sync>>>,
    commit_handlers: RwLock<Vec<Box<dyn Fn(CurrentTx<'_>) + Send + Sync>>>,
    abort_handlers: RwLock<Vec<Box<dyn Fn(CurrentTx<'_>) + Send + Sync>>>,
    weak_self: Weak<Self>,
    tx_table: Mutex<HashMap<Tid, Tx>>,
}

impl TxProvider {
    pub(crate) fn new() -> Arc<Self> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        Arc::new_cyclic(|weak_self| Self {
            id: NEXT_ID.fetch_add(1, Ordering::Release),
            initializer_map: RwLock::new(HashMap::new()),
            precommit_handlers: RwLock::new(Vec::new()),
            commit_handlers: RwLock::new(Vec::new()),
            abort_handlers: RwLock::new(Vec::new()),
            weak_self: weak_self.clone(),
            tx_table: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) fn new_tx(&self) -> CurrentTx<'_> {
        let mut tx_table = self.tx_table.lock();
        let tid = CurrentThread::id();
        if tx_table.contains_key(&tid) {
            return self.current();
        }

        let tx = Tx::new(self.weak_self.clone());
        let _ = tx_table.insert(tid, tx);
        self.current()
    }

    pub(crate) fn current(&self) -> CurrentTx<'_> {
        CurrentTx::new(self)
    }

    pub(crate) fn register_data_initializer<T>(&self, f: Box<dyn Fn() -> T + Send + Sync>)
    where
        T: TxData,
    {
        let mut initializer_map = self.initializer_map.write();
        initializer_map.insert(TypeId::of::<T>(), Box::new(f));
    }

    fn init_data<T>(&self) -> T
    where
        T: TxData,
    {
        let initializer_map = self.initializer_map.read();
        let init_fn = initializer_map
            .get(&TypeId::of::<T>())
            .unwrap()
            .downcast_ref::<Box<dyn Fn() -> T + Send + Sync>>()
            .unwrap();
        init_fn()
    }

    pub(crate) fn register_precommit_handler<F>(&self, f: F)
    where
        F: Fn(CurrentTx<'_>) -> Result<()> + Send + Sync + 'static,
    {
        let f = Box::new(f);
        let mut precommit_handlers = self.precommit_handlers.write();
        precommit_handlers.push(f);
    }

    fn call_precommit_handlers(&self) -> Result<()> {
        let current = self.current();
        let precommit_handlers = self.precommit_handlers.read();
        for precommit_func in precommit_handlers.iter().rev() {
            precommit_func(current.clone())?;
        }
        Ok(())
    }

    pub(crate) fn register_commit_handler<F>(&self, f: F)
    where
        F: Fn(CurrentTx<'_>) + Send + Sync + 'static,
    {
        let f = Box::new(f);
        let mut commit_handlers = self.commit_handlers.write();
        commit_handlers.push(f);
    }

    fn call_commit_handlers(&self) {
        let current = self.current();
        let commit_handlers = self.commit_handlers.read();
        for commit_func in commit_handlers.iter().rev() {
            commit_func(current.clone())
        }
    }

    pub(crate) fn register_abort_handler<F>(&self, f: F)
    where
        F: Fn(CurrentTx<'_>) + Send + Sync + 'static,
    {
        let f = Box::new(f);
        let mut abort_handlers = self.abort_handlers.write();
        abort_handlers.push(f);
    }

    fn call_abort_handlers(&self) {
        let current = self.current();
        let abort_handlers = self.abort_handlers.read();
        for abort_func in abort_handlers.iter().rev() {
            abort_func(current.clone())
        }
    }
}

struct Tx {
    id: TxId,
    provider: Weak<TxProvider>,
    data_map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    status: TxStatus,
    is_accessing_data: AtomicBool,
}

impl Tx {
    fn new(provider: Weak<TxProvider>) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);

        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Release),
            provider,
            data_map: HashMap::new(),
            status: TxStatus::Ongoing,
            is_accessing_data: AtomicBool::new(false),
        }
    }

    fn id(&self) -> TxId {
        self.id
    }

    fn status(&self) -> TxStatus {
        self.status
    }

    fn set_status(&mut self, status: TxStatus) {
        self.status = status;
    }

    fn provider(&self) -> Arc<TxProvider> {
        self.provider.upgrade().unwrap()
    }

    fn data<T>(&mut self) -> &T
    where
        T: TxData,
    {
        self.data_mut::<T>()
    }

    fn data_mut<T>(&mut self) -> &mut T
    where
        T: TxData,
    {
        let exists = self.data_map.contains_key(&TypeId::of::<T>());
        if !exists {
            let provider = self.provider();
            let data: T = provider.init_data::<T>();
            self.data_map.insert(TypeId::of::<T>(), Box::new(data));
        }

        self.data_map
            .get_mut(&TypeId::of::<T>())
            .unwrap()
            .downcast_mut::<T>()
            .unwrap()
    }
}

impl Drop for Tx {
    fn drop(&mut self) {
        assert!(
            self.status() != TxStatus::Ongoing,
            "transactions must be committed or aborted explicitly"
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TxStatus {
    Ongoing,
    Committed,
    Aborted,
}

pub(crate) type TxId = u64;

pub(crate) trait TxData: Any + Send + Sync {}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeSet;

    use super::*;

    pub struct Db<T> {
        all_items: Arc<Mutex<BTreeSet<T>>>,
        tx_provider: Arc<TxProvider>,
    }

    struct DbUpdate<T> {
        new_items: BTreeSet<T>,
    }

    impl<T: 'static> TxData for DbUpdate<T> {}

    impl<T> Db<T>
    where
        T: Ord + 'static,
    {
        pub fn new() -> Self {
            let new_self = Self {
                all_items: Arc::new(Mutex::new(BTreeSet::new())),
                tx_provider: TxProvider::new(),
            };

            new_self
                .tx_provider
                .register_data_initializer(Box::new(|| DbUpdate {
                    new_items: BTreeSet::<T>::new(),
                }));
            new_self.tx_provider.register_commit_handler({
                let all_items = new_self.all_items.clone();
                move |mut current: CurrentTx<'_>| {
                    current.data_mut_with(|update: &mut DbUpdate<T>| {
                        let mut all_items = all_items.lock();
                        all_items.append(&mut update.new_items);
                    });
                }
            });

            new_self
        }

        pub fn new_tx(&self) -> CurrentTx<'_> {
            self.tx_provider.new_tx()
        }

        pub fn contains(&self, item: &T) -> bool {
            let is_new_item = {
                let current_tx = self.tx_provider.current();
                current_tx.data_with(|update: &DbUpdate<T>| update.new_items.contains(item))
            };
            if is_new_item {
                return true;
            }

            let all_items = self.all_items.lock();
            all_items.contains(item)
        }

        pub fn insert(&self, item: T) {
            let all_items = self.all_items.lock();
            if all_items.contains(&item) {
                return;
            }

            let mut current_tx = self.tx_provider.current();
            current_tx.data_mut_with(|update: &mut DbUpdate<_>| {
                update.new_items.insert(item);
            });
        }

        pub fn collect(&self) -> Vec<T>
        where
            T: Copy,
        {
            let all_items = self.all_items.lock();
            let current_tx = self.tx_provider.current();
            current_tx.data_with(|update: &DbUpdate<T>| {
                all_items.union(&update.new_items).cloned().collect()
            })
        }

        pub fn len(&self) -> usize {
            let all_items = self.all_items.lock();
            let current_tx = self.tx_provider.current();
            let new_items_len = current_tx.data_with(|update: &DbUpdate<T>| update.new_items.len());
            all_items.len() + new_items_len
        }
    }

    #[test]
    fn commit_takes_effect() {
        let db: Db<u32> = Db::new();
        let items = vec![1, 2, 3];
        new_tx_and_insert_items::<u32, alloc::vec::IntoIter<u32>>(&db, items.clone().into_iter())
            .commit()
            .unwrap();
        assert!(collect_items(&db) == items);
    }

    #[test]
    fn abort_has_no_effect() {
        let db: Db<u32> = Db::new();
        let items = vec![1, 2, 3];
        new_tx_and_insert_items::<u32, alloc::vec::IntoIter<u32>>(&db, items.into_iter()).abort();
        assert!(collect_items(&db).len() == 0);
    }

    fn new_tx_and_insert_items<T, I>(db: &Db<T>, new_items: I) -> Tx
    where
        I: Iterator<Item = T>,
        T: Copy + Ord + 'static,
    {
        let mut tx = db.new_tx();
        tx.context(move || {
            for new_item in new_items {
                db.insert(new_item);
            }
        });
        tx
    }

    fn collect_items<T>(db: &Db<T>) -> Vec<T>
    where
        T: Copy + Ord + 'static,
    {
        let mut tx = db.new_tx();
        let items = tx.context(|| db.collect());
        tx.commit().unwrap();
        items
    }
}

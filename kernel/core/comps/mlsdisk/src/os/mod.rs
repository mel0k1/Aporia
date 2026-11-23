pub(crate) use alloc::{
    boxed::Box,
    collections::BTreeMap,
    string::{String, ToString},
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

use aes_gcm::{
    Aes128Gcm,
    aead::{AeadInPlace, Key, NewAead, Nonce, Tag},
    aes::Aes128,
};
use ctr::cipher::{NewCipher, StreamCipher};
pub(crate) use hashbrown::{HashMap, HashSet};
pub(crate) use ostd::sync::{Mutex, MutexGuard, RwLock, SpinLock};
use ostd::{
    arch::read_random,
    sync::{self, PreemptDisabled, WaitQueue},
    task::{Task, TaskOptions},
};
use ostd_pod::{FromZeros, IntoBytes, Pod};
use serde::{Deserialize, Serialize};

use crate::{
    error::{Errno, Error},
    prelude::Result,
};

pub(crate) type RwLockReadGuard<'a, T> = sync::RwLockReadGuard<'a, T, PreemptDisabled>;
pub(crate) type RwLockWriteGuard<'a, T> = sync::RwLockWriteGuard<'a, T, PreemptDisabled>;
pub(crate) type SpinLockGuard<'a, T> = sync::SpinLockGuard<'a, T, PreemptDisabled>;
pub(crate) type Tid = u32;

pub(crate) struct CurrentThread;

impl CurrentThread {
    pub(crate) fn id() -> Tid {
        let Some(task) = Task::current() else {
            return 0;
        };

        task.data() as *const _ as u32
    }
}

pub(crate) struct Condvar {
    waitqueue: Arc<WaitQueue>,
    counter: SpinLock<Inner>,
}

struct Inner {
    waiter_count: u64,
    notify_count: u64,
}

impl Condvar {
    pub(crate) fn new() -> Self {
        Condvar {
            waitqueue: Arc::new(WaitQueue::new()),
            counter: SpinLock::new(Inner {
                waiter_count: 0,
                notify_count: 0,
            }),
        }
    }

    pub(crate) fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> Result<MutexGuard<'a, T>> {
        let cond = || {
            let mut counter = self.counter.lock();
            if counter.notify_count > 0 {
                counter.notify_count -= 1;
                Some(())
            } else {
                None
            }
        };
        {
            let mut counter = self.counter.lock();
            counter.waiter_count += 1;
        }
        let lock = MutexGuard::get_lock(&guard);
        drop(guard);
        self.waitqueue.wait_until(cond);
        Ok(lock.lock())
    }

    pub(crate) fn notify_one(&self) {
        let mut counter = self.counter.lock();
        if counter.waiter_count == 0 {
            return;
        }
        counter.notify_count += 1;
        self.waitqueue.wake_one();
        counter.waiter_count -= 1;
    }

    pub(crate) fn notify_all(&self) {
        let mut counter = self.counter.lock();
        if counter.waiter_count == 0 {
            return;
        }
        counter.notify_count = counter.waiter_count;
        self.waitqueue.wake_all();
        counter.waiter_count = 0;
    }
}

impl fmt::Debug for Condvar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Condvar").finish_non_exhaustive()
    }
}

#[repr(transparent)]
pub(crate) struct CvarMutex<T> {
    inner: Mutex<T>,
}

impl<T> CvarMutex<T> {
    pub(crate) fn new(t: T) -> Self {
        Self {
            inner: Mutex::new(t),
        }
    }

    pub(crate) fn lock(&self) -> Result<MutexGuard<'_, T>> {
        let guard = self.inner.lock();
        Ok(guard)
    }
}

impl<T: fmt::Debug> fmt::Debug for CvarMutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("No data, since `CvarMutex` does't support `try_lock` now")
    }
}

pub(crate) fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + Sync + 'static,
    T: Send + 'static,
{
    let is_finished = Arc::new(AtomicBool::new(false));
    let data = Arc::new(SpinLock::new(None));

    let is_finished_clone = is_finished.clone();
    let data_clone = data.clone();
    let task = TaskOptions::new(move || {
        let data = f();
        *data_clone.lock() = Some(data);
        is_finished_clone.store(true, Ordering::Release);
    })
    .spawn()
    .unwrap();

    JoinHandle {
        task,
        is_finished,
        data,
    }
}

pub(crate) struct JoinHandle<T> {
    task: Arc<Task>,
    is_finished: Arc<AtomicBool>,
    data: Arc<SpinLock<Option<T>>>,
}

impl<T> JoinHandle<T> {
    pub(crate) fn is_finished(&self) -> bool {
        self.is_finished.load(Ordering::Acquire)
    }

    pub(crate) fn join(self) -> Result<T> {
        while !self.is_finished() {
            Task::yield_now();
        }

        let data = self.data.lock().take().unwrap();
        Ok(data)
    }
}

impl<T> fmt::Debug for JoinHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JoinHandle").finish_non_exhaustive()
    }
}

pub struct Rng;

impl crate::util::Rng for Rng {
    fn new(_seed: &[u8]) -> Self {
        Self
    }

    fn fill_bytes(&self, dest: &mut [u8]) -> Result<()> {
        let (chunks, remain) = dest.as_chunks_mut::<8>();
        chunks.iter_mut().for_each(|chunk| {
            chunk.copy_from_slice(read_random().unwrap_or(0u64).as_bytes());
        });
        remain.copy_from_slice(&read_random().unwrap_or(0u64).as_bytes()[..remain.len()]);
        Ok(())
    }
}

macro_rules! new_byte_array_type {
    ($vis:vis $name:ident, $n:expr) => {
        #[repr(C)]
        #[derive(Clone, Copy, Debug, Default, Deserialize, Pod, Serialize)]
        $vis struct $name([u8; $n]);

        impl core::ops::Deref for $name {
            type Target = [u8];

            fn deref(&self) -> &Self::Target {
                self.0.as_slice()
            }
        }

        impl core::ops::DerefMut for $name {
            fn deref_mut(&mut self) -> &mut Self::Target {
                self.0.as_mut_slice()
            }
        }

        impl crate::util::RandomInit for $name {
            fn random() -> Self {
                use crate::util::Rng;

                let mut result = Self::default();
                let rng = self::Rng::new(&[]);
                rng.fill_bytes(&mut result).unwrap_or_default();
                result
            }
        }
    };
}

const AES_GCM_KEY_SIZE: usize = 16;
const AES_GCM_IV_SIZE: usize = 12;
const AES_GCM_MAC_SIZE: usize = 16;

new_byte_array_type!(pub AeadKey, AES_GCM_KEY_SIZE);
new_byte_array_type!(pub AeadIv, AES_GCM_IV_SIZE);
new_byte_array_type!(pub AeadMac, AES_GCM_MAC_SIZE);

#[derive(Debug, Default)]
pub struct Aead;

impl Aead {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl crate::util::Aead for Aead {
    type Key = AeadKey;
    type Iv = AeadIv;
    type Mac = AeadMac;

    fn encrypt(
        &self,
        input: &[u8],
        key: &AeadKey,
        iv: &AeadIv,
        aad: &[u8],
        output: &mut [u8],
    ) -> Result<AeadMac> {
        let key = Key::<Aes128Gcm>::from_slice(key);
        let nonce = Nonce::<Aes128Gcm>::from_slice(iv);
        let cipher = Aes128Gcm::new(key);

        output.copy_from_slice(input);
        let tag = cipher
            .encrypt_in_place_detached(nonce, aad, output)
            .map_err(|_| Error::with_msg(Errno::EncryptFailed, "aes-128-gcm encryption failed"))?;

        let mut aead_mac = AeadMac::new_zeroed();
        aead_mac.copy_from_slice(&tag);
        Ok(aead_mac)
    }

    fn decrypt(
        &self,
        input: &[u8],
        key: &AeadKey,
        iv: &AeadIv,
        aad: &[u8],
        mac: &AeadMac,
        output: &mut [u8],
    ) -> Result<()> {
        let key = Key::<Aes128Gcm>::from_slice(key);
        let nonce = Nonce::<Aes128Gcm>::from_slice(iv);
        let tag = Tag::<Aes128Gcm>::from_slice(mac);
        let cipher = Aes128Gcm::new(key);

        output.copy_from_slice(input);
        cipher
            .decrypt_in_place_detached(nonce, aad, output, tag)
            .map_err(|_| Error::with_msg(Errno::DecryptFailed, "aes-128-gcm decryption failed"))
    }
}

type Aes128Ctr = ctr::Ctr128LE<Aes128>;

const AES_CTR_KEY_SIZE: usize = 16;
const AES_CTR_IV_SIZE: usize = 16;

new_byte_array_type!(pub(crate) SkcipherKey, AES_CTR_KEY_SIZE);
new_byte_array_type!(pub(crate) SkcipherIv, AES_CTR_IV_SIZE);

#[derive(Debug, Default)]
pub(crate) struct Skcipher;

impl Skcipher {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl crate::util::Skcipher for Skcipher {
    type Key = SkcipherKey;
    type Iv = SkcipherIv;

    fn encrypt(
        &self,
        input: &[u8],
        key: &SkcipherKey,
        iv: &SkcipherIv,
        output: &mut [u8],
    ) -> Result<()> {
        let mut cipher = Aes128Ctr::new_from_slices(key, iv).unwrap();
        output.copy_from_slice(input);
        cipher.apply_keystream(output);
        Ok(())
    }

    fn decrypt(
        &self,
        input: &[u8],
        key: &SkcipherKey,
        iv: &SkcipherIv,
        output: &mut [u8],
    ) -> Result<()> {
        let mut cipher = Aes128Ctr::new_from_slices(key, iv).unwrap();
        output.copy_from_slice(input);
        cipher.apply_keystream(output);
        Ok(())
    }
}

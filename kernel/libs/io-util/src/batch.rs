extern crate alloc;

use alloc::sync::Arc;
use core::{any::Any, ops};

use smallvec::SmallVec;

use crate::IoError;

pub trait IoCompletion: Send + Sync + Any {
    fn wait(&self) -> Result<(), IoError>;
}

impl dyn IoCompletion {
    pub fn downcast_ref<T: IoCompletion + 'static>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }
}

#[must_use]
pub struct IoBatch {
    completions: SmallVec<[Arc<dyn IoCompletion>; INLINE_CAPACITY]>,
}

const INLINE_CAPACITY: usize = 1;

impl IoBatch {
    pub fn new() -> Self {
        Self {
            completions: SmallVec::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            completions: SmallVec::with_capacity(capacity),
        }
    }

    pub fn push(&mut self, completion: Arc<dyn IoCompletion>) {
        self.completions.push(completion);
    }

    pub fn len(&self) -> usize {
        self.completions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.completions.is_empty()
    }

    pub fn wait_all(&self) -> Result<(), IoError> {
        let mut first_error = None;

        for completion in &self.completions {
            if let Err(error) = completion.wait() {
                first_error.get_or_insert(error);
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Default for IoBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl ops::Index<usize> for IoBatch {
    type Output = Arc<dyn IoCompletion>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.completions[index]
    }
}

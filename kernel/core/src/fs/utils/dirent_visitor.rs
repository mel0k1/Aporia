#![expect(unused_variables)]

use crate::{fs::file::InodeType, prelude::*};

pub trait DirentVisitor {
    fn visit(&mut self, name: &str, ino: u64, type_: InodeType, offset: usize) -> Result<()>;
}

impl DirentVisitor for Vec<String> {
    fn visit(&mut self, name: &str, ino: u64, type_: InodeType, offset: usize) -> Result<()> {
        self.push(name.into());
        Ok(())
    }
}

use crate::prelude::*;

pub(crate) trait CopyCompat {
    fn read_val_compat<T: Pod>(&self, src: Vaddr, size: usize) -> Result<T>;

    fn write_val_compat<'a, T: Pod>(
        &self,
        dest: Vaddr,
        size: usize,
        val: &'a T,
    ) -> Result<TrailingBytes<'a>>;
}

impl CopyCompat for CurrentUserSpace<'_> {
    fn read_val_compat<T: Pod>(&self, src: Vaddr, size: usize) -> Result<T> {
        let mut val = T::new_zeroed();

        let mut reader = self.reader(src, size)?;
        reader.read_fallible(&mut VmWriter::from(val.as_mut_bytes()))?;

        while reader.remain() > size_of::<u64>() {
            if reader.read_val::<u64>()? != 0 {
                return Err(errno!(E2BIG, "the user structure is not compatible"));
            }
        }
        while reader.has_remain() {
            if reader.read_val::<u8>()? != 0 {
                return Err(errno!(E2BIG, "the user structure is not compatible"));
            }
        }

        Ok(val)
    }

    fn write_val_compat<'a, T: Pod>(
        &self,
        dest: Vaddr,
        size: usize,
        val: &'a T,
    ) -> Result<TrailingBytes<'a>> {
        let mut writer = self.writer(dest, size)?;
        writer.write_fallible(&mut VmReader::from(val.as_bytes()))?;

        if size < size_of::<T>() {
            Ok(TrailingBytes(&val.as_bytes()[size..]))
        } else {
            writer.fill_zeros(writer.avail())?;
            Ok(TrailingBytes(&[]))
        }
    }
}

#[expect(dead_code)]
#[must_use]
pub(crate) struct TrailingBytes<'a>(&'a [u8]);

impl TrailingBytes<'_> {
    pub(crate) fn ignore_trailing(self) {}
}

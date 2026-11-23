use core::fmt::{Arguments, Write};

use ostd::mm::{FallibleVmWrite, VmReader, VmWriter};

pub struct VmPrinter<'a, 'b> {
    writer: &'a mut VmWriter<'b>,
    bytes_to_skip: usize,
    bytes_written: usize,
}

impl<'a, 'b> VmPrinter<'a, 'b> {
    fn new(writer: &'a mut VmWriter<'b>) -> Self {
        Self {
            writer,
            bytes_to_skip: 0,
            bytes_written: 0,
        }
    }

    pub fn new_skip(writer: &'a mut VmWriter<'b>, bytes_to_skip: usize) -> Self {
        Self {
            writer,
            bytes_to_skip,
            bytes_written: 0,
        }
    }

    pub fn bytes_written(&self) -> usize {
        self.bytes_written
    }

    pub fn write_fmt(&mut self, args: Arguments<'_>) -> Result<(), VmPrinterError> {
        Write::write_fmt(self, args).map_err(|_| VmPrinterError::PageFault)
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), VmPrinterError> {
        if self.bytes_to_skip >= bytes.len() {
            self.bytes_to_skip -= bytes.len();
            return Ok(());
        }

        let bytes_to_write = &bytes[self.bytes_to_skip..];
        if self.bytes_to_skip > 0 {
            self.bytes_to_skip = 0;
        }

        let mut reader = VmReader::from(bytes_to_write);
        let written_len = self
            .writer
            .write_fallible(&mut reader)
            .map_err(|_| VmPrinterError::PageFault)?;

        self.bytes_written += written_len;

        Ok(())
    }
}

impl Write for VmPrinter<'_, '_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_bytes(s.as_bytes()).map_err(|_| core::fmt::Error)
    }
}

impl<'a, 'b> From<&'a mut VmWriter<'b>> for VmPrinter<'a, 'b> {
    fn from(writer: &'a mut VmWriter<'b>) -> Self {
        Self::new(writer)
    }
}

pub enum VmPrinterError {
    PageFault,
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;
    use ostd_pod::IntoBytes;

    use super::*;

    #[ktest]
    fn basic_write() {
        let mut buf = [0u8; 64];
        let mut writer = VmWriter::from(buf.as_mut_bytes()).to_fallible();
        let mut printer = VmPrinter::from(&mut writer);

        let res = writeln!(printer, "test");
        assert!(res.is_ok());

        assert_eq!(printer.bytes_written(), 5);
        assert_eq!(&buf[..5], b"test\n");
    }

    #[ktest]
    fn write_with_skip() {
        let mut buf = [0u8; 3];
        let mut writer = VmWriter::from(buf.as_mut_bytes()).to_fallible();
        let mut printer = VmPrinter::new_skip(&mut writer, 3);

        let res = writeln!(printer, "val: {}", 123);
        assert!(res.is_ok());

        assert_eq!(printer.bytes_written(), 3);
        assert_eq!(&buf, b": 1");
    }

    #[ktest]
    fn skip_all_content() {
        let mut buf = [0u8; 64];
        let mut writer = VmWriter::from(buf.as_mut_bytes()).to_fallible();
        let mut printer = VmPrinter::new_skip(&mut writer, 100);

        let res = writeln!(printer, "short message");
        assert!(res.is_ok());

        assert_eq!(printer.bytes_written(), 0);
        assert_eq!(buf[0], 0);
    }
}

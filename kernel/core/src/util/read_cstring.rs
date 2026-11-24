use ostd::mm::Fallible;

use super::VmReaderArray;
use crate::prelude::*;

pub(crate) trait ReadCString {
    fn read_cstring_until_nul(&mut self, max_len: usize) -> Result<Option<CString>>;

    fn read_cstring_until_end(&mut self, max_len: usize) -> Result<(CString, usize)>;
}

const INIT_ALLOC_SIZE: usize = 128;

impl ReadCString for VmReader<'_, Fallible> {
    fn read_cstring_until_nul(&mut self, max_len: usize) -> Result<Option<CString>> {
        let mut buffer: Vec<u8> = Vec::with_capacity(INIT_ALLOC_SIZE.min(max_len));

        if read_until_nul_byte(self, &mut buffer, max_len)? {
            return Ok(Some(CString::from_vec_with_nul(buffer).unwrap()));
        }

        Ok(None)
    }

    fn read_cstring_until_end(&mut self, max_len: usize) -> Result<(CString, usize)> {
        let mut buffer: Vec<u8> = Vec::with_capacity(INIT_ALLOC_SIZE.min(max_len));

        if read_until_nul_byte(self, &mut buffer, max_len)? {
            let buffer_len = buffer.len();
            return Ok((CString::from_vec_with_nul(buffer).unwrap(), buffer_len));
        }

        let buffer_len = buffer.len();
        Ok((CString::new(buffer).unwrap(), buffer_len))
    }
}

impl ReadCString for VmReaderArray<'_> {
    fn read_cstring_until_nul(&mut self, max_len: usize) -> Result<Option<CString>> {
        let mut buffer: Vec<u8> = Vec::with_capacity(INIT_ALLOC_SIZE.min(max_len));

        for reader in self.readers_mut() {
            if read_until_nul_byte(reader, &mut buffer, max_len)? {
                return Ok(Some(CString::from_vec_with_nul(buffer).unwrap()));
            }
        }

        Ok(None)
    }

    fn read_cstring_until_end(&mut self, max_len: usize) -> Result<(CString, usize)> {
        let mut buffer: Vec<u8> = Vec::with_capacity(INIT_ALLOC_SIZE.min(max_len));

        for reader in self.readers_mut() {
            if read_until_nul_byte(reader, &mut buffer, max_len)? {
                let buffer_len = buffer.len();
                return Ok((CString::from_vec_with_nul(buffer).unwrap(), buffer_len));
            }
        }

        let buffer_len = buffer.len();
        Ok((CString::new(buffer).unwrap(), buffer_len))
    }
}

fn read_until_nul_byte(
    reader: &mut VmReader,
    buffer: &mut Vec<u8>,
    max_len: usize,
) -> Result<bool> {
    macro_rules! read_one_byte_at_a_time_while {
        ($cond:expr) => {
            while $cond {
                let byte = reader.read_val::<u8>()?;
                buffer.push(byte);
                if byte == 0 {
                    return Ok(true);
                }
            }
        };
    }

    read_one_byte_at_a_time_while!(
        !is_addr_aligned(reader.cursor() as usize) && buffer.len() < max_len && reader.has_remain()
    );

    let mut cloned_reader = reader.clone();
    while (buffer.len() + size_of::<usize>()) <= max_len {
        let Ok(word) = cloned_reader.read_val::<usize>() else {
            break;
        };

        if has_zero(word) {
            for byte in word.to_ne_bytes() {
                reader.skip(1);
                buffer.push(byte);
                if byte == 0 {
                    return Ok(true);
                }
            }
            unreachable!("The branch should never be reached unless `has_zero` has bugs.")
        }

        reader.skip(size_of::<usize>());
        buffer.extend_from_slice(&word.to_ne_bytes());
    }

    read_one_byte_at_a_time_while!(buffer.len() < max_len && reader.has_remain());

    Ok(false)
}

const fn has_zero(value: usize) -> bool {
    const ONE_BITS: usize = usize::from_le_bytes([0x01; size_of::<usize>()]);
    const HIGH_BITS: usize = usize::from_le_bytes([0x80; size_of::<usize>()]);

    value.wrapping_sub(ONE_BITS) & !value & HIGH_BITS != 0
}

const fn is_addr_aligned(addr: usize) -> bool {
    (addr & (size_of::<usize>() - 1)) == 0
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;
    use crate::util::MultiRead;

    fn init_buffer(cstrs: &[CString]) -> Vec<u8> {
        let mut buffer = vec![255u8; 100];

        let mut writer = VmWriter::from(buffer.as_mut_slice());

        for cstr in cstrs {
            writer.write(&mut VmReader::from(cstr.as_bytes_with_nul()));
        }

        buffer
    }

    #[ktest]
    fn read_multiple_cstring() {
        let strs = {
            let str1 = CString::new("hello").unwrap();
            let str2 = CString::new("world!").unwrap();
            vec![str1, str2]
        };

        let buffer = init_buffer(&strs);

        let mut reader = VmReader::from(buffer.as_slice()).to_fallible();
        let read_str1 = reader.read_cstring_until_nul(1024).unwrap();
        assert_eq!(read_str1.as_ref(), Some(&strs[0]));
        let read_str2 = reader.read_cstring_until_nul(1024).unwrap();
        assert_eq!(read_str2.as_ref(), Some(&strs[1]));

        assert_eq!(reader.read_cstring_until_nul(1024).unwrap(), None);
    }

    #[ktest]
    fn read_cstring_from_multiread() {
        let strs = {
            let str1 = CString::new("hello").unwrap();
            let str2 = CString::new("world!").unwrap();
            let str3 = CString::new("aporia").unwrap();
            vec![str1, str2, str3]
        };

        let buffer = init_buffer(&strs);

        let mut readers = {
            let reader1 = VmReader::from(&buffer[0..20]).to_fallible();
            let reader2 = VmReader::from(&buffer[20..40]).to_fallible();
            let reader3 = VmReader::from(&buffer[40..60]).to_fallible();
            VmReaderArray::new(vec![reader1, reader2, reader3].into_boxed_slice())
        };

        let multiread = &mut readers as &mut dyn MultiRead;
        let read_str1 = multiread.read_cstring_until_nul(1024).unwrap();
        assert_eq!(read_str1.as_ref(), Some(&strs[0]));
        let read_str2 = multiread.read_cstring_until_nul(1024).unwrap();
        assert_eq!(read_str2.as_ref(), Some(&strs[1]));
        let read_str3 = multiread.read_cstring_until_nul(1024).unwrap();
        assert_eq!(read_str3.as_ref(), Some(&strs[2]));

        assert_eq!(multiread.read_cstring_until_nul(1024).unwrap(), None);
    }

    #[ktest]
    fn read_cstring_until_end() {
        let strs = {
            let str1 = CString::new("hello").unwrap();
            let str2 = CString::new("world!").unwrap();
            vec![str1, str2]
        };

        let buffer = init_buffer(&strs);

        let mut readers = {
            let reader1 = VmReader::from(&buffer[0..3]).to_fallible();
            let reader2 = VmReader::from(&buffer[3..10]).to_fallible();
            let reader3 = VmReader::from(&buffer[10..60]).to_fallible();
            VmReaderArray::new(vec![reader1, reader2, reader3].into_boxed_slice())
        };

        let multiread = &mut readers as &mut dyn MultiRead;
        let (read_str1, read_len1) = multiread.read_cstring_until_end(4).unwrap();
        assert_eq!(read_str1.as_bytes(), b"hell");
        assert_eq!(read_len1, 4);
        let (read_str2, read_len2) = multiread.read_cstring_until_end(4).unwrap();
        assert_eq!(read_str2.as_bytes(), b"o");
        assert_eq!(read_len2, 2);
        let (read_str3, read_len3) = multiread.read_cstring_until_end(6).unwrap();
        assert_eq!(read_str3.as_bytes(), b"world!");
        assert_eq!(read_len3, 6);
        let (read_str4, read_len4) = multiread.read_cstring_until_end(6).unwrap();
        assert_eq!(read_str4.as_bytes(), b"");
        assert_eq!(read_len4, 1);
    }
}

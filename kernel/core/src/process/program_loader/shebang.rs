use crate::prelude::*;

pub(crate) fn parse_shebang_line(file_first_page: &[u8]) -> Result<Option<Vec<CString>>> {
    if !file_first_page.starts_with(b"#!") {
        return Ok(None);
    }

    let Some(first_line_len) = file_first_page.iter().position(|&c| c == b'\n') else {
        return Err(errno!(ENOEXEC, "the shebang line is too long"));
    };

    let shebang_header = &file_first_page[2..first_line_len];
    let mut shebang_argv = Vec::new();
    for arg in shebang_header.split(|&c| c == b' ' || c == b'\t') {
        if arg.is_empty() {
            continue;
        }

        let arg =
            CString::new(arg).map_err(|_| errno!(ENOEXEC, "unexpected nul terminator is found"))?;
        shebang_argv.push(arg);
    }

    if shebang_argv.is_empty() {
        return Err(errno!(ENOEXEC, "no interpreter program is found"));
    }

    Ok(Some(shebang_argv))
}

#[cfg(ktest)]
mod test {
    use alloc::{ffi::CString, vec};

    use ostd::prelude::*;

    use super::parse_shebang_line;

    #[ktest]
    fn parse_shebang_line_with_multiple_args() {
        const LINE1: &str = "#! /bin/bash -e\n";
        let res = parse_shebang_line(LINE1.as_bytes()).unwrap().unwrap();
        assert_eq!(
            res,
            vec![
                CString::new("/bin/bash").unwrap(),
                CString::new("-e").unwrap()
            ]
        );

        const LINE2: &str = "#!  /bin/env  python3  \n";
        let res = parse_shebang_line(LINE2.as_bytes()).unwrap().unwrap();
        assert_eq!(
            res,
            vec![
                CString::new("/bin/env").unwrap(),
                CString::new("python3").unwrap()
            ]
        );
    }
}

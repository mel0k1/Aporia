use core::ops::Range;

use xmas_elf::{
    header::{self, Header, HeaderPt1, HeaderPt2, HeaderPt2_, Machine_, Type_},
    program::{self, ProgramHeader64},
};

use crate::{
    fs::{file::FileLike, utils::PATH_MAX},
    prelude::*,
    vm::{perms::VmPerms, vmar::VMAR_CAP_ADDR},
};

pub(crate) struct ElfHeaders {
    elf_header: ElfHeader,
    loadable_phdrs: Vec<LoadablePhdr>,
    max_load_align: usize,
    interp_phdr: Option<InterpPhdr>,
}

impl ElfHeaders {
    pub(super) const LEN: usize = size_of::<HeaderPt1>() + size_of::<HeaderPt2_64>();

    pub(crate) fn parse(input: &[u8]) -> Result<Self> {
        let header = header::parse_header(input)
            .map_err(|_| errno!(ENOEXEC, "the ELF header is invalid"))?;
        let elf_header = ElfHeader::parse(header)?;
        check_elf_header(&elf_header)?;

        let ph_count = elf_header.pt2.ph_count;
        let ph_entry_size = elf_header.pt2.ph_entry_size;
        let ph_offset = elf_header.pt2.ph_offset;
        if ph_entry_size as usize != size_of::<ProgramHeader64>() {
            return Err(errno!(
                ENOEXEC,
                "the size of ELF program headers is invalid"
            ));
        }
        if ph_offset >= input.len() as u64
            || (input.len() as u64 - ph_offset) / (ph_entry_size as u64) < (ph_count as u64)
        {
            return Err(errno!(
                ENOEXEC,
                "the placement of ELF program headers is not supported"
            ));
        }

        let mut loadable_phdrs = Vec::with_capacity(ph_count as usize);
        let mut max_load_align = PAGE_SIZE;
        let mut interp_phdr = None;
        for index in 0..ph_count {
            let program_header = program::parse_program_header(input, header, index)
                .map_err(|_| errno!(ENOEXEC, "the ELF program header is invalid"))?;
            let ph64 = match program_header {
                program::ProgramHeader::Ph64(ph64) => *ph64,
                program::ProgramHeader::Ph32(_) => {
                    return Err(errno!(ENOEXEC, "the ELF program header is not 64-bit"));
                }
            };
            match ph64.get_type() {
                Ok(program::Type::Load) => {
                    loadable_phdrs.push(LoadablePhdr::parse(&ph64)?);
                    if ph64.align.is_power_of_two() {
                        max_load_align = max_load_align.max(ph64.align as usize);
                    }
                }
                Ok(program::Type::Interp) if interp_phdr.is_none() => {
                    interp_phdr = Some(InterpPhdr::parse(&ph64)?);
                }
                _ => (),
            }
        }
        if loadable_phdrs.is_empty() {
            return Err(errno!(ENOEXEC, "there are no loadable ELF program headers"));
        }

        Ok(Self {
            elf_header,
            loadable_phdrs,
            max_load_align,
            interp_phdr,
        })
    }

    pub(super) fn is_shared_object(&self) -> bool {
        self.elf_header.pt2.type_.as_type() == header::Type::SharedObject
    }

    pub(super) fn entry_point(&self) -> Vaddr {
        self.elf_header.pt2.entry_point as Vaddr
    }

    pub(super) fn ph_count(&self) -> u16 {
        self.elf_header.pt2.ph_count
    }

    pub(super) fn ph_ent(&self) -> u16 {
        self.elf_header.pt2.ph_entry_size
    }

    pub(super) fn loadable_phdrs(&self) -> &[LoadablePhdr] {
        self.loadable_phdrs.as_slice()
    }

    pub(super) fn max_load_align(&self) -> usize {
        self.max_load_align
    }

    pub(super) fn interp_phdr(&self) -> Option<&InterpPhdr> {
        self.interp_phdr.as_ref()
    }

    pub(super) fn find_vaddr_of_phdrs(&self) -> Result<Vaddr> {
        let ph_offset = self.elf_header.pt2.ph_offset as usize;
        for loadable_phdrs in self.loadable_phdrs.iter() {
            if loadable_phdrs.file_range().contains(&ph_offset) {
                return Ok(loadable_phdrs.virt_range().start
                    + (ph_offset - loadable_phdrs.file_range().start));
            }
        }
        Err(errno!(
            ENOEXEC,
            "the ELF program headers are not located in any segments"
        ))
    }

    pub(super) fn calc_total_vaddr_bounds(&self) -> Range<Vaddr> {
        self.loadable_phdrs
            .iter()
            .map(LoadablePhdr::virt_range)
            .cloned()
            .reduce(|r1, r2| r1.start.min(r2.start)..r1.end.max(r2.end))
            .unwrap()
    }

    pub(super) fn find_last_vaddr_bound(&self) -> Option<Range<Vaddr>> {
        self.loadable_phdrs
            .iter()
            .max_by_key(|phdr| phdr.virt_range().end)
            .map(|phdr| phdr.virt_range().clone())
    }
}

struct ElfHeader {
    pt1: HeaderPt1,
    pt2: HeaderPt2_64,
}

impl ElfHeader {
    pub(self) fn parse(header: Header) -> Result<Self> {
        let pt1 = *header.pt1;
        let pt2 = match header.pt2 {
            HeaderPt2::Header64(header_pt2) => {
                let HeaderPt2_ {
                    type_,
                    machine,
                    version,
                    entry_point,
                    ph_offset,
                    sh_offset,
                    flags,
                    header_size,
                    ph_entry_size,
                    ph_count,
                    sh_entry_size,
                    sh_count,
                    sh_str_index,
                } = header_pt2;
                HeaderPt2_64 {
                    type_: *type_,
                    machine: *machine,
                    version: *version,
                    entry_point: *entry_point,
                    ph_offset: *ph_offset,
                    sh_offset: *sh_offset,
                    flags: *flags,
                    header_size: *header_size,
                    ph_entry_size: *ph_entry_size,
                    ph_count: *ph_count,
                    sh_entry_size: *sh_entry_size,
                    sh_count: *sh_count,
                    sh_str_index: *sh_str_index,
                }
            }
            _ => return Err(errno!(ENOEXEC, "the ELF file is not 64-bit")),
        };
        Ok(ElfHeader { pt1, pt2 })
    }
}

struct HeaderPt2_64 {
    type_: Type_,
    machine: Machine_,
    #[expect(dead_code)]
    version: u32,
    entry_point: u64,
    ph_offset: u64,
    #[expect(dead_code)]
    sh_offset: u64,
    #[expect(dead_code)]
    flags: u32,
    #[expect(dead_code)]
    header_size: u16,
    ph_entry_size: u16,
    ph_count: u16,
    #[expect(dead_code)]
    sh_entry_size: u16,
    #[expect(dead_code)]
    sh_count: u16,
    #[expect(dead_code)]
    sh_str_index: u16,
}

fn check_elf_header(elf_header: &ElfHeader) -> Result<()> {
    #[cfg(target_arch = "x86_64")]
    const EXPECTED_ELF_MACHINE: header::Machine = header::Machine::X86_64;
    #[cfg(target_arch = "riscv64")]
    const EXPECTED_ELF_MACHINE: header::Machine = header::Machine::RISC_V;
    #[cfg(target_arch = "loongarch64")]
    const EXPECTED_ELF_MACHINE: header::Machine = header::Machine::Other(258);
    #[cfg(target_arch = "aarch64")]
    const EXPECTED_ELF_MACHINE: header::Machine = header::Machine::AArch64;

    if elf_header.pt1.class() != header::Class::SixtyFour {
        return Err(errno!(ENOEXEC, "the ELF file is not 64-bit"));
    }

    if elf_header.pt1.data() != header::Data::LittleEndian {
        return Err(errno!(ENOEXEC, "the ELF file is not in little endian"));
    }

    if elf_header.pt2.machine.as_machine() != EXPECTED_ELF_MACHINE {
        return Err(errno!(
            ENOEXEC,
            "the ELF file is of a different architecture"
        ));
    }

    let elf_type = elf_header.pt2.type_.as_type();
    if elf_type != header::Type::Executable && elf_type != header::Type::SharedObject {
        return Err(errno!(ENOEXEC, "the ELF file is not an executable"));
    }

    Ok(())
}

pub(super) struct LoadablePhdr {
    virt_range: Range<Vaddr>,
    file_range: Range<usize>,
    vm_perms: VmPerms,
}

impl LoadablePhdr {
    pub(self) fn parse(phdr: &ProgramHeader64) -> Result<Self> {
        debug_assert_eq!(phdr.get_type(), Ok(program::Type::Load));

        let virt_start = phdr.virtual_addr;
        let virt_end = if let Some(virt_end) = virt_start.checked_add(phdr.mem_size)
            && virt_end <= VMAR_CAP_ADDR as u64
        {
            virt_end
        } else {
            return Err(errno!(ENOMEM, "the mapping address is too large"));
        };

        let file_start = phdr.offset;
        let Some(file_end) = file_start.checked_add(phdr.file_size) else {
            return Err(errno!(EINVAL, "the mapping offset overflows"));
        };
        if file_end >= isize::MAX as u64 {
            return Err(errno!(EOVERFLOW, "the mapping offset overflows"));
        }

        if phdr.mem_size == 0 {
            return Err(errno!(EINVAL, "the mapping length is zero"));
        }
        if phdr.mem_size < phdr.file_size {
            return Err(errno!(
                EINVAL,
                "the mapping length is smaller than the file length"
            ));
        }
        if virt_start % (PAGE_SIZE as u64) != file_start % (PAGE_SIZE as u64) {
            return Err(errno!(EINVAL, "the mapping address is not aligned"));
        }

        Ok(Self {
            virt_range: (virt_start as Vaddr)..(virt_end as Vaddr),
            file_range: (file_start as usize)..(file_end as usize),
            vm_perms: parse_segment_perm(phdr.flags),
        })
    }

    pub(super) fn virt_range(&self) -> &Range<Vaddr> {
        &self.virt_range
    }

    pub(super) fn file_range(&self) -> &Range<usize> {
        &self.file_range
    }

    pub(super) fn vm_perms(&self) -> VmPerms {
        self.vm_perms
    }
}

fn parse_segment_perm(flags: program::Flags) -> VmPerms {
    let mut vm_perm = VmPerms::empty();
    if flags.is_read() {
        vm_perm |= VmPerms::READ;
    }
    if flags.is_write() {
        vm_perm |= VmPerms::WRITE;
    }
    if flags.is_execute() {
        vm_perm |= VmPerms::EXEC;
    }
    vm_perm
}

pub(super) struct InterpPhdr {
    file_offset: usize,
    file_size: u16,
}

impl InterpPhdr {
    pub(self) fn parse(phdr: &ProgramHeader64) -> Result<Self> {
        debug_assert_eq!(phdr.get_type(), Ok(program::Type::Interp));

        if phdr
            .offset
            .checked_add(phdr.file_size)
            .is_none_or(|file_end| file_end > isize::MAX as u64)
        {
            return Err(errno!(EINVAL, "the interpreter offset overflows"));
        }

        if phdr.file_size >= PATH_MAX as u64 {
            return Err(errno!(ENOEXEC, "the interpreter path is too long"));
        }

        const { assert!(PATH_MAX as u64 <= u16::MAX as u64) };

        Ok(Self {
            file_offset: phdr.offset as usize,
            file_size: phdr.file_size as u16,
        })
    }

    pub(super) fn read_ldso_path(&self, elf_file: &Arc<dyn FileLike>) -> Result<CString> {
        let file_size = self.file_size as usize;
        let mut buffer = vec![0; file_size];
        if elf_file.read_bytes_at(self.file_offset, &mut buffer)? != file_size {
            return Err(errno!(EIO, "the interpreter path cannot be fully read"));
        }

        let ldso_path = CString::from_vec_with_nul(buffer)
            .map_err(|_| errno!(ENOEXEC, "the interpreter path is not a valid C string",))?;
        Ok(ldso_path)
    }
}

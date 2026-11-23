#[macro_use]
extern crate ostd_pod;

pub mod encoder;
mod mapping;
mod pe_header;

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

use align_ext::AlignExt;
pub use encoder::{PayloadEncoding, encode_kernel};
use mapping::{SetupFileOffset, SetupVA};
use xmas_elf::{program::SegmentData, sections::SectionData};

pub enum BzImageType {
    Legacy32,
    Efi64,
}

pub fn make_bzimage(target_image_path: &Path, image_type: BzImageType, setup_elf_path: &Path) {
    let mut setup_elf = Vec::new();
    File::open(setup_elf_path)
        .unwrap()
        .read_to_end(&mut setup_elf)
        .unwrap();
    let mut setup = to_flat_binary(&setup_elf);
    setup.resize(setup.len().align_up(pe_header::SECTION_ALIGNMENT), 0x00);

    let mut kernel_image = File::create(target_image_path).unwrap();
    kernel_image.write_all(&setup).unwrap();

    if matches!(image_type, BzImageType::Efi64) {
        assert_elf64_reloc_supported(&setup_elf);

        let pe_header = pe_header::make_pe_coff_header(&setup_elf);
        assert!(pe_header.len() <= 0x1f1, "PE/COFF header is too large");

        kernel_image.seek(SeekFrom::Start(0)).unwrap();
        kernel_image.write_all(&pe_header).unwrap();
    }
}

pub fn legacy32_rust_target_json() -> &'static str {
    include_str!("x86_64-i386_pm-none.json")
}

fn to_flat_binary(elf_file: &[u8]) -> Vec<u8> {
    let elf = xmas_elf::ElfFile::new(elf_file).unwrap();
    let mut bin = Vec::<u8>::new();

    for program in elf.program_iter() {
        if program.get_type().unwrap() == xmas_elf::program::Type::Load {
            let SegmentData::Undefined(header_data) = program.get_data(&elf).unwrap() else {
                panic!("Unexpected segment data type");
            };
            let dst_file_offset = usize::from(SetupFileOffset::from(SetupVA::from(
                program.virtual_addr() as usize,
            )));

            let mem_length = program.mem_size() as usize;
            if bin.len() < dst_file_offset + mem_length {
                bin.resize(dst_file_offset + mem_length, 0);
            }

            let file_length = program.file_size() as usize;
            let dest_slice = bin[dst_file_offset..dst_file_offset + file_length].as_mut();
            dest_slice.copy_from_slice(header_data);
        }
    }

    bin
}

fn assert_elf64_reloc_supported(elf_file: &[u8]) {
    const R_X86_64_RELATIVE: u32 = 8;

    let elf = xmas_elf::ElfFile::new(elf_file).unwrap();

    let SectionData::Rela64(rela64) = elf
        .find_section_by_name(".rela")
        .unwrap()
        .get_data(&elf)
        .unwrap()
    else {
        panic!("the ELF64 relocation data is not of the correct type");
    };

    rela64.iter().for_each(|r| {
        assert_eq!(
            r.get_type(),
            R_X86_64_RELATIVE,
            "the ELF64 relocation type is not supported"
        )
    });
}

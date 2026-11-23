#![expect(dead_code)]
pub(super) const ROOT_INODE_HASH: usize = 0;

pub(super) const MAX_CHARSET_SIZE: usize = 6;
pub(super) const MAX_NAME_LENGTH: usize = 255;
pub(super) const MAX_VFSNAME_BUF_SIZE: usize = (MAX_NAME_LENGTH + 1) * MAX_CHARSET_SIZE;

pub(super) const BOOT_SIGNATURE: u16 = 0xAA55;
pub(super) const EXBOOT_SIGNATURE: u32 = 0xAA550000;
pub(super) const STR_EXFAT: &str = "EXFAT   ";

pub(super) const VOLUME_DIRTY: u16 = 0x0002;
pub(super) const MEDIA_FAILURE: u16 = 0x0004;

pub(super) const EXFAT_RESERVED_CLUSTERS: u32 = 2;
pub(super) const EXFAT_FIRST_CLUSTER: u32 = 2;

pub(super) const EXFAT_MAX_DENTRIES: u32 = 8388608;

pub(super) const EXFAT_FILE_NAME_LEN: usize = 15;

pub(super) const EXFAT_MIN_SECT_SIZE_BITS: u8 = 9;
pub(super) const EXFAT_MAX_SECT_SIZE_BITS: u8 = 12;

pub(super) const EXFAT_MIN_TIMESTAMP_SECS: u64 = 315532800;
pub(super) const EXFAT_MAX_TIMESTAMP_SECS: u64 = 4354819199;

pub(super) const UNICODE_SIZE: usize = 2;

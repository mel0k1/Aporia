use std::{
    ffi::{OsStr, OsString},
    str::FromStr,
};

use miniz_oxide::deflate::{compress_to_vec, compress_to_vec_zlib};
use serde::{Deserialize, Serialize};

const COMPRESSION_LEVEL: u8 = 6;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum PayloadEncoding {
    #[default]
    #[serde(rename = "raw")]
    Raw,
    #[serde(rename = "gzip")]
    Gzip,
    #[serde(rename = "zlib")]
    Zlib,
}

impl FromStr for PayloadEncoding {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "raw" => Ok(Self::Raw),
            "gzip" => Ok(Self::Gzip),
            "zlib" => Ok(Self::Zlib),
            _ => Err(format!("Invalid encoding format: {}", s)),
        }
    }
}

impl From<OsString> for PayloadEncoding {
    fn from(os_string: OsString) -> Self {
        PayloadEncoding::from_str(&os_string.to_string_lossy()).unwrap()
    }
}

impl From<&OsStr> for PayloadEncoding {
    fn from(os_str: &OsStr) -> Self {
        PayloadEncoding::from_str(&os_str.to_string_lossy()).unwrap()
    }
}

pub fn encode_kernel(kernel: Vec<u8>, encoding: PayloadEncoding) -> Vec<u8> {
    match encoding {
        PayloadEncoding::Raw => kernel,
        PayloadEncoding::Gzip => compress_to_vec_gzip(&kernel, COMPRESSION_LEVEL),
        PayloadEncoding::Zlib => compress_to_vec_zlib(&kernel, COMPRESSION_LEVEL),
    }
}

fn compress_to_vec_gzip(input: &[u8], level: u8) -> Vec<u8> {
    let mut out = vec![0x1F, 0x8B, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF];
    out.extend_from_slice(&compress_to_vec(input, level));
    out.extend_from_slice(&crc32fast::hash(input).to_le_bytes());
    out.extend_from_slice(&(input.len() as u32).to_le_bytes());
    out
}

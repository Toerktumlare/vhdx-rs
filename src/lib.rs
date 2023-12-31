use error::VhdxError;
use std::io::{Read, Seek};

pub mod bat;
pub mod bits_parsers;
pub mod error;
pub mod log;
pub mod meta_data;
pub mod parse_utils;
pub mod vhdx;
pub mod vhdx_header;

pub trait DeSerialise<T> {
    type Item;

    fn deserialize(reader: &mut T) -> Result<Self::Item, VhdxError>
    where
        T: Read + Seek;
}

pub trait Crc32 {
    fn crc32(&self) -> u32;
    fn crc32_from_digest(&self, digest: &mut crc::Digest<u32>);
}

pub trait Validation {
    fn validate(&self) -> Result<(), VhdxError>;
}

#[derive(Debug, Eq, PartialEq, Clone, Ord, PartialOrd)]
pub enum Signature {
    Vhdxfile,
    Head,
    Regi,
    Loge,
    Zero,
    Data,
    Desc,
    MetaData,
    Unknown(Vec<u8>),
}

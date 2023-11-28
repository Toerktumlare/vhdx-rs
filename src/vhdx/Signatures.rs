pub const FTI_SIGN: &[u8] = &[0x76, 0x68, 0x64, 0x78, 0x66, 0x69, 0x6C, 0x65];
pub const HEAD_SIGN: &[u8] = &[0x68, 0x65, 0x61, 0x64];
pub const RGT_SIGN: &[u8] = &[0x72, 0x65, 0x67, 0x69];
pub const DESC_SIGN: &[u8] = &[0x63, 0x73, 0x65, 0x64];
pub const DATA_SIGN: &[u8] = &[0x61, 0x74, 0x61, 0x64];
pub const LOGE_SIGN: &[u8] = &[0x63, 0x73, 0x65, 0x64];
pub const ZERO_SIGN: &[u8] = &[0x6F, 0x72, 0x65, 0x7A];

#[derive(Debug, Eq, PartialEq)]
pub enum Signature {
    Vhdxfile,
    Head,
    Regi,
    Loge,
    Zero,
    Data,
    Desc,
    Unknown,
}

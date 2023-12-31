use crc::{Crc, CRC_32_ISCSI};
use nom::Finish;
use std::{
    io::{Read, Seek},
    iter, usize,
};
use uuid::Uuid;

use nom::{
    bytes::complete::take,
    combinator::{map, peek},
    number::complete::{le_u32, le_u64},
    sequence::tuple,
};

use crate::{
    error::VhdxError,
    parse_utils::{t_guid, t_sign_u32, t_u32, t_u64},
    vhdx::Vhdx,
    Crc32, DeSerialise, Signature, Validation,
};

#[derive(Debug)]
pub struct Log {
    pub log_entries: Vec<LogEntry>,
    pub log_sequence: LogSequence,
}

impl Log {
    pub(crate) fn new(log_entries: Vec<LogEntry>) -> Self {
        let entries = log_entries.clone();
        Self {
            log_entries,
            log_sequence: Vhdx::try_get_log_sequence(&entries).unwrap(),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub(crate) header: LogHeader,
    descriptors: Vec<Descriptor>,
}

impl LogEntry {
    const SECTOR_SIZE: usize = 4096;
    const CRC: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

    fn new(header: LogHeader, descriptors: Vec<Descriptor>) -> Self {
        Self {
            header,
            descriptors,
        }
    }
}

impl Validation for LogEntry {
    fn validate(&self) -> Result<(), VhdxError> {
        Ok(())
    }
}

impl<T> DeSerialise<T> for LogEntry {
    type Item = LogEntry;

    fn deserialize(reader: &mut T) -> Result<Self::Item, VhdxError>
    where
        T: Read + Seek,
    {
        let start_pos = reader.stream_position()?;

        let header = LogHeader::deserialize(reader)?;
        let mut descriptors = Vec::with_capacity(header.descript_count as usize);
        if header.descript_count != 0 {
            for _ in 0..header.descript_count {
                let mut buffer = [0; 4];
                reader.read_exact(&mut buffer)?;
                let mut peeker = peek(t_sign_u32);
                let (_, signature) = peeker(&buffer)?;
                reader.seek(std::io::SeekFrom::Current(-4))?;
                let desc = match signature {
                    Signature::Desc => Descriptor::Data(DataDesc::deserialize(reader)?),
                    Signature::Zero => Descriptor::Zero(ZeroDesc::deserialize(reader)?),
                    _ => panic!("Fix this error"),
                };
                descriptors.push(desc);
            }
        }

        let current_pos = reader.stream_position()?;
        let offset = LogEntry::SECTOR_SIZE as u64 - (current_pos - start_pos);
        reader.seek(std::io::SeekFrom::Current(offset as i64))?;

        descriptors.iter_mut().for_each(|v| match v {
            Descriptor::Data(desc) => {
                let d_sector = DataSector::deserialize(reader).unwrap();
                desc.data_sector = Some(d_sector);
            }
            Descriptor::Zero(_) => todo!(),
        });
        let log_entry = LogEntry::new(header, descriptors);
        Ok(log_entry)
    }
}

impl Crc32 for LogEntry {
    fn crc32(&self) -> u32 {
        let mut digest = LogEntry::CRC.digest();
        self.crc32_from_digest(&mut digest);
        digest.finalize()
    }

    fn crc32_from_digest(&self, digest: &mut crc::Digest<u32>) {
        self.header.crc32_from_digest(digest);
        self.descriptors.crc32_from_digest(digest);
    }
}

impl Crc32 for Vec<Descriptor> {
    fn crc32(&self) -> u32 {
        todo!()
    }

    fn crc32_from_digest(&self, digest: &mut crc::Digest<u32>) {
        self.iter().for_each(|desc| {
            desc.crc32_from_digest(digest);
        });

        let zeros: Vec<u8> = iter::repeat(0)
            .take(4096 - ((64 + (self.len() * 32)) % 4096))
            .collect();
        digest.update(&zeros);

        self.iter().for_each(|desc| {
            if let Descriptor::Data(desc) = desc {
                if let Some(data) = &desc.data_sector {
                    data.crc32_from_digest(digest);
                }
            }
        });
    }
}

#[derive(Debug, Clone)]
pub struct LogHeader {
    // Signature (4 bytes): MUST be 0x65676F6C ("loge" as UTF8).
    pub signature: Signature,

    // Checksum (4 bytes): A CRC-32C hash computed over the entire entry specified by the
    // EntryLength field, with the Checksum field taking the value of zero during the computation
    // of the checksum value.
    pub checksum: u32,

    // EntryLength (4 bytes): Specifies the total length of the entry in bytes. The value MUST be a
    // multiple of 4 KB.
    pub entry_length: u32,

    // Tail (4 bytes): The offset, in bytes, from the beginning of the log to the beginning log
    // entry of a sequence ending with this entry. The value MUST be a multiple of 4 KB. A tail
    // entry could point to itself, as would be the case when a log is initialized.
    pub tail: u32,

    // SequenceNumber (8 bytes): A 64-bit integer incremented between each log entry. It must be
    // larger than zero.
    pub seq_number: u64,

    // DescriptorCount (4 bytes): Specifies the number of descriptors that are contained in this
    // log entry. The value can be zero.
    pub descript_count: u32,

    // LogGuid (16 bytes): Contains the LogGuid value in the file header that was present when this
    // log entry was written. When replaying, if this LogGuid does not match the LogGuid field in
    // the file header, this entry MUST NOT be considered valid.
    pub log_guid: Uuid,

    // FlushedFileOffset (8 bytes): Stores the VHDX file size in bytes that MUST be at least as
    // large as the size of the VHDX file at the time the log entry was written. The file size
    // specified in the log entry must have been stable on the host disk such that, even in the
    // case of a system power failure, a noncorrupted VHDX file will be at least as large as the
    // size specified by the log entry. Before shrinking a file while the log is in use, an
    // implementation MUST write the target size to a log entry and flush the entry so that the
    // update is stable on the log that is on the host-disk storage media; this will ensure that
    // the VHDX file is not treated as truncated during log replay. An implementation SHOULD write
    // the largest possible value that satisfies these requirements. The value MUST be a multiple
    // of 1 MB. LastFileOffset (8 bytes): Stores a file size in bytes that all allocated file
    // structures fit into, at the time the log entry was written. An implementation SHOULD write
    // the smallest possible value that satisfies these requirements. The value MUST be a multiple
    // of 1 MB.
    pub flushed_file_offset: u64,

    // LastFileOffset (8 bytes): Stores a file size in bytes that all allocated file structures fit
    // into, at the time the log entry was written. An implementation SHOULD write the smallest
    // possible value that satisfies these requirements. The value MUST be a multiple of 1 MB.
    pub last_file_offset: u64,
}

impl LogHeader {
    pub const SIGN: &'static [u8] = &[0x6C, 0x6F, 0x67, 0x65];
    const CRC: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);
    fn new(
        signature: Signature,
        checksum: u32,
        entry_length: u32,
        tail: u32,
        seq_number: u64,
        descript_count: u32,
        log_guid: Uuid,
        flushed_file_offset: u64,
        last_file_offset: u64,
    ) -> Self {
        Self {
            signature,
            checksum,
            entry_length,
            tail,
            seq_number,
            descript_count,
            log_guid,
            flushed_file_offset,
            last_file_offset,
        }
    }
}

impl<T> DeSerialise<T> for LogHeader {
    type Item = LogHeader;

    fn deserialize(reader: &mut T) -> Result<Self::Item, VhdxError>
    where
        T: Read + Seek,
    {
        let mut buffer = [0; 64];
        reader.read_exact(&mut buffer)?;

        let (_, header) = map(
            tuple((
                t_sign_u32, t_u32, t_u32, t_u32, t_u64, t_u32, t_u32, t_guid, t_u64, t_u64,
            )),
            |(
                signature,
                checksum,
                entry_length,
                tail,
                seq_number,
                descript_count,
                _,
                log_guid,
                flushed_file_offset,
                last_file_offset,
            )| {
                LogHeader::new(
                    signature,
                    checksum,
                    entry_length,
                    tail,
                    seq_number,
                    descript_count,
                    log_guid,
                    flushed_file_offset,
                    last_file_offset,
                )
            },
        )(&buffer)
        .finish()?;
        Ok(header)
    }
}

impl Crc32 for LogHeader {
    fn crc32(&self) -> u32 {
        let mut digest = LogHeader::CRC.digest();
        self.crc32_from_digest(&mut digest);
        digest.finalize()
    }

    fn crc32_from_digest(&self, digest: &mut crc::Digest<u32>) {
        digest.update(LogHeader::SIGN);
        digest.update(&[0; 4]);
        digest.update(&self.entry_length.to_le_bytes());
        digest.update(&self.tail.to_le_bytes());
        digest.update(&self.seq_number.to_le_bytes());
        digest.update(&self.descript_count.to_le_bytes());
        digest.update(&[0; 4]);
        digest.update(&self.log_guid.to_bytes_le());
        digest.update(&self.flushed_file_offset.to_le_bytes());
        digest.update(&self.last_file_offset.to_le_bytes());
    }
}

impl Validation for LogHeader {
    fn validate(&self) -> Result<(), VhdxError> {
        if self.signature != Signature::Loge {
            return Err(VhdxError::SignatureError(
                Signature::Loge,
                self.signature.clone(),
            ));
        }

        // TODO: Calc checksum

        if self.entry_length as u64 % (Vhdx::KB * 4) != 0 {
            return Err(VhdxError::NotDivisbleByMB(
                "Log Entry Length",
                self.entry_length as u64,
            ));
        }

        if self.tail as u64 % (Vhdx::KB * 4) != 0 {
            return Err(VhdxError::NotDivisbleByMB("Log Tail", self.tail as u64));
        }

        if self.seq_number == 0 {
            return Err(VhdxError::NotAllowedToBeZero("Log Sequence Number"));
        }

        if self.descript_count == 0 {
            return Err(VhdxError::NotAllowedToBeZero("Log Description Count"));
        }

        if self.flushed_file_offset % Vhdx::MB != 0 {
            return Err(VhdxError::NotDivisbleByMB(
                "Flushed File Offset",
                self.flushed_file_offset,
            ));
        }

        if self.last_file_offset % Vhdx::MB != 0 {
            return Err(VhdxError::NotDivisbleByMB(
                "Last File Offset",
                self.last_file_offset,
            ));
        }

        Ok(())
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) enum Descriptor {
    Zero(ZeroDesc),
    Data(DataDesc),
}

impl Descriptor {
    const CRC: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);
}

#[derive(Clone)]
pub(crate) struct ZeroDesc {
    // ZeroSignature (4 bytes): MUST be 0x6F72657A ("zero" as ASCII).
    signature: Signature,

    // ZeroLength (8 bytes): Specifies the length of the section to zero. The value MUST be a
    // multiple of 4 KB.
    zero_length: u64,

    // FileOffset (8 bytes): Specifies the file offset to which zeros MUST be written. The
    // value MUST be a multiple of 4 KB.
    file_offset: u64,

    // SequenceNumber (8 bytes): MUST match the SequenceNumber field of the log entry's header.
    seq_number: u64,
}
impl ZeroDesc {
    pub(crate) const SIGN: &'static [u8] = &[0x7A, 0x65, 0x72, 0x67];
    const CRC: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);
}

impl Crc32 for Descriptor {
    fn crc32(&self) -> u32 {
        let mut digest = Descriptor::CRC.digest();
        self.crc32_from_digest(&mut digest);
        digest.finalize()
    }

    fn crc32_from_digest(&self, digest: &mut crc::Digest<u32>) {
        match self {
            Descriptor::Zero(z) => z.crc32_from_digest(digest),
            Descriptor::Data(d) => d.crc32_from_digest(digest),
        }
    }
}

impl<T> DeSerialise<T> for ZeroDesc {
    type Item = ZeroDesc;

    fn deserialize(reader: &mut T) -> Result<Self::Item, VhdxError>
    where
        T: Read + Seek,
    {
        let mut buffer = [0; 32];
        reader.read_exact(&mut buffer)?;
        let (_, zero_desc) = map(
            tuple((t_sign_u32, le_u32, le_u64, le_u64, le_u64)),
            |(signature, _, zero_length, file_offset, seq_number)| ZeroDesc {
                signature,
                zero_length,
                file_offset,
                seq_number,
            },
        )(&buffer)
        .finish()?;
        Ok(zero_desc)
    }
}

impl std::fmt::Debug for ZeroDesc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Descriptor")
            .field("signature", &self.signature)
            .field("file_offset", &self.file_offset)
            .field("seq_number", &self.seq_number)
            .finish()
    }
}

impl Crc32 for ZeroDesc {
    fn crc32(&self) -> u32 {
        let mut digest = ZeroDesc::CRC.digest();
        self.crc32_from_digest(&mut digest);
        digest.finalize()
    }

    fn crc32_from_digest(&self, digest: &mut crc::Digest<u32>) {
        digest.update(ZeroDesc::SIGN);
        digest.update(&[0; 4]);
        digest.update(&self.zero_length.to_le_bytes());
        digest.update(&self.file_offset.to_le_bytes());
        digest.update(&self.seq_number.to_le_bytes());
    }
}

#[derive(Clone)]
pub(crate) struct DataDesc {
    signature: Signature,

    // TrailingBytes (4 bytes): Contains the four trailing bytes that were removed from the
    // update when it was converted to a data sector. These trailing bytes MUST be restored
    // before the data sector is written to its final location on disk.
    trailing_bytes: Vec<u8>,

    // LeadingBytes (8 bytes): Contains the first eight bytes that were removed from the update
    // when it was converted to a data sector. These leading bytes MUST be restored before the
    // data sector is written to its final location on disk.
    leading_bytes: Vec<u8>,

    // FileOffset (8 bytes): Specifies the file offset to which the data described by this
    // descriptor MUST be written. The value MUST be a multiple of 4 KB.
    file_offset: u64,

    // SequenceNumber (8 bytes): MUST match the SequenceNumber field of the entry's header.
    seq_number: u64,

    // Data sector belonging to this descriptor
    data_sector: Option<DataSector>,
}

impl DataDesc {
    pub(crate) const SIGN: &'static [u8] = &[0x64, 0x65, 0x73, 0x63];
    const CRC: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);
}

impl<T> DeSerialise<T> for DataDesc {
    type Item = DataDesc;

    fn deserialize(reader: &mut T) -> Result<Self::Item, VhdxError>
    where
        T: Read + Seek,
    {
        let mut buffer = [0; 32];
        reader.read_exact(&mut buffer)?;
        let (_, data_desc) = map(
            tuple((t_sign_u32, take(4usize), take(8usize), le_u64, le_u64)),
            |(signature, trailing_bytes, leading_bytes, file_offset, seq_number)| DataDesc {
                signature,
                trailing_bytes: trailing_bytes.to_vec(),
                leading_bytes: leading_bytes.to_vec(),
                file_offset,
                seq_number,
                data_sector: None,
            },
        )(&buffer)
        .finish()?;
        Ok(data_desc)
    }
}

impl Crc32 for DataDesc {
    fn crc32(&self) -> u32 {
        let mut digest = DataDesc::CRC.digest();
        self.crc32_from_digest(&mut digest);
        digest.finalize()
    }

    fn crc32_from_digest(&self, digest: &mut crc::Digest<u32>) {
        digest.update(DataDesc::SIGN);
        digest.update(&self.trailing_bytes);
        digest.update(&self.leading_bytes);
        digest.update(&self.file_offset.to_le_bytes());
        digest.update(&self.seq_number.to_le_bytes());
    }
}

impl std::fmt::Debug for DataDesc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Data")
            .field("signature", &self.signature)
            .field("file_offset", &self.file_offset)
            .field("seq_number", &self.seq_number)
            .field("data_sector", &self.data_sector)
            .finish()
    }
}

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct DataSector {
    // DataSignature (4 bytes): MUST be 0x61746164 ("data" as ASCII).
    signature: Signature,

    // SequenceHigh (4 bytes): MUST
    // contain the four most significant bytes of the SequenceNumber field of the associated entry.
    seq_high: u32,

    // Data (4084 bytes): Contains the raw data associated with the update, bytes 8 through 4,091,
    // inclusive. Bytes 0 through 7 and 4,092 through 4,096 are stored in the data descriptor, in
    // the LeadingBytes and TrailingBytes fields, respectively.
    data: Vec<u8>,
    //
    // SequenceLow (4 bytes): MUST contain
    // the four least significant bytes of the SequenceNumber field of the associated entry.
    seq_low: u32,
}

impl DataSector {
    pub(crate) const SIGN: &'static [u8] = &[0x64, 0x61, 0x74, 0x61];
    const CRC: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

    fn new(signature: Signature, seq_high: u32, data: &[u8], seq_low: u32) -> Self {
        Self {
            signature,
            seq_high,
            data: data.to_vec(),
            seq_low,
        }
    }

    fn sequence_number(&self) -> u64 {
        ((self.seq_high as u64) << 32) | self.seq_low as u64
    }
}

impl<T> DeSerialise<T> for DataSector {
    type Item = DataSector;

    fn deserialize(reader: &mut T) -> Result<Self::Item, VhdxError>
    where
        T: Read + Seek,
    {
        let mut buffer = [0; 4096];
        reader.read_exact(&mut buffer)?;
        let (_, data_sector) = map(
            tuple((t_sign_u32, le_u32, take(4084usize), le_u32)),
            |(signature, sequence_high, data, sequence_low)| {
                DataSector::new(signature, sequence_high, data, sequence_low)
            },
        )(&buffer)?;

        Ok(data_sector)
    }
}

impl Crc32 for DataSector {
    fn crc32(&self) -> u32 {
        let mut digest = DataSector::CRC.digest();
        self.crc32_from_digest(&mut digest);
        digest.finalize()
    }

    fn crc32_from_digest(&self, digest: &mut crc::Digest<u32>) {
        digest.update(DataSector::SIGN);
        digest.update(&self.seq_high.to_le_bytes());
        digest.update(&self.data);
        digest.update(&self.seq_low.to_le_bytes());
    }
}

impl std::fmt::Debug for DataSector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataSector")
            .field("signature", &self.signature)
            .field("sequence_number", &self.sequence_number())
            .finish()
    }
}

#[derive(Debug)]
pub struct LogSequence {
    pub sequence_number: u64,
    pub entries: Vec<LogEntry>,
    pub head_value: u64,
    pub tail_value: u64,
}
impl LogSequence {
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.head()
            .map(|v| {
                self.tail_value <= v.header.tail as u64 && self.head_value >= v.header.tail as u64
            })
            .unwrap_or(false)
    }

    fn head(&self) -> Option<&LogEntry> {
        self.entries.last()
    }
}

#[cfg(test)]
mod tests {

    use std::io::Cursor;

    use super::*;

    #[test]
    fn should_deserialize_entry_header() {
        // FTI
        let bytes = vec![
            0x6c, 0x6f, 0x67, 0x65, 0xbc, 0x30, 0xfd, 0xe9, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x8d, 0xec, 0x92, 0x41, 0x0f, 0x51, 0x28, 0x36, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x02, 0x0a, 0x46, 0xdd, 0xb4, 0x1d, 0x13, 0x4d, 0xad, 0x70,
            0xdc, 0x30, 0x93, 0xaf, 0xd5, 0xc2, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];

        let mut bytes = Cursor::new(bytes);

        // 2 header sections
        let entry_header = LogHeader::deserialize(&mut bytes).unwrap();

        assert_eq!(Signature::Loge, entry_header.signature);
    }
}

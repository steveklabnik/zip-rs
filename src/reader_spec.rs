use std::io;
use std::io::{IoResult, IoError};
use compression;
use types::ZipFile;
use spec;
use util;

pub fn central_header_to_zip_file<R: Reader+Seek>(reader: &mut R) -> IoResult<ZipFile>
{
    // Parse central header
    let signature = try!(reader.read_le_u32());
    if signature != spec::CENTRAL_DIRECTORY_HEADER_SIGNATURE
    {
        return Err(IoError {
            kind: io::MismatchedFileTypeForOperation,
            desc: "Invalid central directory header",
            detail: None })
    }

    try!(reader.read_le_u16());
    try!(reader.read_le_u16());
    let flags = try!(reader.read_le_u16());
    let encrypted = flags & 1 == 1;
    let is_utf8 = flags & (1 << 11) != 0;
    let compression_method = try!(reader.read_le_u16());
    let last_mod_time = try!(reader.read_le_u16());
    let last_mod_date = try!(reader.read_le_u16());
    let crc32 = try!(reader.read_le_u32());
    let compressed_size = try!(reader.read_le_u32());
    let uncompressed_size = try!(reader.read_le_u32());
    let file_name_length = try!(reader.read_le_u16()) as uint;
    let extra_field_length = try!(reader.read_le_u16()) as uint;
    let file_comment_length = try!(reader.read_le_u16()) as uint;
    try!(reader.read_le_u16());
    try!(reader.read_le_u16());
    try!(reader.read_le_u32());
    let offset = try!(reader.read_le_u32()) as i64;
    let file_name_raw = try!(reader.read_exact(file_name_length));
    let extra_field = try!(reader.read_exact(extra_field_length));
    let file_comment_raw  = try!(reader.read_exact(file_comment_length));

    let file_name = match is_utf8
    {
        true => String::from_utf8_lossy(file_name_raw.as_slice()).into_string(),
        false => ::cp437::to_string(file_name_raw.as_slice()),
    };
    let file_comment = match is_utf8
    {
        true => String::from_utf8_lossy(file_comment_raw.as_slice()).into_string(),
        false => ::cp437::to_string(file_comment_raw.as_slice()),
    };

    // Remember end of central header
    let return_position = try!(reader.tell()) as i64;

    // Parse local header
    try!(reader.seek(offset, io::SeekSet));
    let signature = try!(reader.read_le_u32());
    if signature != spec::LOCAL_FILE_HEADER_SIGNATURE
    {
        return Err(IoError {
            kind: io::MismatchedFileTypeForOperation,
            desc: "Invalid local file header",
            detail: None })
    }

    try!(reader.seek(22, io::SeekCur));
    let file_name_length = try!(reader.read_le_u16()) as u64;
    let extra_field_length = try!(reader.read_le_u16()) as u64;
    let magic_and_header = 4 + 22 + 2 + 2;
    let data_start = offset as u64 + magic_and_header + file_name_length + extra_field_length;

    // Construct the result
    let mut result = ZipFile
    {
        encrypted: encrypted,
        compression_method: FromPrimitive::from_u16(compression_method).unwrap_or(compression::Unknown),
        last_modified_time: util::msdos_datetime_to_tm(last_mod_time, last_mod_date),
        crc32: crc32,
        compressed_size: compressed_size as u64,
        uncompressed_size: uncompressed_size as u64,
        file_name: file_name,
        file_comment: file_comment,
        header_start: offset as u64,
        data_start: data_start,
    };

    try!(parse_extra_field(&mut result, extra_field.as_slice()));

    // Go back after the central header
    try!(reader.seek(return_position, io::SeekSet));

    Ok(result)
}

fn parse_extra_field(_file: &mut ZipFile, data: &[u8]) -> IoResult<()>
{
    let mut reader = io::BufReader::new(data);
    while !reader.eof()
    {
        let kind = try!(reader.read_le_u16());
        let len = try!(reader.read_le_u16());
        debug!("Parsing extra block {:04x}", kind);
        match kind
        {
            _ => try!(reader.seek(len as i64, io::SeekCur)),
        }
    }
    Ok(())
}

use compression;
use types::ZipFile;
use spec;
use writer_spec;
use crc32;
use std::default::Default;
use std::io;
use std::io::{IoResult, IoError};
use std::mem;
use time;
use flate2;
use flate2::FlateWriter;
use flate2::writer::DeflateEncoder;

enum GenericZipWriter<W>
{
    Closed,
    Storer(W),
    Deflater(DeflateEncoder<W>),
}

/// Generator for ZIP files.
///
/// ```
/// fn doit() -> std::io::IoResult<()>
/// {
///     // For this example we write to a buffer, but normally you should use a File
///     let mut buf = [0u8, ..65536];
///     let w = std::io::BufWriter::new(&mut buf);
///     let mut zip = zip::ZipWriter::new(w);
///
///     try!(zip.start_file("hello_world.txt", zip::compression::Stored));
///     try!(zip.write(b"Hello, World!"));
///
///     // Optionally finish the zip. (this is also done on drop)
///     try!(zip.finish());
///
///     Ok(())
/// }
///
/// println!("Result: {}", doit());
/// ```
pub struct ZipWriter<W>
{
    inner: GenericZipWriter<W>,
    files: Vec<ZipFile>,
    stats: ZipWriterStats,
}

#[deriving(Default)]
struct ZipWriterStats
{
    crc32: u32,
    start: u64,
    bytes_written: u64,
}

fn writer_closed_error<T>() -> IoResult<T>
{
    Err(IoError { kind: io::Closed, desc: "This writer has been closed", detail: None })
}

impl<W: Writer+Seek> Writer for ZipWriter<W>
{
    fn write(&mut self, buf: &[u8]) -> IoResult<()>
    {
        if self.files.len() == 0 { return Err(IoError { kind: io::OtherIoError, desc: "No file has been started", detail: None, }) }
        self.stats.update(buf);
        match self.inner
        {
            Storer(ref mut w) => w.write(buf),
            Deflater(ref mut w) => w.write(buf),
            Closed => writer_closed_error(),
        }
    }
}

impl ZipWriterStats
{
    fn update(&mut self, buf: &[u8])
    {
        self.crc32 = crc32::update(self.crc32, buf);
        self.bytes_written += buf.len() as u64;
    }
}

impl<W: Writer+Seek> ZipWriter<W>
{
    /// Initializes the ZipWriter.
    ///
    /// Before writing to this object, the start_file command should be called.
    pub fn new(inner: W) -> ZipWriter<W>
    {
        ZipWriter
        {
            inner: Storer(inner),
            files: Vec::new(),
            stats: Default::default(),
        }
    }

    /// Start a new file for with the requested compression method.
    pub fn start_file(&mut self, name: &str, compression: compression::CompressionMethod) -> IoResult<()>
    {
        try!(self.finish_file());

        {
            let writer = self.inner.get_plain();
            let header_start = try!(writer.tell());

            let mut file = ZipFile
            {
                encrypted: false,
                compression_method: compression,
                last_modified_time: time::now(),
                crc32: 0,
                compressed_size: 0,
                uncompressed_size: 0,
                file_name: String::from_str(name),
                file_comment: String::new(),
                header_start: header_start,
                data_start: 0,
            };
            try!(writer_spec::write_local_file_header(writer, &file));

            let header_end = try!(writer.tell());
            self.stats.start = header_end;
            file.data_start = header_end;

            self.stats.bytes_written = 0;
            self.stats.crc32 = 0;

            self.files.push(file);
        }

        try!(self.inner.switch_to(compression));

        Ok(())
    }

    fn finish_file(&mut self) -> IoResult<()>
    {
        try!(self.inner.switch_to(compression::Stored));
        let writer = self.inner.get_plain();

        let file = match self.files.last_mut()
        {
            None => return Ok(()),
            Some(f) => f,
        };
        file.crc32 = self.stats.crc32;
        file.uncompressed_size = self.stats.bytes_written;
        file.compressed_size = try!(writer.tell()) - self.stats.start;

        try!(writer_spec::update_local_file_header(writer, file));
        try!(writer.seek(0, io::SeekEnd));
        Ok(())
    }

    /// Finish the last file and write all other zip-structures
    ///
    /// This will return the writer, but one should normally not append any data to the end of the file.
    /// Note that the zipfile will also be finished on drop.
    pub fn finish(mut self) -> IoResult<W>
    {
        try!(self.finalize());
        let inner = mem::replace(&mut self.inner, Closed);
        Ok(inner.unwrap())
    }

    fn finalize(&mut self) -> IoResult<()>
    {
        try!(self.finish_file());

        {
            let writer = self.inner.get_plain();

            let central_start = try!(writer.tell());
            for file in self.files.iter()
            {
                try!(writer_spec::write_central_directory_header(writer, file));
            }
            let central_size = try!(writer.tell()) - central_start;

            let footer = spec::CentralDirectoryEnd
            {
                disk_number: 0,
                disk_with_central_directory: 0,
                number_of_files_on_this_disk: self.files.len() as u16,
                number_of_files: self.files.len() as u16,
                central_directory_size: central_size as u32,
                central_directory_offset: central_start as u32,
                zip_file_comment: b"zip-rs".to_vec(),
            };

            try!(footer.write(writer));
        }

        Ok(())
    }
}

#[unsafe_destructor]
impl<W: Writer+Seek> Drop for ZipWriter<W>
{
    fn drop(&mut self)
    {
        if !self.inner.is_closed()
        {
            match self.finalize()
            {
                Ok(_) => {},
                Err(e) => warn!("ZipWriter drop failed: {}", e),
            }
        }
    }
}

impl<W: Writer+Seek> GenericZipWriter<W>
{
    fn switch_to(&mut self, compression: compression::CompressionMethod) -> IoResult<()>
    {
        let bare = match mem::replace(self, Closed)
        {
            Storer(w) => w,
            Deflater(w) => try!(w.finish()),
            Closed => return writer_closed_error(),
        };

        *self = match compression
        {
            compression::Stored => Storer(bare),
            compression::Deflated => Deflater(bare.deflate_encode(flate2::Default)),
            _ => return Err(IoError { kind: io::OtherIoError, desc: "Unsupported compression requested", detail: None }),
        };

        Ok(())
    }

    fn is_closed(&self) -> bool
    {
        match *self
        {
            Closed => true,
            _ => false,
        }
    }

    fn get_plain(&mut self) -> &mut W
    {
        match *self
        {
            Storer(ref mut w) => w,
            _ => panic!("Should have switched to stored beforehand"),
        }
    }

    fn unwrap(self) -> W
    {
        match self
        {
            Storer(w) => w,
            _ => panic!("Should have switched to stored beforehand"),
        }
    }
}

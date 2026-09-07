fn compress_bytes(format: &str, value: &[u8]) -> io::Result<Vec<u8>> {
    use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
    use flate2::Compression;
    let compression = Compression::default();
    match format {
        "brotli" => {
            let mut encoder = brotli::CompressorWriter::new(Vec::new(), 4096, 5, 22);
            encoder.write_all(value)?;
            Ok(encoder.into_inner())
        }
        "gzip" => {
            let mut encoder = GzEncoder::new(Vec::new(), compression);
            encoder.write_all(value)?;
            encoder.finish()
        }
        "deflateRaw" => {
            let mut encoder = DeflateEncoder::new(Vec::new(), compression);
            encoder.write_all(value)?;
            encoder.finish()
        }
        _ => {
            let mut encoder = ZlibEncoder::new(Vec::new(), compression);
            encoder.write_all(value)?;
            encoder.finish()
        }
    }
}

fn decompress_bytes(format: &str, value: &[u8]) -> io::Result<Vec<u8>> {
    use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
    let mut output = Vec::new();
    match format {
        "brotli" => brotli::Decompressor::new(value, 4096).read_to_end(&mut output)?,
        "gzip" => GzDecoder::new(value).read_to_end(&mut output)?,
        "deflateRaw" => DeflateDecoder::new(value).read_to_end(&mut output)?,
        _ => ZlibDecoder::new(value).read_to_end(&mut output)?,
    };
    Ok(output)
}

enum WebZlibWriter {
    GzipEncoder(flate2::write::GzEncoder<Vec<u8>>),
    ZlibEncoder(flate2::write::ZlibEncoder<Vec<u8>>),
    DeflateEncoder(flate2::write::DeflateEncoder<Vec<u8>>),
    GzipDecoder(flate2::write::GzDecoder<Vec<u8>>),
    ZlibDecoder(flate2::write::ZlibDecoder<Vec<u8>>),
    DeflateDecoder(flate2::write::DeflateDecoder<Vec<u8>>),
    BrotliEncoder(Box<brotli::CompressorWriter<Vec<u8>>>),
    BrotliDecoder(Box<brotli::DecompressorWriter<Vec<u8>>>),
}

struct WebZlibStream {
    writer: WebZlibWriter,
    emitted: usize,
}

impl WebZlibStream {
    fn new(operation: &str, format: &str) -> io::Result<Self> {
        use flate2::Compression;
        let writer = match (operation, format) {
            ("compress", "gzip") => WebZlibWriter::GzipEncoder(flate2::write::GzEncoder::new(
                Vec::new(),
                Compression::default(),
            )),
            ("compress", "deflateRaw") => WebZlibWriter::DeflateEncoder(
                flate2::write::DeflateEncoder::new(Vec::new(), Compression::default()),
            ),
            ("compress", "deflate") => WebZlibWriter::ZlibEncoder(flate2::write::ZlibEncoder::new(
                Vec::new(),
                Compression::default(),
            )),
            ("decompress", "gzip") => {
                WebZlibWriter::GzipDecoder(flate2::write::GzDecoder::new(Vec::new()))
            }
            ("decompress", "deflateRaw") => {
                WebZlibWriter::DeflateDecoder(flate2::write::DeflateDecoder::new(Vec::new()))
            }
            ("decompress", "deflate") => {
                WebZlibWriter::ZlibDecoder(flate2::write::ZlibDecoder::new(Vec::new()))
            }
            ("compress", "brotli") => WebZlibWriter::BrotliEncoder(Box::new(
                brotli::CompressorWriter::new(Vec::new(), 4096, 5, 22),
            )),
            ("decompress", "brotli") => WebZlibWriter::BrotliDecoder(Box::new(
                brotli::DecompressorWriter::new(Vec::new(), 4096),
            )),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid zlib stream",
                ))
            }
        };
        Ok(Self { writer, emitted: 0 })
    }

    fn write_and_flush(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
        macro_rules! write {
            ($writer:expr) => {{
                $writer.write_all(input)?;
                $writer.flush()?;
                $writer.get_ref()
            }};
        }
        let output = match &mut self.writer {
            WebZlibWriter::GzipEncoder(writer) => write!(writer),
            WebZlibWriter::ZlibEncoder(writer) => write!(writer),
            WebZlibWriter::DeflateEncoder(writer) => write!(writer),
            WebZlibWriter::GzipDecoder(writer) => write!(writer),
            WebZlibWriter::ZlibDecoder(writer) => write!(writer),
            WebZlibWriter::DeflateDecoder(writer) => write!(writer),
            WebZlibWriter::BrotliEncoder(writer) => write!(writer),
            WebZlibWriter::BrotliDecoder(writer) => write!(writer),
        };
        let chunk = output[self.emitted..].to_vec();
        self.emitted = output.len();
        Ok(chunk)
    }

    fn finish(self, input: &[u8]) -> io::Result<Vec<u8>> {
        macro_rules! finish {
            ($mut_writer:expr) => {{
                let mut writer = $mut_writer;
                writer.write_all(input)?;
                writer.finish()?
            }};
        }
        let output = match self.writer {
            WebZlibWriter::GzipEncoder(writer) => finish!(writer),
            WebZlibWriter::ZlibEncoder(writer) => finish!(writer),
            WebZlibWriter::DeflateEncoder(writer) => finish!(writer),
            WebZlibWriter::GzipDecoder(writer) => finish!(writer),
            WebZlibWriter::ZlibDecoder(writer) => finish!(writer),
            WebZlibWriter::DeflateDecoder(writer) => finish!(writer),
            WebZlibWriter::BrotliEncoder(mut writer) => {
                writer.write_all(input)?;
                writer.flush()?;
                (*writer).into_inner()
            }
            WebZlibWriter::BrotliDecoder(mut writer) => {
                writer.write_all(input)?;
                writer.flush()?;
                (*writer).into_inner().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid Brotli stream")
                })?
            }
        };
        Ok(output[self.emitted..].to_vec())
    }
}

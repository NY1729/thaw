fn compress_bytes(format: &str, value: &[u8]) -> io::Result<Vec<u8>> {
    use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
    use flate2::Compression;
    let compression = Compression::default();
    match format {
        #[cfg(feature = "brotli")]
        "brotli" => {
            let mut encoder = brotli::CompressorWriter::new(Vec::new(), 4096, 5, 22);
            encoder.write_all(value)?;
            Ok(encoder.into_inner())
        }
        #[cfg(not(feature = "brotli"))]
        "brotli" => Err(io::Error::new(io::ErrorKind::Unsupported, "Brotli support is not linked")),
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
    use flate2::read::{DeflateDecoder, MultiGzDecoder, ZlibDecoder};
    let mut output = Vec::new();
    match format {
        #[cfg(feature = "brotli")]
        "brotli" => brotli::Decompressor::new(value, 4096).read_to_end(&mut output)?,
        #[cfg(not(feature = "brotli"))]
        "brotli" => return Err(io::Error::new(io::ErrorKind::Unsupported, "Brotli support is not linked")),
        "gzip" => MultiGzDecoder::new(value).read_to_end(&mut output)?,
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
    ZlibDecoder(Inflate),
    DeflateDecoder(Inflate),
    #[cfg(feature = "brotli")]
    BrotliEncoder(Box<brotli::CompressorWriter<Vec<u8>>>),
    #[cfg(feature = "brotli")]
    BrotliDecoder(Box<brotli::DecompressorWriter<Vec<u8>>>),
}

/// Streaming inflate that remembers whether the end of the compressed stream
/// was reached: flate2's write-side decoders report success at `finish()` for
/// a truncated input, which a decompressor must reject.
struct Inflate {
    decompress: flate2::Decompress,
    sink: Vec<u8>,
    ended: bool,
}

impl Inflate {
    fn new(zlib_header: bool) -> Self {
        Self { decompress: flate2::Decompress::new(zlib_header), sink: Vec::new(), ended: false }
    }

    fn get_mut(&mut self) -> &mut Vec<u8> {
        &mut self.sink
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn write_all(&mut self, mut input: &[u8]) -> io::Result<()> {
        use flate2::{FlushDecompress, Status};
        while !self.ended {
            if self.sink.capacity() - self.sink.len() < 4096 {
                self.sink.reserve(32 * 1024);
            }
            let before = self.decompress.total_in();
            let status = self
                .decompress
                .decompress_vec(input, &mut self.sink, FlushDecompress::None)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            input = &input[(self.decompress.total_in() - before) as usize..];
            if status == Status::StreamEnd {
                self.ended = true;
            } else if input.is_empty() && self.sink.len() < self.sink.capacity() {
                break;
            }
        }
        Ok(())
    }

    fn finish(self) -> io::Result<Vec<u8>> {
        if !self.ended {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected end of file"));
        }
        Ok(self.sink)
    }
}

struct WebZlibStream {
    writer: WebZlibWriter,
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
            ("decompress", "deflateRaw") => WebZlibWriter::DeflateDecoder(Inflate::new(false)),
            ("decompress", "deflate") => WebZlibWriter::ZlibDecoder(Inflate::new(true)),
            #[cfg(feature = "brotli")]
            ("compress", "brotli") => WebZlibWriter::BrotliEncoder(Box::new(
                brotli::CompressorWriter::new(Vec::new(), 4096, 5, 22),
            )),
            #[cfg(feature = "brotli")]
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
        Ok(Self { writer })
    }

    fn write_and_flush(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
        macro_rules! write {
            ($writer:expr) => {{
                $writer.write_all(input)?;
                $writer.flush()?;
                std::mem::take($writer.get_mut())
            }};
        }
        Ok(match &mut self.writer {
            WebZlibWriter::GzipEncoder(writer) => write!(writer),
            WebZlibWriter::ZlibEncoder(writer) => write!(writer),
            WebZlibWriter::DeflateEncoder(writer) => write!(writer),
            WebZlibWriter::GzipDecoder(writer) => write!(writer),
            WebZlibWriter::ZlibDecoder(writer) => write!(writer),
            WebZlibWriter::DeflateDecoder(writer) => write!(writer),
            #[cfg(feature = "brotli")]
            WebZlibWriter::BrotliEncoder(writer) => write!(writer),
            #[cfg(feature = "brotli")]
            WebZlibWriter::BrotliDecoder(writer) => write!(writer),
        })
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
            #[cfg(feature = "brotli")]
            WebZlibWriter::BrotliEncoder(mut writer) => {
                writer.write_all(input)?;
                writer.flush()?;
                (*writer).into_inner()
            }
            #[cfg(feature = "brotli")]
            WebZlibWriter::BrotliDecoder(mut writer) => {
                writer.write_all(input)?;
                writer.flush()?;
                (*writer).into_inner().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid Brotli stream")
                })?
            }
        };
        Ok(output)
    }
}

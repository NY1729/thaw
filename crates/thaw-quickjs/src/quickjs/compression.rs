/// Options accepted by the zlib built-in (`level`, `dictionary`, `maxOutputLength`), decoded from the JSON the JS side passes.
#[derive(Default, Clone)]
struct ZlibOptions {
    level: Option<u32>,
    dictionary: Vec<u8>,
    max_output: Option<usize>,
}

impl ZlibOptions {
    fn parse(json: &str) -> Self {
        let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(json) else {
            return Self::default();
        };
        Self {
            level: map.get("level").and_then(|value| value.as_i64()).filter(|level| *level >= 0).map(|level| level as u32),
            dictionary: map.get("dictionary").and_then(|value| value.as_str()).map(hex_decode).unwrap_or_default(),
            max_output: map.get("maxOutputLength").and_then(|value| value.as_u64()).map(|limit| limit as usize),
        }
    }

    fn compression(&self) -> flate2::Compression {
        self.level.map(flate2::Compression::new).unwrap_or_default()
    }
}

type ZlibJobResult = Result<Vec<u8>, String>;

thread_local! {
    static ZLIB_JOBS: RefCell<HashMap<u32, Receiver<ZlibJobResult>>> = RefCell::new(HashMap::new());
    static ZLIB_DONE: RefCell<HashMap<u32, Vec<u8>>> = RefCell::new(HashMap::new());
    static ZLIB_NEXT_JOB: Cell<u32> = const { Cell::new(1) };
}

/// Starts a one-shot (de)compression on its own thread and returns its handle; the result is collected by
/// `zlib_async_poll`, so a large buffer never holds the event loop. The input is read straight from the
/// typed array (no text encoding of the payload).
fn zlib_async_start<'js>(
    operation: String,
    format: String,
    value: rquickjs::TypedArray<'js, u8>,
    options: String,
) -> u32 {
    let input = value.as_bytes().map(<[u8]>::to_vec).unwrap_or_default();
    let options = ZlibOptions::parse(&options);
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = if operation == "compress" {
            compress_bytes_with(&format, &input, &options)
        } else {
            decompress_bytes_with(&format, &input, &options)
        };
        let _ = sender.send(result.map_err(|error| error.to_string()));
    });
    let handle = ZLIB_NEXT_JOB.with(|next| {
        let handle = next.get();
        next.set(handle.wrapping_add(1).max(1));
        handle
    });
    ZLIB_JOBS.with(|jobs| jobs.borrow_mut().insert(handle, receiver));
    handle
}

/// The finished jobs as `[{handle, ok, message?}]` JSON; the bytes of a successful job are taken with
/// `zlib_async_take`.
fn zlib_async_poll() -> String {
    let mut finished = Vec::new();
    ZLIB_JOBS.with(|jobs| {
        jobs.borrow_mut().retain(|handle, receiver| match receiver.try_recv() {
            Ok(Ok(bytes)) => {
                ZLIB_DONE.with(|done| done.borrow_mut().insert(*handle, bytes));
                finished.push(serde_json::json!({ "handle": handle, "ok": true }));
                false
            }
            Ok(Err(message)) => {
                finished.push(serde_json::json!({ "handle": handle, "ok": false, "message": message }));
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => {
                finished.push(serde_json::json!({ "handle": handle, "ok": false, "message": "zlib worker stopped" }));
                false
            }
        });
    });
    serde_json::Value::Array(finished).to_string()
}

fn zlib_async_take<'js>(ctx: Ctx<'js>, handle: u32) -> rquickjs::Result<rquickjs::ArrayBuffer<'js>> {
    let bytes = ZLIB_DONE.with(|done| done.borrow_mut().remove(&handle)).unwrap_or_default();
    rquickjs::ArrayBuffer::new(ctx, bytes)
}

/// A pending job keeps the event loop alive.
fn zlib_async_active() -> bool {
    ZLIB_JOBS.with(|jobs| !jobs.borrow().is_empty()) || ZLIB_DONE.with(|done| !done.borrow().is_empty())
}

fn zlib_error(code: &str, message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{code}:{message}"))
}

#[cfg(test)]
fn compress_bytes(format: &str, value: &[u8]) -> io::Result<Vec<u8>> {
    compress_bytes_with(format, value, &ZlibOptions::default())
}

fn compress_bytes_with(format: &str, value: &[u8], options: &ZlibOptions) -> io::Result<Vec<u8>> {
    use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
    let compression = options.compression();
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
        "deflateRaw" | "deflate" if !options.dictionary.is_empty() => {
            let mut compress = flate2::Compress::new(compression, format == "deflate");
            compress.set_dictionary(&options.dictionary).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
            let mut output = Vec::with_capacity(value.len() / 2 + 64);
            loop {
                let consumed = compress.total_in() as usize;
                let status = compress
                    .compress_vec(&value[consumed..], &mut output, flate2::FlushCompress::Finish)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                if status == flate2::Status::StreamEnd {
                    return Ok(output);
                }
                output.reserve(32 * 1024);
            }
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

#[cfg(test)]
fn decompress_bytes(format: &str, value: &[u8]) -> io::Result<Vec<u8>> {
    decompress_bytes_with(format, value, &ZlibOptions::default())
}

fn too_large(limit: usize) -> io::Error {
    zlib_error("ERR_BUFFER_TOO_LARGE", &format!("Cannot create a Buffer larger than {limit} bytes"))
}

fn decompress_bytes_with(format: &str, value: &[u8], options: &ZlibOptions) -> io::Result<Vec<u8>> {
    use flate2::read::{DeflateDecoder, MultiGzDecoder, ZlibDecoder};
    let mut output = Vec::new();
    // Read one byte past the limit so an exact fit succeeds and an overrun is detected.
    let limit = options.max_output.map(|limit| limit as u64 + 1).unwrap_or(u64::MAX);
    match format {
        #[cfg(feature = "brotli")]
        "brotli" => {
            brotli::Decompressor::new(value, 4096).take(limit).read_to_end(&mut output)?;
        }
        #[cfg(not(feature = "brotli"))]
        "brotli" => return Err(io::Error::new(io::ErrorKind::Unsupported, "Brotli support is not linked")),
        "gzip" => {
            MultiGzDecoder::new(value).take(limit).read_to_end(&mut output)?;
        }
        "deflateRaw" | "deflate" if !options.dictionary.is_empty() || format == "deflate" => {
            output = inflate_with_dictionary(format == "deflate", value, options)?;
        }
        "deflateRaw" => {
            DeflateDecoder::new(value).take(limit).read_to_end(&mut output)?;
        }
        _ => {
            ZlibDecoder::new(value).take(limit).read_to_end(&mut output)?;
        }
    };
    if let Some(max) = options.max_output {
        if output.len() > max {
            return Err(too_large(max));
        }
    }
    Ok(output)
}

/// One-shot inflate with zlib's preset-dictionary handshake: a stream that asks for a dictionary the caller did not
/// supply fails with `Z_NEED_DICT`, exactly like Node.
fn inflate_with_dictionary(zlib_header: bool, value: &[u8], options: &ZlibOptions) -> io::Result<Vec<u8>> {
    let mut decompress = flate2::Decompress::new(zlib_header);
    if !zlib_header && !options.dictionary.is_empty() {
        decompress.set_dictionary(&options.dictionary).map_err(|_| zlib_error("Z_NEED_DICT", "Bad dictionary"))?;
    }
    let mut output = Vec::new();
    let mut supplied = false;
    loop {
        let consumed = decompress.total_in() as usize;
        if let Some(max) = options.max_output {
            if output.len() > max {
                return Err(too_large(max));
            }
        }
        output.reserve(32 * 1024);
        match decompress.decompress_vec(&value[consumed..], &mut output, flate2::FlushDecompress::Finish) {
            Ok(flate2::Status::StreamEnd) => return Ok(output),
            Ok(_) if consumed == decompress.total_in() as usize && output.len() < output.capacity() => {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected end of file"));
            }
            Ok(_) => {}
            Err(error) if error.needs_dictionary().is_some() => {
                if options.dictionary.is_empty() {
                    return Err(zlib_error("Z_NEED_DICT", "Missing dictionary"));
                }
                if supplied {
                    return Err(zlib_error("Z_NEED_DICT", "Bad dictionary"));
                }
                supplied = true;
                decompress.set_dictionary(&options.dictionary).map_err(|_| zlib_error("Z_NEED_DICT", "Bad dictionary"))?;
            }
            Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidData, error)),
        }
    }
}

enum WebZlibWriter {
    GzipEncoder(flate2::write::GzEncoder<Vec<u8>>),
    ZlibEncoder(flate2::write::ZlibEncoder<Vec<u8>>),
    DeflateEncoder(Deflate),
    GzipDecoder(flate2::write::GzDecoder<Vec<u8>>),
    ZlibDecoder(Inflate),
    DeflateDecoder(Inflate),
    #[cfg(feature = "brotli")]
    BrotliEncoder(Box<brotli::CompressorWriter<Vec<u8>>>),
    #[cfg(feature = "brotli")]
    BrotliDecoder(Box<brotli::DecompressorWriter<Vec<u8>>>),
}

/// Raw-deflate streaming encoder on top of `flate2::Compress` (flate2's write-side `DeflateEncoder` cannot take a
/// preset dictionary).
struct Deflate {
    compress: flate2::Compress,
    sink: Vec<u8>,
}

impl Deflate {
    fn new(compress: flate2::Compress) -> Self {
        Self { compress, sink: Vec::new() }
    }

    fn get_mut(&mut self) -> &mut Vec<u8> {
        &mut self.sink
    }

    fn run(&mut self, mut input: &[u8], flush: flate2::FlushCompress) -> io::Result<()> {
        loop {
            self.sink.reserve(32 * 1024);
            let before = self.compress.total_in();
            let status = self
                .compress
                .compress_vec(input, &mut self.sink, flush)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            input = &input[(self.compress.total_in() - before) as usize..];
            let done = match flush {
                flate2::FlushCompress::Finish => status == flate2::Status::StreamEnd,
                _ => input.is_empty() && self.sink.len() < self.sink.capacity(),
            };
            if done {
                return Ok(());
            }
        }
    }

    fn write_all(&mut self, input: &[u8]) -> io::Result<()> {
        self.run(input, flate2::FlushCompress::None)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.run(&[], flate2::FlushCompress::Sync)
    }

    fn finish(mut self) -> io::Result<Vec<u8>> {
        self.run(&[], flate2::FlushCompress::Finish)?;
        Ok(self.sink)
    }
}

/// Streaming inflate that remembers whether the end of the compressed stream
/// was reached: flate2's write-side decoders report success at `finish()` for
/// a truncated input, which a decompressor must reject.
struct Inflate {
    decompress: flate2::Decompress,
    sink: Vec<u8>,
    ended: bool,
    dictionary: Vec<u8>,
    dictionary_supplied: bool,
}

impl Inflate {
    fn new(zlib_header: bool, dictionary: Vec<u8>) -> io::Result<Self> {
        let mut decompress = flate2::Decompress::new(zlib_header);
        if !zlib_header && !dictionary.is_empty() {
            decompress.set_dictionary(&dictionary).map_err(|_| zlib_error("Z_NEED_DICT", "Bad dictionary"))?;
        }
        Ok(Self { decompress, sink: Vec::new(), ended: false, dictionary, dictionary_supplied: false })
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
            let status = match self.decompress.decompress_vec(input, &mut self.sink, FlushDecompress::None) {
                Ok(status) => status,
                Err(error) if error.needs_dictionary().is_some() => {
                    if self.dictionary.is_empty() {
                        return Err(zlib_error("Z_NEED_DICT", "Missing dictionary"));
                    }
                    if self.dictionary_supplied {
                        return Err(zlib_error("Z_NEED_DICT", "Bad dictionary"));
                    }
                    self.dictionary_supplied = true;
                    self.decompress.set_dictionary(&self.dictionary).map_err(|_| zlib_error("Z_NEED_DICT", "Bad dictionary"))?;
                    input = &input[(self.decompress.total_in() - before) as usize..];
                    continue;
                }
                Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidData, error)),
            };
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
    fn new(operation: &str, format: &str, options: &ZlibOptions) -> io::Result<Self> {
        let compression = options.compression();
        let encoder_state = |zlib_header: bool| -> io::Result<flate2::Compress> {
            let mut compress = flate2::Compress::new(compression, zlib_header);
            if !options.dictionary.is_empty() {
                compress.set_dictionary(&options.dictionary).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
            }
            Ok(compress)
        };
        let writer = match (operation, format) {
            ("compress", "gzip") => WebZlibWriter::GzipEncoder(flate2::write::GzEncoder::new(Vec::new(), compression)),
            ("compress", "deflateRaw") => WebZlibWriter::DeflateEncoder(Deflate::new(encoder_state(false)?)),
            ("compress", "deflate") => WebZlibWriter::ZlibEncoder(flate2::write::ZlibEncoder::new_with_compress(
                Vec::new(),
                encoder_state(true)?,
            )),
            ("decompress", "gzip") => {
                WebZlibWriter::GzipDecoder(flate2::write::GzDecoder::new(Vec::new()))
            }
            ("decompress", "deflateRaw") => WebZlibWriter::DeflateDecoder(Inflate::new(false, options.dictionary.clone())?),
            ("decompress", "deflate") => WebZlibWriter::ZlibDecoder(Inflate::new(true, options.dictionary.clone())?),
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

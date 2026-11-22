//! `node:zlib` zstd one-shot (de)compression over C libzstd 1.5.7 (zstd-safe), the library version Node 22 links,
//! so the frames match Node byte for byte. Built only with the `zstd` cargo feature (the JS side feature-detects it).

use std::io;

use zstd_safe::zstd_sys::ZSTD_EndDirective;
use zstd_safe::{CCtx, CParameter, DCtx, DParameter, InBuffer, OutBuffer, Strategy};

use super::zlib_error;

/// Maps a `ZSTD_c_*` key to the zstd-safe parameter; `None` for keys libzstd's compressor does not have.
fn compress_param(key: u32, value: i32) -> Option<CParameter> {
    let flag = value != 0;
    let unsigned = value as u32;
    Some(match key {
        100 => CParameter::CompressionLevel(value),
        101 => CParameter::WindowLog(unsigned),
        102 => CParameter::HashLog(unsigned),
        103 => CParameter::ChainLog(unsigned),
        104 => CParameter::SearchLog(unsigned),
        105 => CParameter::MinMatch(unsigned),
        106 => CParameter::TargetLength(unsigned),
        107 => CParameter::Strategy(match value {
            1 => Strategy::ZSTD_fast,
            2 => Strategy::ZSTD_dfast,
            3 => Strategy::ZSTD_greedy,
            4 => Strategy::ZSTD_lazy,
            5 => Strategy::ZSTD_lazy2,
            6 => Strategy::ZSTD_btlazy2,
            7 => Strategy::ZSTD_btopt,
            8 => Strategy::ZSTD_btultra,
            9 => Strategy::ZSTD_btultra2,
            _ => return None,
        }),
        160 => CParameter::EnableLongDistanceMatching(flag),
        161 => CParameter::LdmHashLog(unsigned),
        162 => CParameter::LdmMinMatch(unsigned),
        163 => CParameter::LdmBucketSizeLog(unsigned),
        164 => CParameter::LdmHashRateLog(unsigned),
        200 => CParameter::ContentSizeFlag(flag),
        201 => CParameter::ChecksumFlag(flag),
        202 => CParameter::DictIdFlag(flag),
        400 => CParameter::NbWorkers(unsigned),
        401 => CParameter::JobSize(unsigned),
        402 => CParameter::OverlapSizeLog(unsigned),
        _ => return None,
    })
}

/// The libzstd error as Node's `code`/`errno`/`message` triple, carried in the message and split by the JS side
/// (`ZSTD_ERRNO_<errno>:<message>`). libzstd errors are `(size_t)-errno`.
fn libzstd_error(code: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "ZSTD_ERRNO_{}:{}",
            code.wrapping_neg(),
            zstd_safe::get_error_name(code)
        ),
    )
}

fn invalid_param(key: u32) -> io::Error {
    zlib_error(
        "ERR_ZSTD_INVALID_PARAM",
        &format!("{key} is not a valid zstd parameter"),
    )
}

fn setting_failed() -> io::Error {
    zlib_error("ERR_ZLIB_INITIALIZATION_FAILED", "Setting parameter failed")
}

fn set_compress_params(cctx: &mut CCtx<'_>, params: &[(u32, i32)]) -> io::Result<()> {
    for &(key, value) in params {
        let param = compress_param(key, value).ok_or_else(|| invalid_param(key))?;
        cctx.set_parameter(param).map_err(|_| setting_failed())?;
    }
    Ok(())
}

/// `ZSTD_e_continue` / `ZSTD_e_flush` / `ZSTD_e_end` as the JS side numbers them (`zlib.constants`).
pub(super) fn directive(value: u32) -> io::Result<ZSTD_EndDirective> {
    Ok(match value {
        0 => ZSTD_EndDirective::ZSTD_e_continue,
        1 => ZSTD_EndDirective::ZSTD_e_flush,
        2 => ZSTD_EndDirective::ZSTD_e_end,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid zstd directive",
            ))
        }
    })
}

/// Compresses `input` in one call (`ZSTD_compress2`, so the frame header carries the content size, as Node's sync
/// API does). `streamed` takes the streaming path that Node's callback API uses instead: the header then has no
/// content size.
pub(super) fn compress(input: &[u8], params: &[(u32, i32)], streamed: bool) -> io::Result<Vec<u8>> {
    if streamed {
        // Node's streaming API writes the input with `ZSTD_e_continue` and ends the frame with `ZSTD_e_end`; libzstd
        // leaves the content size out of the header on that path only.
        let mut compressor = Compressor::new(params)?;
        let mut output = compressor.write(input, ZSTD_EndDirective::ZSTD_e_continue)?;
        output.extend(compressor.write(&[], ZSTD_EndDirective::ZSTD_e_end)?);
        return Ok(output);
    }
    let mut cctx = CCtx::create();
    set_compress_params(&mut cctx, params)?;
    let mut output = Vec::with_capacity(zstd_safe::compress_bound(input.len()));
    cctx.compress2(&mut output, input).map_err(libzstd_error)?;
    Ok(output)
}

/// A compressor kept across writes (`node:zlib` ZstdCompress). Every write runs `ZSTD_compressStream2` to completion
/// for its directive and appends the bytes libzstd produced.
pub(super) struct Compressor {
    cctx: CCtx<'static>,
    scratch: Vec<u8>,
}

impl Compressor {
    pub(super) fn new(params: &[(u32, i32)]) -> io::Result<Self> {
        let mut cctx = CCtx::create();
        set_compress_params(&mut cctx, params)?;
        Ok(Self {
            cctx,
            scratch: vec![0u8; CCtx::out_size()],
        })
    }

    /// Feeds `input` with `directive` until libzstd has taken all of it (`ZSTD_e_continue`) or has nothing left to
    /// emit for a flush or end.
    pub(super) fn write(
        &mut self,
        input: &[u8],
        directive: ZSTD_EndDirective,
    ) -> io::Result<Vec<u8>> {
        let continues = matches!(directive, ZSTD_EndDirective::ZSTD_e_continue);
        let mut output = Vec::new();
        let mut source = InBuffer::around(input);
        loop {
            let (remaining, produced) = {
                let mut sink = OutBuffer::around(&mut self.scratch[..]);
                let remaining = self
                    .cctx
                    .compress_stream2(&mut sink, &mut source, directive)
                    .map_err(libzstd_error)?;
                (remaining, sink.pos())
            };
            output.extend_from_slice(&self.scratch[..produced]);
            if source.pos() == input.len() && (continues || remaining == 0) {
                return Ok(output);
            }
        }
    }
}

/// Decompresses every frame of `input` (`ZSTD_decompressStream`, which moves on to the next frame by itself).
/// Like Node: truncated or empty input is not an error, and once a frame has decoded, whatever follows it is ignored
/// (trailing garbage, a corrupt or truncated next frame); only an error before the first frame ends is reported.
pub(super) fn decompress(
    input: &[u8],
    params: &[(u32, i32)],
    max_output: Option<usize>,
) -> io::Result<Vec<u8>> {
    let mut decoder = Decompressor::new(params)?;
    let mut output = Vec::new();
    let mut source = InBuffer::around(input);
    let mut frame_done = false;
    loop {
        let (result, produced) = {
            let mut sink = OutBuffer::around(&mut decoder.scratch[..]);
            let result = decoder.dctx.decompress_stream(&mut sink, &mut source);
            (result, sink.pos())
        };
        if let Err(code) = result {
            if frame_done {
                return Ok(output);
            }
            return Err(libzstd_error(code));
        }
        frame_done |= result == Ok(0);
        output.extend_from_slice(&decoder.scratch[..produced]);
        if let Some(max) = max_output {
            if output.len() > max {
                return Err(zlib_error(
                    "ERR_BUFFER_TOO_LARGE",
                    &format!("Cannot create a Buffer larger than {max} bytes"),
                ));
            }
        }
        if source.pos() == input.len() && produced < decoder.scratch.len() {
            return Ok(output);
        }
    }
}

/// A decompressor kept across writes (`node:zlib` ZstdDecompress). Output is whatever the input completed so far.
pub(super) struct Decompressor {
    dctx: DCtx<'static>,
    scratch: Vec<u8>,
}

impl Decompressor {
    pub(super) fn new(params: &[(u32, i32)]) -> io::Result<Self> {
        let mut dctx = DCtx::create();
        for &(key, value) in params {
            if key != 100 {
                return Err(invalid_param(key));
            }
            dctx.set_parameter(DParameter::WindowLogMax(value as u32))
                .map_err(|_| setting_failed())?;
        }
        Ok(Self {
            dctx,
            scratch: vec![0u8; DCtx::out_size()],
        })
    }

    pub(super) fn write(&mut self, input: &[u8]) -> io::Result<Vec<u8>> {
        let mut output = Vec::new();
        let mut source = InBuffer::around(input);
        loop {
            let (result, produced) = {
                let mut sink = OutBuffer::around(&mut self.scratch[..]);
                let result = self.dctx.decompress_stream(&mut sink, &mut source);
                (result, sink.pos())
            };
            result.map_err(libzstd_error)?;
            output.extend_from_slice(&self.scratch[..produced]);
            if source.pos() == input.len() && produced < self.scratch.len() {
                return Ok(output);
            }
        }
    }
}

pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
        "_stream_readable" => Some("module.exports = require('node:stream').Readable;\n"),
        "_stream_writable" => Some("module.exports = require('node:stream').Writable;\n"),
        "_stream_duplex" => Some("module.exports = require('node:stream').Duplex;\n"),
        "_stream_transform" => Some("module.exports = require('node:stream').Transform;\n"),
        "_stream_passthrough" => Some("module.exports = require('node:stream').PassThrough;\n"),
        "_stream_wrap" => Some("module.exports = require('node:stream').Duplex;\n"),
        "stream/promises" => Some(
            "var callbackStream = require('node:stream'); function finished(stream, options) { return new Promise(function(resolve, reject) { callbackStream.finished(stream, Object.assign({}, options || {}, { cleanup: true }), function(error) { if (error) reject(error); else resolve(); }); }); }\
            function pipeline() { var stages = Array.prototype.slice.call(arguments); return new Promise(function(resolve, reject) { stages.push(function(error, value) { if (error) reject(error); else resolve(value); }); callbackStream.pipeline.apply(callbackStream, stages); }); }\
             module.exports = { pipeline: pipeline, finished: finished }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "stream/consumers" => Some(
            "function collect(stream) { if (stream && typeof stream[Symbol.asyncIterator] === 'function') return (async function() { var chunks = []; for await (var chunk of stream) chunks.push(Buffer.from(chunk)); return Buffer.concat(chunks); })(); return new Promise(function(resolve, reject) { var chunks = []; function cleanup() { if (stream.off) { stream.off('data', data); stream.off('end', end); stream.off('error', reject); } } function data(chunk) { chunks.push(Buffer.from(chunk)); } function end() { cleanup(); resolve(Buffer.concat(chunks)); } if (!stream || typeof stream.on !== 'function') { reject(new TypeError('stream must be readable')); return; } stream.on('data', data); stream.once('end', end); stream.once('error', reject); }); } function buffer(stream) { return collect(stream); } function text(stream) { return collect(stream).then(function(value) { return value.toString('utf8'); }); } function json(stream) { return text(stream).then(JSON.parse); } function arrayBuffer(stream) { return collect(stream).then(function(value) { var copy = Uint8Array.from(value); return copy.buffer; }); } function blob(stream) { return collect(stream).then(function(value) { return new Blob([value]); }); } module.exports = { arrayBuffer: arrayBuffer, blob: blob, buffer: buffer, json: json, text: text }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "stream/web" => Some(
            "module.exports = { ReadableStream: globalThis.ReadableStream, ReadableStreamDefaultReader: globalThis.ReadableStreamDefaultReader, ReadableStreamDefaultController: globalThis.ReadableStreamDefaultController, ReadableByteStreamController: globalThis.ReadableByteStreamController, ReadableStreamBYOBReader: globalThis.ReadableStreamBYOBReader, ReadableStreamBYOBRequest: globalThis.ReadableStreamBYOBRequest, WritableStream: globalThis.WritableStream, WritableStreamDefaultWriter: globalThis.WritableStreamDefaultWriter, WritableStreamDefaultController: globalThis.WritableStreamDefaultController, TransformStream: globalThis.TransformStream, TransformStreamDefaultController: globalThis.TransformStreamDefaultController, ByteLengthQueuingStrategy: globalThis.ByteLengthQueuingStrategy, CountQueuingStrategy: globalThis.CountQueuingStrategy, TextEncoderStream: globalThis.TextEncoderStream, TextDecoderStream: globalThis.TextDecoderStream, CompressionStream: globalThis.CompressionStream, DecompressionStream: globalThis.DecompressionStream }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        _ => None,
    }
}

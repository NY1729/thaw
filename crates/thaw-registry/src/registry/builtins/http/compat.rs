pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
        "https" => Some(
            "var http = require('node:http'), tls = require('node:tls'); module.exports = http.__createSecureModule(tls); module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "_http_agent" => Some(
            "var http = require('node:http'); module.exports = { Agent: http.Agent, globalAgent: http.globalAgent }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "_http_client" => Some(
            "var http = require('node:http'); module.exports = { ClientRequest: http.ClientRequest }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "_http_incoming" => Some(
            "var http = require('node:http'); module.exports = { IncomingMessage: http.IncomingMessage, readStart: function(socket) { if (socket && socket.resume) socket.resume(); }, readStop: function(socket) { if (socket && socket.pause) socket.pause(); } }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "_http_outgoing" => Some(
            "var http = require('node:http'), OutgoingMessage = http.ServerResponse; module.exports = { OutgoingMessage: OutgoingMessage, validateHeaderName: http.validateHeaderName, validateHeaderValue: http.validateHeaderValue }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "_http_server" => Some(
            "var http = require('node:http'); module.exports = { Server: http.Server, ServerResponse: http.ServerResponse, setupConnectionsTracking: function() {}, storeHTTPOptions: function(options) { return options || {}; }, httpServerPreClose: function() {} }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "_http_common" => Some(
            r#"var http = require('node:http');
             function HTTPParser(type) { this.type = type; this._headers = []; this._url = ''; }
             HTTPParser.REQUEST = 1; HTTPParser.RESPONSE = 2;
             HTTPParser.prototype.initialize = function(type) { this.type = type; return this; }; HTTPParser.prototype.close = function() {}; HTTPParser.prototype.free = function() {}; HTTPParser.prototype.remove = function() {};
             var parsers = { list: [], alloc: function() { return this.list.pop() || new HTTPParser(); }, free: function(parser) { parser._headers = []; parser._url = ''; this.list.push(parser); } };
             function freeParser(parser) { if (parser && parser.free) parser.free(); }
             function isToken(value) { return /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(String(value)); }
             function invalidHeaderChar(value) { return /[^\t\x20-\x7e\x80-\xff]/.test(String(value)); }
             module.exports = { HTTPParser: HTTPParser, parsers: parsers, freeParser: freeParser, methods: http.METHODS, allMethods: http.METHODS, _checkIsHttpToken: isToken, _checkInvalidHeaderChar: invalidHeaderChar, kLenientNone: 0, kLenientHeaders: 1, kLenientChunkedLength: 2, kLenientKeepAlive: 4, kLenientTransferEncoding: 8, kLenientVersion: 16, kLenientDataAfterClose: 32, kLenientOptionalLFAfterCR: 64, kLenientOptionalCRLFAfterChunk: 128, kLenientOptionalCRBeforeLF: 256, kLenientSpacesAfterChunkSize: 512, kLenientAll: 1023 }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "_tls_common" => Some(
            "var tls = require('node:tls'); module.exports = { SecureContext: tls.SecureContext, createSecureContext: tls.createSecureContext, translatePeerCertificate: function(certificate) { return certificate; } }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "_tls_wrap" => Some("module.exports = require('node:tls');\n"),
        _ => None,
    }
}

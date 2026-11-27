/// `res.end(b)` / `res.write(b)` on a native `http.ServerResponse` must route a byte-buffer chunk to
/// `__thaw_endBytes` / `__thaw_writeBytes`, for every shape of the receiver (`any`-annotated callback parameter,
/// contextual, unannotated) and of the chunk (annotated local/parameter, subarray, call result,
/// unannotated `new Uint8Array`). `createServer` is declared locally with the `ServerResponse` shape
/// because thaw-hir unit tests cannot resolve `node:http`; the end-to-end `node:http` form is in
/// thaw-cli (`node_http_bytes_chunks_of_every_shape_reach_the_wire`).
fn http_server_program(handler_body: &str) -> String {
    format!(
        r#"interface ServerResponse {{ end: (chunk: string) => boolean; write: (chunk: string) => boolean; __thaw_endEncoded: (content: string, encoding: string) => boolean; __thaw_writeBytes: (chunk: Uint8Array) => boolean; __thaw_endBytes: (chunk: Uint8Array) => boolean; }}
declare function createServer(callback: (request: any, response: ServerResponse) => void): void;
function makeBytes(): Buffer {{ return Buffer.from([200, 1, 255]); }}
function main(): void {{
  createServer((req: any, res: any) => {{ {handler_body} }});
}}"#
    )
}

#[test]
fn server_response_bytes_chunks_route_to_bytes_methods_for_every_shape() {
    let cases = [
        ("annotated Uint8Array local, end", "__thaw_endBytes", "const b: Uint8Array = new Uint8Array([200, 1, 255]); res.end(b);"),
        ("annotated Buffer local, write", "__thaw_writeBytes", "const b: Buffer = Buffer.from([200, 1, 255]); res.write(b);"),
        ("unannotated new Uint8Array, end", "__thaw_endBytes", "const b = new Uint8Array([200, 1, 255]); res.end(b);"),
        ("annotated Buffer subarray, write", "__thaw_writeBytes", "const g: Buffer = Buffer.from([200, 1, 255]); res.write(g.subarray(0, 2));"),
        ("call result typed Buffer, end", "__thaw_endBytes", "res.end(makeBytes());"),
    ];
    let mut failures = Vec::new();
    for (name, route, body) in cases {
        let module = thaw_parser::parse_typescript(&http_server_program(body)).expect("parse error");
        match lower_module(&module) {
            Ok(program) => {
                if !format!("{program:?}").contains(&format!("\"{route}\"")) {
                    failures.push(format!("{name}: no `{route}` route in HIR"));
                }
            }
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    assert!(failures.is_empty(), "bytes chunks did not route:\n{}", failures.join("\n"));
}

#[test]
fn server_response_bytes_parameter_chunk_routes_to_end_bytes() {
    let source = http_server_program("send(res, new Uint8Array([200, 1, 255]));").replace(
        "function main(): void {",
        "function send(res: ServerResponse, body: Uint8Array): void { res.end(body); }\nfunction main(): void {",
    );
    let module = thaw_parser::parse_typescript(&source).expect("parse error");
    let program = lower_module(&module).expect("a Uint8Array parameter chunk must lower");
    assert!(format!("{program:?}").contains("\"__thaw_endBytes\""), "parameter chunk did not route to __thaw_endBytes");
}

#[test]
fn server_response_bytes_chunk_with_an_encoding_is_a_compile_error() {
    let module = thaw_parser::parse_typescript(&http_server_program(
        "const b: Buffer = Buffer.from([200, 1, 255]); res.write(b, \"hex\");",
    ))
    .expect("parse error");
    let error = lower_module(&module)
        .expect_err("a Buffer chunk with an encoding argument must not fall through to the string write");
    assert!(error.contains("`write`") && error.contains("Buffer"), "{error}");
}

/// `res.writeHead(code, headers)` takes any headers value (variable, call result) and chains; the native
/// `__thaw_writeHead` closure receives status, message and headers.
#[test]
fn write_head_accepts_non_literal_headers_and_chains() {
    let source = r#"interface ServerResponse { statusCode: number; setHeader: (name: string, value: any) => boolean; end: (chunk: string) => boolean; write: (chunk: string) => boolean; __thaw_writeHead: (status: number, message: string, headers: any) => boolean; }
declare function createServer(callback: (request: any, response: ServerResponse) => void): void;
function main(): void {
  createServer((req: any, res) => {
    const extra: any = { "x-a": "1" };
    res.writeHead(201, extra).end("a");
    res.writeHead(200, "OK", JSON.parse("{}")).end("b");
    res.writeHead(204);
  });
}"#;
    let module = thaw_parser::parse_typescript(source).expect("parse error");
    let program = lower_module(&module).expect("writeHead with variable headers lowers");
    assert!(format!("{program:?}").contains("__thaw_writeHead"));
}

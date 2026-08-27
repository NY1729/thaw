const PLATFORM_GLOBALS: &str = concat!(
    include_str!("platform_globals/webassembly.js"),
    include_str!("platform_globals/runtime.js"),
    include_str!("platform_globals/performance.js"),
    include_str!("platform_globals/workers.js"),
    include_str!("platform_globals/text_encoding.js"),
    include_str!("platform_globals/buffer_crypto.js"),
);

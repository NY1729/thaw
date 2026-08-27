const PLATFORM_GLOBALS: &str = concat!(
    include_str!("platform_globals/webassembly.js"),
    include_str!("platform_globals/runtime.js"),
    include_str!("platform_globals/performance.js"),
    include_str!("platform_globals/workers/structured_clone.js"),
    include_str!("platform_globals/workers/url.js"),
    include_str!("platform_globals/workers/blob_events.js"),
    include_str!("platform_globals/workers/fetch_streams.js"),
    include_str!("platform_globals/workers/messaging.js"),
    include_str!("platform_globals/workers/abort_timers.js"),
    include_str!("platform_globals/text_encoding.js"),
    include_str!("platform_globals/buffer_crypto.js"),
);

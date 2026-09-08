enum JsonTaggedKind {
    Optional,
    Nullable,
    Nullish,
}
include!("json_bridge/encoding.rs");
include!("json_bridge/decoding.rs");

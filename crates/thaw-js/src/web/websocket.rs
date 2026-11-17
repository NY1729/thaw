//! `WebSocket` (`src/web/websocket.js`): an RFC 6455 client over the `__thaw_net_*` natives. Pure JS; the
//! prelude installs the global `WebSocket` and reads only the crypto and net natives the host already declares.

use crate::Interp;

const SOURCE: &str = include_str!("websocket.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run_builtin(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("WebSocket prelude must evaluate"));
}

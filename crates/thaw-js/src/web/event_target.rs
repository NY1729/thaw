//! `Event` / `EventTarget`.
//! `install` evaluates the port once; `AbortSignal` (abort.rs) builds on it.

use crate::Interp;

const SOURCE: &str = include_str!("event_target.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run_builtin(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("Event/EventTarget prelude must evaluate"));
}

#[cfg(test)]
mod tests {
    use crate::Interp;

    #[test]
    fn event_target_constructor_carries_the_nodejs_marker() {
        let mut it = Interp::new();
        super::install(&mut it);
        let value = it
            .run(
                "[EventTarget[Symbol.for('nodejs.event_target')], \
                 Object.getOwnPropertyDescriptor(EventTarget, Symbol.for('nodejs.event_target')).enumerable].join(':')",
            )
            .unwrap();
        assert_eq!(it.to_str(&value).to_string(), "true:true");
    }
}

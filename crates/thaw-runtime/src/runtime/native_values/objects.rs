thread_local! {
    static OBJECT_STATES: RefCell<HashMap<usize, u8>> = RefCell::new(HashMap::new());
}

const NON_EXTENSIBLE: u8 = 1;
const SEALED: u8 = 2;
const FROZEN: u8 = 4;

/// Records `preventExtensions` (1), `seal` (2), or `freeze` (3) for one
/// native object identity. Native layouts are fixed already; this table only
/// carries the observable integrity state across aliases and control flow.
#[no_mangle]
pub extern "C" fn thaw_object_set_state(object: *const u8, operation: u8) -> bool {
    if object.is_null() {
        return false;
    }
    let flags = match operation {
        1 => NON_EXTENSIBLE,
        2 => NON_EXTENSIBLE | SEALED,
        3 => NON_EXTENSIBLE | SEALED | FROZEN,
        _ => return false,
    };
    OBJECT_STATES.with(|states| {
        *states.borrow_mut().entry(object as usize).or_default() |= flags;
    });
    true
}

/// Queries extensible (0), sealed (1), or frozen (2) state.
#[no_mangle]
pub extern "C" fn thaw_object_state(object: *const u8, query: u8) -> bool {
    let flags = OBJECT_STATES.with(|states| {
        states
            .borrow()
            .get(&(object as usize))
            .copied()
            .unwrap_or_default()
    });
    match query {
        0 => flags & NON_EXTENSIBLE == 0,
        1 => flags & SEALED != 0,
        2 => flags & FROZEN != 0,
        _ => false,
    }
}

fn clear_object_states() {
    OBJECT_STATES.with(|states| states.borrow_mut().clear());
}

#[cfg(test)]
mod object_state_tests {
    use super::*;

    #[test]
    fn integrity_state_follows_pointer_identity_and_resets() {
        clear_object_states();
        let object = 0_u8;
        let identity = &object as *const u8;
        assert!(thaw_object_state(identity, 0));
        assert!(thaw_object_set_state(identity, 3));
        assert!(thaw_object_state(identity, 2));
        assert!(thaw_object_state(identity, 1));
        assert!(!thaw_object_state(identity, 0));
        clear_object_states();
        assert!(thaw_object_state(identity, 0));
    }
}

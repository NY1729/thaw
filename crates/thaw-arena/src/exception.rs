//! Runtime-owned payloads for native exceptions crossing a JavaScript frame.
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

/// Exact pending exception channels captured when a native callback publishes an error.
/// Pointer fields remain borrowed from their original owner and are pinned while the
/// packet has an authenticated carrier.
pub struct ExceptionPacket {
    pub id: u64,
    pub pending: usize,
    pub native_text: usize,
    /// Original arena framed-error pointer, when `native_text` was copied from one.
    pub native_error_identity: usize,
    pub object: usize,
    pub native: usize,
    pub aggregate: usize,
    pub tag: i64,
    pub number: u64,
    pub bigint: u64,
    pub boolean: u64,
    pub _roots: Vec<super::ArenaRoot>,
    pub _owned_bytes: Vec<Box<[u8]>>,
    /// Opaque runtime-specific leases (for tracked Box<Value> payloads).
    pub _leases: Vec<Box<dyn Any>>,
}

thread_local! {
    static NEXT_ID: RefCell<u64> = const { RefCell::new(0) };
    static PACKETS: RefCell<HashMap<u64, Weak<ExceptionPacket>>> = RefCell::new(HashMap::new());
}

/// Publishes one packet and returns its checked, never-reused id.
pub fn register_packet(mut packet: ExceptionPacket) -> Option<(u64, Rc<ExceptionPacket>)> {
    let id = NEXT_ID.with(|next| {
        let mut next = next.borrow_mut();
        *next = next.checked_add(1)?;
        Some(*next)
    })?;
    packet.id = id;
    let packet = Rc::new(packet);
    PACKETS.with(|packets| {
        packets.borrow_mut().insert(id, Rc::downgrade(&packet));
    });
    Some((id, packet))
}

impl Drop for ExceptionPacket {
    fn drop(&mut self) {
        for bytes in &self._owned_bytes {
            super::unregister_exception_bytes(bytes.as_ptr().cast());
        }
        let _ = PACKETS.try_with(|packets| {
            let mut packets = packets.borrow_mut();
            if packets
                .get(&self.id)
                .is_some_and(|entry| entry.strong_count() == 0)
            {
                packets.remove(&self.id);
            }
        });
    }
}

pub fn packet(id: u64) -> Option<Rc<ExceptionPacket>> {
    if id == 0 {
        return None;
    }
    PACKETS.with(|packets| packets.borrow().get(&id).and_then(Weak::upgrade))
}

pub fn is_live(id: u64) -> bool {
    packet(id).is_some()
}

/// Returns a field only while some authenticated owner still keeps the packet alive.
pub fn field(id: u64, index: u32) -> Option<u64> {
    let packet = packet(id)?;
    Some(match index {
        0 => packet.tag as u64,
        1 => packet.number,
        2 => packet.bigint,
        3 => packet.boolean,
        4 => packet.object as u64,
        5 => packet.native as u64,
        6 => packet.pending as u64,
        7 => packet.native_text as u64,
        8 => packet.aggregate as u64,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> ExceptionPacket {
        ExceptionPacket {
            id: 0,
            pending: 11,
            native_text: 12,
            native_error_identity: 0,
            object: 13,
            native: 14,
            aggregate: 15,
            tag: 4,
            number: f64::NAN.to_bits(),
            bigint: (-7i64) as u64,
            boolean: 1,
            _roots: Vec::new(),
            _owned_bytes: Vec::new(),
            _leases: Vec::new(),
        }
    }

    #[test]
    fn packet_is_bound_to_exact_owned_pointer_until_its_last_owner_drops() {
        let pointer = crate::owned_string(b"same bytes");
        let lookalike = crate::owned_string(b"same bytes");
        let (id, packet) = register_packet(sample()).unwrap();
        assert!(crate::bind_owned_exception(pointer, packet.clone()));
        assert!(!crate::bind_owned_exception(pointer, packet.clone()));
        assert!(crate::owned_exception(lookalike).is_none());
        assert_eq!(field(id, 0), Some(4));
        assert_eq!(field(id, 1), Some(f64::NAN.to_bits()));
        assert_eq!(field(id, 2), Some((-7i64) as u64));
        assert_eq!(field(id, 8), Some(15));
        let moved = crate::take_owned_exception(pointer).unwrap();
        assert!(is_live(id));
        unsafe { crate::destroy_string(pointer) };
        unsafe { crate::destroy_string(lookalike) };
        drop(packet);
        assert!(is_live(id));
        drop(moved);
        assert!(!is_live(id));
        assert_eq!(field(id, 0), None);
    }
}

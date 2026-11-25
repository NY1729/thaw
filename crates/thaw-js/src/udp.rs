//! `node:dgram` host (UDP): the `__thaw_udp_*` natives the registry's dgram shim calls. Same wire protocol as the retired-engine
//! host (the retired engine's networking.rs): `ok|...` / `pending` / `err|CODE|message`, payloads as hex.
//!
//! Liveness is not handled here. The shim's receive pump is a timer that re-arms itself while the socket is open, so an
//! open socket keeps the event loop alive exactly as long as that timer is ref'd; `socket.unref()` unrefs the timer.

use std::collections::HashMap;
use std::io;
use std::net::UdpSocket;
use std::sync::Mutex;

use crate::net::{Table, With};
use crate::zlib::hex_encode;

static UDP_SOCKETS: Table<(u32, HashMap<u32, UdpSocket>)> =
    Table::new(|| Mutex::new((1, HashMap::new())));

const BAD_HANDLE: &str = "err|EBADF|socket is closed";
const BAD_PORT: &str = "err|EINVAL|invalid UDP port";

/// The libuv/Node error name for an I/O error.
fn udp_code(error: &io::Error) -> &'static str {
    let code = match error.kind() {
        io::ErrorKind::AddrInUse => "EADDRINUSE",
        io::ErrorKind::AddrNotAvailable => "EADDRNOTAVAIL",
        io::ErrorKind::PermissionDenied => "EACCES",
        io::ErrorKind::ConnectionRefused => "ECONNREFUSED",
        io::ErrorKind::InvalidInput => "EINVAL",
        io::ErrorKind::NotFound => "ENOENT",
        _ => "EUNKNOWN",
    };
    #[cfg(unix)]
    let code = match error.raw_os_error() {
        Some(libc::ENETUNREACH) => "ENETUNREACH",
        Some(libc::EHOSTUNREACH) => "EHOSTUNREACH",
        Some(libc::EAFNOSUPPORT) => "EAFNOSUPPORT",
        Some(libc::EBADF) => "EBADF",
        _ => code,
    };
    code
}

fn udp_error(error: io::Error) -> String {
    format!("err|{}|{error}", udp_code(&error))
}

/// Ports arrive as 32-bit numbers; `0` is allowed for bind only (an ephemeral port).
fn udp_port(port: u32) -> Option<u16> {
    u16::try_from(port).ok()
}

pub(crate) fn bind(host: &str, port: u32) -> String {
    let Some(port) = udp_port(port) else {
        return BAD_PORT.to_string();
    };
    match UdpSocket::bind((host, port)) {
        Ok(socket) => {
            let address = match socket.local_addr() {
                Ok(address) => address,
                Err(error) => return udp_error(error),
            };
            if let Err(error) = socket.set_nonblocking(true) {
                return udp_error(error);
            }
            UDP_SOCKETS.with(|sockets| {
                let mut sockets = sockets.borrow_mut();
                let handle = sockets.0;
                sockets.0 = sockets.0.wrapping_add(1).max(1);
                sockets.1.insert(handle, socket);
                format!("ok|{handle}|{}|{}", address.ip(), address.port())
            })
        }
        // Node's message names the call and the address: `bind EADDRINUSE 127.0.0.1:4000` (no `:port` for port 0).
        Err(error) => {
            let code = udp_code(&error);
            let suffix = if port == 0 {
                String::new()
            } else {
                format!(":{port}")
            };
            format!("err|{code}|bind {code} {host}{suffix}")
        }
    }
}

pub(crate) fn send(handle: u32, value: &[u8], host: &str, port: u32) -> String {
    let Some(port) = udp_port(port).filter(|port| *port != 0) else {
        return BAD_PORT.to_string();
    };
    UDP_SOCKETS.with(|sockets| {
        let sockets = sockets.borrow();
        let Some(socket) = sockets.1.get(&handle) else {
            return BAD_HANDLE.to_string();
        };
        let sent = if socket.peer_addr().is_ok() {
            socket.send(value)
        } else {
            socket.send_to(value, (host, port))
        };
        sent.map(|written| format!("ok|{written}"))
            .unwrap_or_else(udp_error)
    })
}

/// One datagram without blocking: `ok|<hex>|<ip>|<port>|<length>`, `pending` when none is queued.
pub(crate) fn receive(handle: u32) -> String {
    UDP_SOCKETS.with(|sockets| {
        let sockets = sockets.borrow();
        let Some(socket) = sockets.1.get(&handle) else {
            return BAD_HANDLE.to_string();
        };
        // ponytail: a 64 KiB buffer per poll (the retired-engine host does the same); reuse one per socket if polls show up in profiles.
        let mut value = vec![0u8; 65_536];
        match socket.recv_from(&mut value) {
            Ok((length, peer)) => {
                value.truncate(length);
                format!(
                    "ok|{}|{}|{}|{}",
                    hex_encode(&value),
                    peer.ip(),
                    peer.port(),
                    length
                )
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => "pending".to_string(),
            Err(error) => udp_error(error),
        }
    })
}

pub(crate) fn connect(handle: u32, host: &str, port: u32) -> String {
    let Some(port) = udp_port(port).filter(|port| *port != 0) else {
        return BAD_PORT.to_string();
    };
    UDP_SOCKETS.with(|sockets| {
        let sockets = sockets.borrow();
        let Some(socket) = sockets.1.get(&handle) else {
            return BAD_HANDLE.to_string();
        };
        socket
            .connect((host, port))
            .map(|()| "ok".to_string())
            .unwrap_or_else(udp_error)
    })
}

pub(crate) fn set_broadcast(handle: u32, enabled: bool) -> String {
    UDP_SOCKETS.with(|sockets| {
        let sockets = sockets.borrow();
        let Some(socket) = sockets.1.get(&handle) else {
            return BAD_HANDLE.to_string();
        };
        socket
            .set_broadcast(enabled)
            .map(|()| "ok".to_string())
            .unwrap_or_else(udp_error)
    })
}

/// UDP socket options Node exposes beyond broadcast: TTLs, multicast loopback / interface / membership.
/// `name` selects the option; `a`, `b`, `c` are its textual arguments (group, interface, source address or a number).
#[cfg(unix)]
pub(crate) fn option(handle: u32, name: &str, a: &str, b: &str, c: &str) -> String {
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::os::fd::AsRawFd;
    UDP_SOCKETS.with(|sockets| {
        let sockets = sockets.borrow();
        let Some(socket) = sockets.1.get(&handle) else {
            return BAD_HANDLE.to_string();
        };
        let fd = socket.as_raw_fd();
        let v6 = socket
            .local_addr()
            .map(|address| address.is_ipv6())
            .unwrap_or(false);
        let invalid = |what: &str| format!("err|EINVAL|{what} EINVAL");
        fn set<T>(
            fd: libc::c_int,
            level: libc::c_int,
            option: libc::c_int,
            value: &T,
        ) -> Result<(), String> {
            let result = unsafe {
                libc::setsockopt(
                    fd,
                    level,
                    option,
                    (value as *const T).cast(),
                    std::mem::size_of::<T>() as libc::socklen_t,
                )
            };
            if result == 0 {
                Ok(())
            } else {
                Err(udp_error(io::Error::last_os_error()))
            }
        }
        let number = |text: &str| text.parse::<libc::c_int>().ok();
        let outcome: Result<(), String> = match name {
            "ttl" => match number(a) {
                Some(value) if v6 => set(fd, libc::IPPROTO_IPV6, libc::IPV6_UNICAST_HOPS, &value),
                Some(value) => set(fd, libc::IPPROTO_IP, libc::IP_TTL, &value),
                None => return invalid("setTTL"),
            },
            "multicast_ttl" => match number(a) {
                Some(value) if v6 => set(fd, libc::IPPROTO_IPV6, libc::IPV6_MULTICAST_HOPS, &value),
                Some(value) => set(fd, libc::IPPROTO_IP, libc::IP_MULTICAST_TTL, &value),
                None => return invalid("setMulticastTTL"),
            },
            "multicast_loop" => {
                let value = libc::c_int::from(a == "1");
                if v6 {
                    set(fd, libc::IPPROTO_IPV6, libc::IPV6_MULTICAST_LOOP, &value)
                } else {
                    set(fd, libc::IPPROTO_IP, libc::IP_MULTICAST_LOOP, &value)
                }
            }
            "multicast_if" => {
                if v6 {
                    let index = number(a).map(|value| value as libc::c_uint).unwrap_or(0);
                    set(fd, libc::IPPROTO_IPV6, libc::IPV6_MULTICAST_IF, &index)
                } else {
                    match a.parse::<Ipv4Addr>() {
                        Ok(address) => set(
                            fd,
                            libc::IPPROTO_IP,
                            libc::IP_MULTICAST_IF,
                            &libc::in_addr {
                                s_addr: u32::from_ne_bytes(address.octets()),
                            },
                        ),
                        Err(_) => return invalid("setMulticastInterface"),
                    }
                }
            }
            "add_membership" | "drop_membership" => {
                let add = name == "add_membership";
                let call = if add {
                    "addMembership"
                } else {
                    "dropMembership"
                };
                if v6 {
                    let Ok(group) = a.parse::<Ipv6Addr>() else {
                        return invalid(call);
                    };
                    let request = libc::ipv6_mreq {
                        ipv6mr_multiaddr: libc::in6_addr {
                            s6_addr: group.octets(),
                        },
                        ipv6mr_interface: number(b).map(|value| value as libc::c_uint).unwrap_or(0),
                    };
                    set(
                        fd,
                        libc::IPPROTO_IPV6,
                        if add {
                            libc::IPV6_ADD_MEMBERSHIP
                        } else {
                            libc::IPV6_DROP_MEMBERSHIP
                        },
                        &request,
                    )
                } else {
                    let Ok(group) = a.parse::<Ipv4Addr>() else {
                        return invalid(call);
                    };
                    let interface = if b.is_empty() {
                        Some(Ipv4Addr::UNSPECIFIED)
                    } else {
                        b.parse::<Ipv4Addr>().ok()
                    };
                    let Some(interface) = interface else {
                        return invalid(call);
                    };
                    let request = libc::ip_mreq {
                        imr_multiaddr: libc::in_addr {
                            s_addr: u32::from_ne_bytes(group.octets()),
                        },
                        imr_interface: libc::in_addr {
                            s_addr: u32::from_ne_bytes(interface.octets()),
                        },
                    };
                    set(
                        fd,
                        libc::IPPROTO_IP,
                        if add {
                            libc::IP_ADD_MEMBERSHIP
                        } else {
                            libc::IP_DROP_MEMBERSHIP
                        },
                        &request,
                    )
                }
            }
            #[cfg(any(target_os = "linux", target_os = "android"))]
            "add_source_membership" | "drop_source_membership" => {
                let add = name == "add_source_membership";
                if v6 {
                    return "err|ENOTSUP|source-specific IPv6 membership is not supported"
                        .to_string();
                }
                // a = source, b = group, c = interface
                let (Ok(source), Ok(group)) = (a.parse::<Ipv4Addr>(), b.parse::<Ipv4Addr>()) else {
                    return invalid("addSourceSpecificMembership");
                };
                let interface = if c.is_empty() {
                    Some(Ipv4Addr::UNSPECIFIED)
                } else {
                    c.parse::<Ipv4Addr>().ok()
                };
                let Some(interface) = interface else {
                    return invalid("addSourceSpecificMembership");
                };
                let request = libc::ip_mreq_source {
                    imr_multiaddr: libc::in_addr {
                        s_addr: u32::from_ne_bytes(group.octets()),
                    },
                    imr_interface: libc::in_addr {
                        s_addr: u32::from_ne_bytes(interface.octets()),
                    },
                    imr_sourceaddr: libc::in_addr {
                        s_addr: u32::from_ne_bytes(source.octets()),
                    },
                };
                set(
                    fd,
                    libc::IPPROTO_IP,
                    if add {
                        libc::IP_ADD_SOURCE_MEMBERSHIP
                    } else {
                        libc::IP_DROP_SOURCE_MEMBERSHIP
                    },
                    &request,
                )
            }
            _ => return invalid("socket option"),
        };
        match outcome {
            Ok(()) => "ok".to_string(),
            Err(wire) => wire,
        }
    })
}

#[cfg(not(unix))]
pub(crate) fn option(_: u32, _: &str, _: &str, _: &str, _: &str) -> String {
    "err|ENOTSUP|UDP socket options are not supported on this platform".to_string()
}

/// Dissolves the peer association by connecting to `AF_UNSPEC`.
#[cfg(unix)]
pub(crate) fn disconnect(handle: u32) -> String {
    use std::os::fd::AsRawFd;
    UDP_SOCKETS.with(|sockets| {
        let sockets = sockets.borrow();
        let Some(socket) = sockets.1.get(&handle) else {
            return BAD_HANDLE.to_string();
        };
        let mut address: libc::sockaddr = unsafe { std::mem::zeroed() };
        address.sa_family = libc::AF_UNSPEC as libc::sa_family_t;
        let result = unsafe {
            libc::connect(
                socket.as_raw_fd(),
                &address,
                std::mem::size_of::<libc::sockaddr>() as libc::socklen_t,
            )
        };
        if result == 0 {
            "ok".to_string()
        } else {
            udp_error(io::Error::last_os_error())
        }
    })
}

#[cfg(not(unix))]
pub(crate) fn disconnect(_handle: u32) -> String {
    "err|ENOTSUP|UDP disconnect is unsupported on this platform".to_string()
}

pub(crate) fn close(handle: u32) {
    UDP_SOCKETS.with(|sockets| {
        sockets.borrow_mut().1.remove(&handle);
    });
}

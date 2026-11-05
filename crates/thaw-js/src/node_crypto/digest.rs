//! Pure-Rust digests and HMAC for `node:crypto`: SHA-3 (Keccak), RIPEMD-160 and HMAC over every
//! digest. Copied from thaw-quickjs (lib.rs, quickjs/symmetric_cipher.rs), which stays the oracle
//! until QuickJS is removed; the md5/sha1/sha2 crates do the rest.

use sha2::Digest;

/// Digest by canonical lowercase name (`sha3256`, not `sha3-256`); `None` = unsupported.
pub(crate) fn digest_bytes(algorithm: &str, value: &[u8]) -> Option<Vec<u8>> {
    Some(match algorithm {
        "md5" => md5::Md5::digest(value).to_vec(),
        "sha1" => sha1::Sha1::digest(value).to_vec(),
        "sha224" => sha2::Sha224::digest(value).to_vec(),
        "sha256" => sha2::Sha256::digest(value).to_vec(),
        "sha384" => sha2::Sha384::digest(value).to_vec(),
        "sha512" => sha2::Sha512::digest(value).to_vec(),
        "sha3224" => sha3_digest(value, 28),
        "sha3256" => sha3_digest(value, 32),
        "sha3384" => sha3_digest(value, 48),
        "sha3512" => sha3_digest(value, 64),
        "ripemd160" => ripemd160_digest(value),
        _ => return None,
    })
}

/// Keccak-f[1600] (FIPS 202); `sha3-*` digests only, no extendable output.
fn keccak_f1600(state: &mut [u64; 25]) {
    const ROUND_CONSTANTS: [u64; 24] = [
        0x0000000000000001,
        0x0000000000008082,
        0x800000000000808a,
        0x8000000080008000,
        0x000000000000808b,
        0x0000000080000001,
        0x8000000080008081,
        0x8000000000008009,
        0x000000000000008a,
        0x0000000000000088,
        0x0000000080008009,
        0x000000008000000a,
        0x000000008000808b,
        0x800000000000008b,
        0x8000000000008089,
        0x8000000000008003,
        0x8000000000008002,
        0x8000000000000080,
        0x000000000000800a,
        0x800000008000000a,
        0x8000000080008081,
        0x8000000000008080,
        0x0000000080000001,
        0x8000000080008008,
    ];
    const ROTATIONS: [u32; 24] = [
        1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44,
    ];
    const LANES: [usize; 24] = [
        10, 7, 11, 17, 18, 3, 5, 16, 8, 21, 24, 4, 15, 23, 19, 13, 12, 2, 20, 14, 22, 9, 6, 1,
    ];
    for constant in ROUND_CONSTANTS {
        let mut columns = [0u64; 5];
        for (x, column) in columns.iter_mut().enumerate() {
            *column = state[x] ^ state[x + 5] ^ state[x + 10] ^ state[x + 15] ^ state[x + 20];
        }
        for x in 0..5 {
            let delta = columns[(x + 4) % 5] ^ columns[(x + 1) % 5].rotate_left(1);
            for y in 0..5 {
                state[x + 5 * y] ^= delta;
            }
        }
        let mut carried = state[1];
        for (lane, rotation) in LANES.into_iter().zip(ROTATIONS) {
            let next = state[lane];
            state[lane] = carried.rotate_left(rotation);
            carried = next;
        }
        for y in 0..5 {
            let row = [
                state[5 * y],
                state[5 * y + 1],
                state[5 * y + 2],
                state[5 * y + 3],
                state[5 * y + 4],
            ];
            for x in 0..5 {
                state[5 * y + x] = row[x] ^ (!row[(x + 1) % 5] & row[(x + 2) % 5]);
            }
        }
        state[0] ^= constant;
    }
}

fn sha3_digest(value: &[u8], output_bytes: usize) -> Vec<u8> {
    let rate = 200 - 2 * output_bytes;
    let mut state = [0u64; 25];
    let mut padded = value.to_vec();
    padded.push(0x06);
    padded.resize(padded.len().div_ceil(rate) * rate, 0);
    *padded.last_mut().expect("padded block") |= 0x80;
    for block in padded.chunks(rate) {
        for (lane, bytes) in state.iter_mut().zip(block.as_chunks::<8>().0) {
            *lane ^= u64::from_le_bytes(*bytes);
        }
        keccak_f1600(&mut state);
    }
    state
        .iter()
        .flat_map(|lane| lane.to_le_bytes())
        .take(output_bytes)
        .collect()
}

/// RIPEMD-160 (ISO/IEC 10118-3).
fn ripemd160_digest(value: &[u8]) -> Vec<u8> {
    const RL: [u8; 80] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 7, 4, 13, 1, 10, 6, 15, 3, 12, 0, 9,
        5, 2, 14, 11, 8, 3, 10, 14, 4, 9, 15, 8, 1, 2, 7, 0, 6, 13, 11, 5, 12, 1, 9, 11, 10, 0, 8,
        12, 4, 13, 3, 7, 15, 14, 5, 6, 2, 4, 0, 5, 9, 7, 12, 2, 10, 14, 1, 3, 8, 11, 6, 15, 13,
    ];
    const RR: [u8; 80] = [
        5, 14, 7, 0, 9, 2, 11, 4, 13, 6, 15, 8, 1, 10, 3, 12, 6, 11, 3, 7, 0, 13, 5, 10, 14, 15, 8,
        12, 4, 9, 1, 2, 15, 5, 1, 3, 7, 14, 6, 9, 11, 8, 12, 2, 10, 0, 4, 13, 8, 6, 4, 1, 3, 11,
        15, 0, 5, 12, 2, 13, 9, 7, 10, 14, 12, 15, 10, 4, 1, 5, 8, 7, 6, 2, 13, 14, 0, 3, 9, 11,
    ];
    const SL: [u32; 80] = [
        11, 14, 15, 12, 5, 8, 7, 9, 11, 13, 14, 15, 6, 7, 9, 8, 7, 6, 8, 13, 11, 9, 7, 15, 7, 12,
        15, 9, 11, 7, 13, 12, 11, 13, 6, 7, 14, 9, 13, 15, 14, 8, 13, 6, 5, 12, 7, 5, 11, 12, 14,
        15, 14, 15, 9, 8, 9, 14, 5, 6, 8, 6, 5, 12, 9, 15, 5, 11, 6, 8, 13, 12, 5, 12, 13, 14, 11,
        8, 5, 6,
    ];
    const SR: [u32; 80] = [
        8, 9, 9, 11, 13, 15, 15, 5, 7, 7, 8, 11, 14, 14, 12, 6, 9, 13, 15, 7, 12, 8, 9, 11, 7, 7,
        12, 7, 6, 15, 13, 11, 9, 7, 15, 11, 8, 6, 6, 14, 12, 13, 5, 14, 13, 13, 7, 5, 15, 5, 8, 11,
        14, 14, 6, 14, 6, 9, 12, 9, 12, 5, 15, 8, 8, 5, 12, 9, 12, 5, 14, 6, 8, 13, 6, 5, 15, 13,
        11, 11,
    ];
    const KL: [u32; 5] = [0, 0x5a82_7999, 0x6ed9_eba1, 0x8f1b_bcdc, 0xa953_fd4e];
    const KR: [u32; 5] = [0x50a2_8be6, 0x5c4d_d124, 0x6d70_3ef3, 0x7a6d_76e9, 0];
    fn f(round: usize, x: u32, y: u32, z: u32) -> u32 {
        match round {
            0 => x ^ y ^ z,
            1 => (x & y) | (!x & z),
            2 => (x | !y) ^ z,
            3 => (x & z) | (y & !z),
            _ => x ^ (y | !z),
        }
    }
    let mut state: [u32; 5] = [
        0x6745_2301,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    let mut padded = value.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&(value.len() as u64 * 8).to_le_bytes());
    for block in padded.as_chunks::<64>().0.iter() {
        let mut words = [0u32; 16];
        for (word, bytes) in words.iter_mut().zip(block.as_chunks::<4>().0) {
            *word = u32::from_le_bytes(*bytes);
        }
        let [mut al, mut bl, mut cl, mut dl, mut el] = state;
        let [mut ar, mut br, mut cr, mut dr, mut er] = state;
        for step in 0..80 {
            let group = step / 16;
            let left = al
                .wrapping_add(f(group, bl, cl, dl))
                .wrapping_add(words[RL[step] as usize])
                .wrapping_add(KL[group])
                .rotate_left(SL[step])
                .wrapping_add(el);
            (al, el, dl, cl, bl) = (el, dl, cl.rotate_left(10), bl, left);
            let right = ar
                .wrapping_add(f(4 - group, br, cr, dr))
                .wrapping_add(words[RR[step] as usize])
                .wrapping_add(KR[group])
                .rotate_left(SR[step])
                .wrapping_add(er);
            (ar, er, dr, cr, br) = (er, dr, cr.rotate_left(10), br, right);
        }
        let combined = state[1].wrapping_add(cl).wrapping_add(dr);
        state[1] = state[2].wrapping_add(dl).wrapping_add(er);
        state[2] = state[3].wrapping_add(el).wrapping_add(ar);
        state[3] = state[4].wrapping_add(al).wrapping_add(br);
        state[4] = state[0].wrapping_add(bl).wrapping_add(cr);
        state[0] = combined;
    }
    state.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// HMAC block size: the digest's input block (the SHA-3 sponge rate for `sha3*`).
fn hmac_block_size(algorithm: &str) -> usize {
    match algorithm {
        "sha384" | "sha512" => 128,
        "sha3224" => 144,
        "sha3256" => 136,
        "sha3384" => 104,
        "sha3512" => 72,
        _ => 64,
    }
}

/// HMAC (RFC 2104) over any `digest_bytes` algorithm; `None` = unsupported.
pub(crate) fn hmac_bytes(algorithm: &str, key: &[u8], value: &[u8]) -> Option<Vec<u8>> {
    let block_size = hmac_block_size(algorithm);
    let mut normalized = if key.len() > block_size {
        digest_bytes(algorithm, key)?
    } else {
        key.to_vec()
    };
    normalized.resize(block_size, 0);
    let mut inner: Vec<u8> = normalized.iter().map(|byte| byte ^ 0x36).collect();
    inner.extend_from_slice(value);
    let inner_digest = digest_bytes(algorithm, &inner)?;
    let mut outer: Vec<u8> = normalized.iter().map(|byte| byte ^ 0x5c).collect();
    outer.extend_from_slice(&inner_digest);
    digest_bytes(algorithm, &outer)
}

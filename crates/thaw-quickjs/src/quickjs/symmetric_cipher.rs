// AES-{128,192,256} for `node:crypto` ciphers: CBC/ECB block modes, a stateless CTR keystream
// (used for CTR and for the GCM payload) and the GCM authentication tag. Included into lib.rs.

use aes::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};

fn sym_encrypt_block(key: &[u8], block: &mut [u8; 16]) -> Result<(), String> {
    macro_rules! with {
        ($cipher:ty) => {{
            let cipher =
                <$cipher>::new_from_slice(key).map_err(|_| "Invalid key length".to_string())?;
            cipher.encrypt_block(GenericArray::from_mut_slice(block));
            Ok(())
        }};
    }
    match key.len() {
        16 => with!(aes::Aes128),
        24 => with!(aes::Aes192),
        32 => with!(aes::Aes256),
        _ => Err("Invalid key length".to_string()),
    }
}

fn sym_decrypt_block(key: &[u8], block: &mut [u8; 16]) -> Result<(), String> {
    use aes::cipher::BlockDecrypt;
    macro_rules! with {
        ($cipher:ty) => {{
            let cipher =
                <$cipher>::new_from_slice(key).map_err(|_| "Invalid key length".to_string())?;
            cipher.decrypt_block(GenericArray::from_mut_slice(block));
            Ok(())
        }};
    }
    match key.len() {
        16 => with!(aes::Aes128),
        24 => with!(aes::Aes192),
        32 => with!(aes::Aes256),
        _ => Err("Invalid key length".to_string()),
    }
}

/// CBC/ECB over whole 16-byte blocks; `finalize` adds (encrypt) or strips (decrypt) PKCS#7 padding.
fn sym_block_cipher(
    ecb: bool,
    key: &[u8],
    iv: &[u8],
    value: &[u8],
    decrypt: bool,
    finalize: bool,
    padding: bool,
) -> Result<Vec<u8>, String> {
    let mut chain = [0u8; 16];
    if !ecb {
        if iv.len() != 16 {
            return Err("Invalid initialization vector".to_string());
        }
        chain.copy_from_slice(iv);
    }
    let mut data = value.to_vec();
    if !decrypt && finalize && padding {
        let pad = 16 - data.len() % 16;
        data.extend(std::iter::repeat_n(pad as u8, pad));
    }
    if !data.len().is_multiple_of(16) {
        return Err(if decrypt {
            "wrong final block length".to_string()
        } else {
            "data not multiple of block length".to_string()
        });
    }
    let mut output = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(16) {
        let mut block = [0u8; 16];
        block.copy_from_slice(chunk);
        if decrypt {
            let ciphertext = block;
            sym_decrypt_block(key, &mut block)?;
            if !ecb {
                for (byte, previous) in block.iter_mut().zip(chain) {
                    *byte ^= previous;
                }
                chain = ciphertext;
            }
        } else {
            if !ecb {
                for (byte, previous) in block.iter_mut().zip(chain) {
                    *byte ^= previous;
                }
            }
            sym_encrypt_block(key, &mut block)?;
            chain = block;
        }
        output.extend_from_slice(&block);
    }
    if decrypt && finalize && padding {
        let pad = *output.last().ok_or("bad decrypt")? as usize;
        if pad == 0
            || pad > 16
            || pad > output.len()
            || output[output.len() - pad..]
                .iter()
                .any(|byte| *byte as usize != pad)
        {
            return Err("bad decrypt".to_string());
        }
        output.truncate(output.len() - pad);
    }
    Ok(output)
}

/// XOR `value` with the AES-CTR keystream that starts `offset` bytes after `counter`.
/// `wide` increments the whole 128-bit counter (CTR); otherwise only the low 32 bits (GCM).
fn sym_ctr_xor(
    key: &[u8],
    counter: &[u8],
    offset: usize,
    value: &[u8],
    wide: bool,
) -> Result<Vec<u8>, String> {
    if counter.len() != 16 {
        return Err("Invalid initialization vector".to_string());
    }
    let mut base = [0u8; 16];
    base.copy_from_slice(counter);
    let mut output = Vec::with_capacity(value.len());
    let mut position = offset;
    while output.len() < value.len() {
        let index = (position / 16) as u128;
        let mut block = if wide {
            (u128::from_be_bytes(base).wrapping_add(index)).to_be_bytes()
        } else {
            let mut copy = base;
            let low = u32::from_be_bytes([copy[12], copy[13], copy[14], copy[15]])
                .wrapping_add(index as u32);
            copy[12..].copy_from_slice(&low.to_be_bytes());
            copy
        };
        sym_encrypt_block(key, &mut block)?;
        let skip = position % 16;
        for byte in &block[skip..] {
            if output.len() == value.len() {
                break;
            }
            output.push(value[output.len()] ^ byte);
            position += 1;
        }
    }
    Ok(output)
}

fn sym_ghash_multiply(x: u128, h: u128) -> u128 {
    let mut z = 0u128;
    let mut v = h;
    for bit in 0..128 {
        if x >> (127 - bit) & 1 == 1 {
            z ^= v;
        }
        v = if v & 1 == 1 {
            (v >> 1) ^ (0xe1u128 << 120)
        } else {
            v >> 1
        };
    }
    z
}

fn sym_ghash_update(state: &mut u128, h: u128, data: &[u8]) {
    for chunk in data.chunks(16) {
        let mut block = [0u8; 16];
        block[..chunk.len()].copy_from_slice(chunk);
        *state = sym_ghash_multiply(*state ^ u128::from_be_bytes(block), h);
    }
}

/// The GCM pre-counter block J0 for an IV of any length.
fn sym_gcm_j0(key: &[u8], iv: &[u8]) -> Result<([u8; 16], u128), String> {
    let mut h_block = [0u8; 16];
    sym_encrypt_block(key, &mut h_block)?;
    let h = u128::from_be_bytes(h_block);
    if iv.len() == 12 {
        let mut j0 = [0u8; 16];
        j0[..12].copy_from_slice(iv);
        j0[15] = 1;
        return Ok((j0, h));
    }
    let mut state = 0u128;
    sym_ghash_update(&mut state, h, iv);
    state = sym_ghash_multiply(state ^ ((iv.len() as u128 * 8) & u64::MAX as u128), h);
    Ok((state.to_be_bytes(), h))
}

/// Encrypts/decrypts the GCM payload starting `offset` bytes in.
fn sym_gcm_xor(key: &[u8], iv: &[u8], offset: usize, value: &[u8]) -> Result<Vec<u8>, String> {
    let (mut counter, _) = sym_gcm_j0(key, iv)?;
    let low =
        u32::from_be_bytes([counter[12], counter[13], counter[14], counter[15]]).wrapping_add(1);
    counter[12..].copy_from_slice(&low.to_be_bytes());
    sym_ctr_xor(key, &counter, offset, value, false)
}

fn sym_gcm_tag(key: &[u8], iv: &[u8], aad: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    let (j0, h) = sym_gcm_j0(key, iv)?;
    let mut state = 0u128;
    sym_ghash_update(&mut state, h, aad);
    sym_ghash_update(&mut state, h, ciphertext);
    let lengths = ((aad.len() as u128 * 8) << 64) | (ciphertext.len() as u128 * 8);
    state = sym_ghash_multiply(state ^ lengths, h);
    let mut mask = j0;
    sym_encrypt_block(key, &mut mask)?;
    Ok((state ^ u128::from_be_bytes(mask)).to_be_bytes().to_vec())
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
    for block in padded.chunks_exact(64) {
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

#[cfg(test)]
mod symmetric_cipher_tests {
    use super::*;

    #[test]
    fn ripemd160_matches_the_reference_vectors() {
        assert_eq!(
            hex_encode(&ripemd160_digest(b"abc")),
            "8eb208f7e05d987a9b044a8e98c6b087f15a0bfc"
        );
        assert_eq!(
            hex_encode(&ripemd160_digest(b"")),
            "9c1185a5c5e9fc54612808977ee8f548b2258d31"
        );
    }

    #[test]
    fn gcm_matches_the_nist_test_case_two() {
        let (key, iv, plain) = ([0u8; 16], [0u8; 12], [0u8; 16]);
        let cipher = sym_gcm_xor(&key, &iv, 0, &plain).unwrap();
        assert_eq!(hex_encode(&cipher), "0388dace60b6a392f328c2b971b2fe78");
        assert_eq!(
            hex_encode(&sym_gcm_tag(&key, &iv, &[], &cipher).unwrap()),
            "ab6e47d42cec13bdf53a67b21257bddf"
        );
    }

    #[test]
    fn ctr_keystream_is_stateless_across_offsets() {
        let (key, iv) = ([7u8; 32], [9u8; 16]);
        let whole = sym_ctr_xor(&key, &iv, 0, &[0u8; 40], true).unwrap();
        let tail = sym_ctr_xor(&key, &iv, 5, &[0u8; 35], true).unwrap();
        assert_eq!(&whole[5..], &tail[..]);
    }

    #[test]
    fn cbc_round_trips_with_padding_and_rejects_a_bad_final_block() {
        let (key, iv) = ([1u8; 24], [2u8; 16]);
        let encrypted =
            sym_block_cipher(false, &key, &iv, b"padded text", false, true, true).unwrap();
        assert_eq!(encrypted.len(), 16);
        assert_eq!(
            sym_block_cipher(false, &key, &iv, &encrypted, true, true, true).unwrap(),
            b"padded text"
        );
        let unpadded = sym_block_cipher(false, &key, &iv, &[0u8; 16], false, true, false).unwrap();
        assert_eq!(
            sym_block_cipher(false, &key, &iv, &unpadded, true, true, true),
            Err("bad decrypt".to_string())
        );
    }
}

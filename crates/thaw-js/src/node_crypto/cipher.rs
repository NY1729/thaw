//! AES-{128,192,256} for `node:crypto` ciphers: CBC/ECB block modes, a stateless CTR keystream
//! (CTR, and the GCM payload) and the GCM tag. Copied from thaw-quickjs (quickjs/symmetric_cipher.rs,
//! the oracle until QuickJS is removed); only the RIPEMD-160 part stays in `digest.rs`.

use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};

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
pub(crate) fn sym_block_cipher(
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
    for chunk in data.as_chunks::<16>().0.iter() {
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
pub(crate) fn sym_ctr_xor(
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
pub(crate) fn sym_gcm_xor(
    key: &[u8],
    iv: &[u8],
    offset: usize,
    value: &[u8],
) -> Result<Vec<u8>, String> {
    let (mut counter, _) = sym_gcm_j0(key, iv)?;
    let low =
        u32::from_be_bytes([counter[12], counter[13], counter[14], counter[15]]).wrapping_add(1);
    counter[12..].copy_from_slice(&low.to_be_bytes());
    sym_ctr_xor(key, &counter, offset, value, false)
}

pub(crate) fn sym_gcm_tag(
    key: &[u8],
    iv: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
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

//! SLIP-0010 Ed25519 derivation.
//! Default path `m/44'/501'/0'/0'` is Phantom / Ledger compatible.

use hmac::{Hmac, Mac};
use sha2::Sha512;
use zeroize::Zeroize;

use crate::error::{Error, Result};

type HmacSha512 = Hmac<Sha512>;

pub const DEFAULT_PATH: &str = "m/44'/501'/0'/0'";

pub fn solana_account_path(account: u32) -> String {
    format!("m/44'/501'/{account}'/0'")
}

pub fn account_index(path: &str) -> Option<u32> {
    let parts: Vec<&str> = path
        .trim()
        .trim_start_matches("m/")
        .trim_start_matches("M/")
        .split('/')
        .collect();
    if parts.len() < 3 {
        return None;
    }
    if !parts[0].trim_end_matches('\'').eq("44") {
        return None;
    }
    if !parts[1].trim_end_matches('\'').eq("501") {
        return None;
    }
    parts[2].trim_end_matches('\'').parse().ok()
}

pub fn derive_seed32(seed64: &[u8], path: &str) -> Result<[u8; 32]> {
    if seed64.len() != 64 {
        return Err(Error::msg("BIP39 seed must be 64 bytes"));
    }
    let segments = parse_path(path)?;

    let mut mac = HmacSha512::new_from_slice(b"ed25519 seed")
        .map_err(|e| Error::msg(format!("hmac: {e}")))?;
    mac.update(seed64);
    let mut i = mac.finalize().into_bytes();

    let mut key = [0u8; 32];
    let mut chain = [0u8; 32];
    key.copy_from_slice(&i[..32]);
    chain.copy_from_slice(&i[32..]);

    for segment in segments {
        let index = segment | 0x8000_0000;
        let mut data = [0u8; 37];
        data[0] = 0x00;
        data[1..33].copy_from_slice(&key);
        data[33] = ((index >> 24) & 0xff) as u8;
        data[34] = ((index >> 16) & 0xff) as u8;
        data[35] = ((index >> 8) & 0xff) as u8;
        data[36] = (index & 0xff) as u8;

        let mut mac = HmacSha512::new_from_slice(&chain)
            .map_err(|e| Error::msg(format!("hmac: {e}")))?;
        mac.update(&data);
        i = mac.finalize().into_bytes();
        key.copy_from_slice(&i[..32]);
        chain.copy_from_slice(&i[32..]);
        data.zeroize();
    }

    i.zeroize();
    chain.zeroize();
    Ok(key)
}

pub fn path_indexes(path: &str) -> Result<Vec<u32>> {
    parse_path(path)
}

fn parse_path(path: &str) -> Result<Vec<u32>> {
    let p = path.trim().trim_start_matches("m/").trim_start_matches("M/");
    if p.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for part in p.split('/') {
        let num = part.trim_end_matches('\'').trim();
        let val: u32 = num
            .parse()
            .map_err(|_| Error::msg(format!("bad derivation path segment: {part}")))?;
        out.push(val);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_path_is_32_bytes() {
        let seed = [7u8; 64];
        let k = derive_seed32(&seed, DEFAULT_PATH).unwrap();
        assert_eq!(k.len(), 32);
        let k2 = derive_seed32(&seed, "m/44'/501'/0'/0'").unwrap();
        assert_eq!(k, k2);
        let other = derive_seed32(&seed, "m/44'/501'/1'/0'").unwrap();
        assert_ne!(k, other);
    }
}

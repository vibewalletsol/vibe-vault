//! Solana off-chain message version 1 (sRFC 38).
//!
//! `solana:signMessage` keeps signing the exact bytes a dApp sent.
//! This module is only for `solana:signOffchainMessage`: a fixed prefix,
//! version 1, the signer keys, then the UTF-8 text. The prefix starts with
//! 0xff so the signature cannot be replayed as a transaction.

use crate::error::{Error, Result};

/// `b"\xffsolana offchain"`.
pub const SIGNING_DOMAIN: &[u8; 16] = b"\xffsolana offchain";

/// Ledger Solana app `MAX_OFFCHAIN_MESSAGE_LENGTH` (`PACKET_DATA_SIZE`).
pub const LEDGER_MAX_LEN: usize = (15 * 1024) - 40 - 8;

/// Bytes that will be signed, plus the text the confirm window should show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub bytes: Vec<u8>,
    pub content: String,
}

/// Build a version 1 message. Signers are sorted and must be unique.
pub fn build_v1(signers: &[[u8; 32]], content: &[u8]) -> Result<Vec<u8>> {
    if signers.is_empty() || signers.len() > 255 {
        return Err(Error::msg("Off-chain message needs 1–255 signers"));
    }
    let text =
        std::str::from_utf8(content).map_err(|_| Error::msg("Off-chain message must be UTF-8"))?;
    check_content(text)?;
    let mut keys = signers.to_vec();
    keys.sort();
    for pair in keys.windows(2) {
        if pair[0] == pair[1] {
            return Err(Error::msg("Off-chain signers must be unique"));
        }
    }
    let mut out = Vec::with_capacity(18 + keys.len() * 32 + content.len());
    out.extend_from_slice(SIGNING_DOMAIN);
    out.push(1);
    out.push(keys.len() as u8);
    for key in &keys {
        out.extend_from_slice(key);
    }
    out.extend_from_slice(content);
    Ok(out)
}

/// Turn a dApp payload into the version 1 bytes this vault will sign.
///
/// A payload that already starts with the signing domain is signed as-is
/// after it is checked. Anything else is treated as the UTF-8 text and
/// wrapped with this account as the only signer.
pub fn prepare(signer: &[u8; 32], raw: &[u8]) -> Result<Prepared> {
    if raw.is_empty() {
        return Err(Error::msg("Empty off-chain message"));
    }
    if raw.starts_with(SIGNING_DOMAIN) {
        return accept_prebuilt(signer, raw);
    }
    let content =
        std::str::from_utf8(raw).map_err(|_| Error::msg("Off-chain message must be UTF-8"))?;
    check_content(content)?;
    let bytes = build_v1(&[*signer], content.as_bytes())?;
    Ok(Prepared {
        bytes,
        content: content.to_string(),
    })
}

/// Text to show on the confirm window. Does not check that this vault is a signer.
pub fn display_text(raw: &[u8]) -> Result<String> {
    if raw.is_empty() {
        return Err(Error::msg("Empty off-chain message"));
    }
    if raw.starts_with(SIGNING_DOMAIN) {
        return Ok(split_v1(raw)?.1);
    }
    let content =
        std::str::from_utf8(raw).map_err(|_| Error::msg("Off-chain message must be UTF-8"))?;
    check_content(content)?;
    Ok(content.to_string())
}

pub fn encode_b64(bytes: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
}

fn accept_prebuilt(signer: &[u8; 32], raw: &[u8]) -> Result<Prepared> {
    let (signers, content) = split_v1(raw)?;
    if !signers.iter().any(|key| key == signer) {
        return Err(Error::msg(
            "This vault is not a signer of that off-chain message",
        ));
    }
    Ok(Prepared {
        bytes: raw.to_vec(),
        content,
    })
}

/// Returns the signer keys and the UTF-8 body of a version 1 message.
fn split_v1(raw: &[u8]) -> Result<(Vec<[u8; 32]>, String)> {
    if raw.len() < 18 || &raw[..16] != SIGNING_DOMAIN {
        return Err(Error::msg(
            "Off-chain message header is not valid version 1",
        ));
    }
    if raw[16] != 1 {
        return Err(Error::msg("Only off-chain message version 1 is supported"));
    }
    let count = raw[17] as usize;
    if count == 0 {
        return Err(Error::msg("Off-chain message has no signer"));
    }
    let signers_end = 18usize.saturating_add(count.saturating_mul(32));
    if signers_end > raw.len() || raw.len() == signers_end {
        return Err(Error::msg("Off-chain message has no text"));
    }
    let mut signers = Vec::with_capacity(count);
    let mut prev: Option<&[u8]> = None;
    for i in 0..count {
        let start = 18 + i * 32;
        let key = &raw[start..start + 32];
        if let Some(earlier) = prev {
            if key <= earlier {
                return Err(Error::msg("Off-chain signers must be unique and sorted"));
            }
        }
        prev = Some(key);
        let mut owned = [0u8; 32];
        owned.copy_from_slice(key);
        signers.push(owned);
    }
    let content = std::str::from_utf8(&raw[signers_end..])
        .map_err(|_| Error::msg("Off-chain message must be UTF-8"))?;
    check_content(content)?;
    Ok((signers, content.to_string()))
}

fn check_content(content: &str) -> Result<()> {
    if content.is_empty() {
        return Err(Error::msg("Empty off-chain message"));
    }
    if content.chars().any(|c| {
        c == '\0'
            || (c.is_control() && c != '\n' && c != '\t')
            || matches!(c as u32, 0x202A..=0x202E | 0x2066..=0x2069 | 0x200E | 0x200F | 0x061C)
    }) {
        return Err(Error::msg(
            "Off-chain message contains hidden or control characters",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keypair::Keypair;

    #[test]
    fn v1_layout_and_signature_differ_from_raw() {
        let kp = Keypair::from_seed32(&[9u8; 32]).unwrap();
        let text = b"Hello world";
        let prepared = prepare(&kp.public_bytes(), text).unwrap();
        assert_eq!(&prepared.bytes[..16], SIGNING_DOMAIN);
        assert_eq!(prepared.bytes[0], 0xff);
        assert_eq!(prepared.bytes[16], 1);
        assert_eq!(prepared.bytes[17], 1);
        assert_eq!(&prepared.bytes[18..50], &kp.public_bytes());
        assert_eq!(&prepared.bytes[50..], text);
        assert_eq!(prepared.content, "Hello world");
        let sig = kp.sign(&prepared.bytes);
        assert!(Keypair::verify(&kp.public_bytes(), &prepared.bytes, &sig));
        assert_ne!(kp.sign(text), sig);
    }

    #[test]
    fn signers_are_sorted_and_duplicates_rejected() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let one = build_v1(&[b, a], b"limit").unwrap();
        let two = build_v1(&[a, b], b"limit").unwrap();
        assert_eq!(one, two);
        assert_eq!(one[17], 2);
        assert_eq!(&one[18..50], &a);
        assert_eq!(&one[50..82], &b);
        assert!(build_v1(&[a, a], b"limit").is_err());
    }

    #[test]
    fn prebuilt_is_signed_as_is() {
        let kp = Keypair::from_seed32(&[3u8; 32]).unwrap();
        let other = Keypair::from_seed32(&[4u8; 32]).unwrap();
        let built = build_v1(&[kp.public_bytes()], b"vault sign-in").unwrap();
        let prepared = prepare(&kp.public_bytes(), &built).unwrap();
        assert_eq!(prepared.bytes, built);
        assert_eq!(prepared.content, "vault sign-in");
        assert!(prepare(&other.public_bytes(), &built).is_err());
    }

    #[test]
    fn rejects_empty_control_and_old_version() {
        let kp = Keypair::from_seed32(&[5u8; 32]).unwrap();
        assert!(prepare(&kp.public_bytes(), b"").is_err());
        assert!(prepare(&kp.public_bytes(), b"line\0hidden").is_err());
        assert!(prepare(&kp.public_bytes(), "\u{202E}evil".as_bytes()).is_err());
        let mut old = SIGNING_DOMAIN.to_vec();
        old.push(0);
        old.extend_from_slice(&[0u8; 40]);
        assert!(prepare(&kp.public_bytes(), &old)
            .unwrap_err()
            .to_string()
            .contains("version 1"));
        assert!(display_text("keep\nlines".as_bytes())
            .unwrap()
            .contains('\n'));
    }
}

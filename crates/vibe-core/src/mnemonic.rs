use bip39::{Language, Mnemonic};
use rand::RngCore;
use zeroize::Zeroize;

use crate::error::{Error, Result};

pub fn generate(words: u8) -> Result<String> {
    let ent_len = if words == 24 { 32 } else { 16 };
    let mut entropy = vec![0u8; ent_len];
    rand::rngs::OsRng.fill_bytes(&mut entropy);
    let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)
        .map_err(|e| Error::msg(format!("mnemonic generate: {e}")))?;
    entropy.zeroize();
    Ok(mnemonic.to_string())
}

pub fn validate(phrase: &str) -> bool {
    Mnemonic::parse_in_normalized(Language::English, phrase.trim()).is_ok()
}

/// BIP39 seed (64 bytes). Caller must zeroize.
pub fn to_seed(phrase: &str, passphrase: &str) -> Result<[u8; 64]> {
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, phrase.trim())
        .map_err(|_| Error::InvalidMnemonic)?;
    Ok(mnemonic.to_seed(passphrase))
}

pub fn wipe_string(s: &mut String) {
    s.zeroize();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_validate() {
        let phrase = generate(12).unwrap();
        assert_eq!(phrase.split_whitespace().count(), 12);
        assert!(validate(&phrase));
        assert!(!validate("not a real mnemonic phrase at all"));
        let seed = to_seed(&phrase, "").unwrap();
        assert_eq!(seed.len(), 64);
    }
}

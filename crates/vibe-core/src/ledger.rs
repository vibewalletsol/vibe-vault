//! Ledger Nano Solana app over Linux hidraw.
//! APDU codec is tested without a device. USB I/O is best-effort.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::hd;
use crate::signer::Signer;
use crate::tx;
use crate::wallet::WalletKeyType;

pub const LEDGER_VID: u16 = 0x2c97;
pub const DEFAULT_PATH: &str = "m/44'/501'/0'";
const CHANNEL: u16 = 0x0101;
const TAG_APDU: u8 = 0x05;
const PACKET: usize = 64;
const CLA: u8 = 0xE0;
const INS_GET_PUBKEY: u8 = 0x05;
const INS_SIGN: u8 = 0x06;
const P2_LAST: u8 = 0x00;
const P2_MORE: u8 = 0x02;
const HARDENED: u32 = 0x8000_0000;

#[derive(Clone, Debug)]
pub struct LedgerDevice {
    pub path: String,
    pub hidraw: String,
    pub model: String,
    pub serial: String,
}

pub fn encode_derivation_path(path: &str) -> Result<Vec<u8>> {
    let indexes = hd::path_indexes(path)?;
    if indexes.is_empty() || indexes.len() > 10 {
        return Err(Error::msg("Ledger derivation path must have 1–10 segments"));
    }
    let mut out = Vec::with_capacity(1 + indexes.len() * 4);
    out.push(indexes.len() as u8);
    for idx in indexes {
        out.extend_from_slice(&(idx | HARDENED).to_be_bytes());
    }
    Ok(out)
}

pub fn build_get_pubkey_apdu(path: &str, display: bool) -> Result<Vec<u8>> {
    let deriv = encode_derivation_path(path)?;
    let mut apdu = vec![CLA, INS_GET_PUBKEY, if display { 1 } else { 0 }, 0, deriv.len() as u8];
    apdu.extend_from_slice(&deriv);
    Ok(apdu)
}

pub fn build_sign_apdus(path: &str, message: &[u8]) -> Result<Vec<Vec<u8>>> {
    let deriv = encode_derivation_path(path)?;
    let first_max = 255usize.saturating_sub(1 + deriv.len());
    let mut out = Vec::new();
    if message.len() <= first_max {
        let mut data = Vec::with_capacity(1 + deriv.len() + message.len());
        data.push(0x01);
        data.extend_from_slice(&deriv);
        data.extend_from_slice(message);
        out.push(sign_apdu(&data, true));
        return Ok(out);
    }
    let mut data = Vec::with_capacity(1 + deriv.len() + first_max);
    data.push(0x01);
    data.extend_from_slice(&deriv);
    data.extend_from_slice(&message[..first_max]);
    out.push(sign_apdu(&data, false));
    let mut offset = first_max;
    while offset < message.len() {
        let chunk = (message.len() - offset).min(255);
        let last = offset + chunk >= message.len();
        out.push(sign_apdu(&message[offset..offset + chunk], last));
        offset += chunk;
    }
    Ok(out)
}

fn sign_apdu(data: &[u8], last: bool) -> Vec<u8> {
    let mut apdu = vec![CLA, INS_SIGN, 0x01, if last { P2_LAST } else { P2_MORE }, data.len() as u8];
    apdu.extend_from_slice(data);
    apdu
}

pub fn pack_hid_frames(apdu: &[u8]) -> Vec<[u8; PACKET]> {
    let mut frames = Vec::new();
    let mut seq: u16 = 0;
    let mut offset = 0;
    while offset < apdu.len() || seq == 0 {
        let mut pkt = [0u8; PACKET];
        pkt[0] = (CHANNEL >> 8) as u8;
        pkt[1] = (CHANNEL & 0xff) as u8;
        pkt[2] = TAG_APDU;
        pkt[3] = (seq >> 8) as u8;
        pkt[4] = (seq & 0xff) as u8;
        let mut i = 5;
        if seq == 0 {
            pkt[5] = (apdu.len() >> 8) as u8;
            pkt[6] = (apdu.len() & 0xff) as u8;
            i = 7;
        }
        let take = (PACKET - i).min(apdu.len() - offset);
        pkt[i..i + take].copy_from_slice(&apdu[offset..offset + take]);
        offset += take;
        frames.push(pkt);
        seq += 1;
        if offset >= apdu.len() {
            break;
        }
    }
    frames
}

pub fn unpack_hid_frames(frames: &[[u8; PACKET]]) -> Result<Vec<u8>> {
    if frames.is_empty() {
        return Err(Error::msg("Ledger returned no HID frames"));
    }
    let first = &frames[0];
    if first[2] != TAG_APDU {
        return Err(Error::msg("Ledger HID tag mismatch"));
    }
    let len = u16::from_be_bytes([first[5], first[6]]) as usize;
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&first[7..]);
    for frame in frames.iter().skip(1) {
        if frame[2] != TAG_APDU {
            return Err(Error::msg("Ledger HID continuation tag mismatch"));
        }
        out.extend_from_slice(&frame[5..]);
    }
    out.truncate(len);
    if out.len() < 2 {
        return Err(Error::msg("Ledger APDU truncated"));
    }
    Ok(out)
}

pub fn split_status(mut resp: Vec<u8>) -> Result<Vec<u8>> {
    if resp.len() < 2 {
        return Err(Error::msg("Ledger APDU truncated"));
    }
    let sw = u16::from_be_bytes([resp[resp.len() - 2], resp[resp.len() - 1]]);
    resp.truncate(resp.len() - 2);
    if sw != 0x9000 {
        return Err(Error::msg(status_message(sw)));
    }
    Ok(resp)
}

pub fn status_message(sw: u16) -> String {
    match sw {
        0x6985 => "Rejected on the Ledger".into(),
        0x6A82 => "Open the Solana app on the Ledger".into(),
        0x6E00 => "Ledger CLA not supported — open the Solana app".into(),
        0x5515 => "Ledger is locked".into(),
        0x6FAA => "Ledger disconnected".into(),
        other => format!("Ledger error 0x{other:04X}"),
    }
}

pub fn scan_devices() -> Result<Vec<LedgerDevice>> {
    let mut out = Vec::new();
    let class = PathBuf::from("/sys/class/hidraw");
    let Ok(entries) = fs::read_dir(&class) else {
        return Ok(out);
    };
    for ent in entries.flatten() {
        let name = ent.file_name().to_string_lossy().into_owned();
        if !name.starts_with("hidraw") {
            continue;
        }
        let uevent = ent.path().join("device/uevent");
        let Ok(text) = fs::read_to_string(&uevent) else {
            continue;
        };
        let mut vid = 0u16;
        let mut pid = 0u16;
        let mut serial = String::new();
        let mut product = String::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("HID_ID=") {
                let bits: Vec<&str> = rest.split(':').collect();
                if bits.len() >= 3 {
                    vid = u16::from_str_radix(bits[1].trim_start_matches("0000"), 16).unwrap_or(0);
                    pid = u16::from_str_radix(bits[2].trim_start_matches("0000"), 16).unwrap_or(0);
                }
            }
            if let Some(rest) = line.strip_prefix("HID_UNIQ=") {
                serial = rest.trim().to_string();
            }
            if let Some(rest) = line.strip_prefix("HID_NAME=") {
                product = rest.trim().to_string();
            }
        }
        if vid != LEDGER_VID {
            continue;
        }
        let hidraw = format!("/dev/{name}");
        out.push(LedgerDevice {
            path: hidraw.clone(),
            hidraw,
            model: ledger_model(pid, &product),
            serial,
        });
    }
    Ok(out)
}

fn ledger_model(pid: u16, product: &str) -> String {
    let p = product.to_ascii_lowercase();
    if p.contains("nano s plus") || pid == 0x0005 {
        "Nano S Plus".into()
    } else if p.contains("nano x") || pid == 0x0004 {
        "Nano X".into()
    } else if p.contains("nano s") || pid == 0x0001 {
        "Nano S".into()
    } else if p.contains("stax") {
        "Stax".into()
    } else if p.contains("flex") {
        "Flex".into()
    } else if product.is_empty() {
        format!("Ledger {pid:04x}")
    } else {
        product.to_string()
    }
}

pub fn get_address(device_path: &str, derivation: &str, display: bool) -> Result<String> {
    let apdu = build_get_pubkey_apdu(derivation, display)?;
    let resp = exchange(device_path, &apdu, Duration::from_secs(20))?;
    if resp.len() < 32 {
        return Err(Error::msg("Ledger pubkey truncated"));
    }
    let mut pk = [0u8; 32];
    pk.copy_from_slice(&resp[..32]);
    Ok(tx::encode_pubkey(&pk))
}

pub fn sign_message(device_path: &str, derivation: &str, message: &[u8]) -> Result<[u8; 64]> {
    let apdus = build_sign_apdus(derivation, message)?;
    let mut last = Vec::new();
    let timeout = Duration::from_secs(60);
    for (i, apdu) in apdus.iter().enumerate() {
        let wait = if i + 1 == apdus.len() {
            timeout
        } else {
            Duration::from_secs(15)
        };
        last = exchange(device_path, apdu, wait)?;
    }
    if last.len() != 64 {
        return Err(Error::msg(format!(
            "Ledger signature length {} (expected 64)",
            last.len()
        )));
    }
    let mut sig = [0u8; 64];
    sig.copy_from_slice(&last);
    Ok(sig)
}

pub struct LedgerSigner {
    device_path: String,
    derivation: String,
    address: String,
    public: [u8; 32],
}

impl LedgerSigner {
    pub fn connect(device_path: &str, derivation: &str, display: bool) -> Result<Self> {
        let address = get_address(device_path, derivation, display)?;
        let public = tx::decode_pubkey(&address)?;
        Ok(Self {
            device_path: device_path.to_string(),
            derivation: derivation.to_string(),
            address,
            public,
        })
    }

    pub fn from_stored(device_path: &str, derivation: &str, address: &str) -> Result<Self> {
        Ok(Self {
            device_path: device_path.to_string(),
            derivation: derivation.to_string(),
            address: address.to_string(),
            public: tx::decode_pubkey(address)?,
        })
    }
}

impl Signer for LedgerSigner {
    fn address(&self) -> &str {
        &self.address
    }
    fn public_key(&self) -> [u8; 32] {
        self.public
    }
    fn sign(&self, message: &[u8]) -> Result<[u8; 64]> {
        sign_message(&self.device_path, &self.derivation, message)
    }
    fn can_export_secret(&self) -> bool {
        false
    }
    fn is_connected(&self) -> bool {
        PathBuf::from(&self.device_path).exists()
    }
    fn kind(&self) -> WalletKeyType {
        WalletKeyType::Ledger
    }
}

fn exchange(device_path: &str, apdu: &[u8], timeout: Duration) -> Result<Vec<u8>> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc_nonblock())
        .open(device_path)
        .or_else(|_| File::open(device_path).and_then(|_f| OpenOptions::new().read(true).write(true).open(device_path)))
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                Error::msg(format!(
                    "Cannot open Ledger at {device_path}: permission denied. Install the VIBE udev rules, then unplug and replug."
                ))
            } else {
                Error::msg(format!("Cannot open Ledger at {device_path}: {e}"))
            }
        })?;
    for frame in pack_hid_frames(apdu) {
        file.write_all(&frame)
            .map_err(|e| Error::msg(format!("Ledger write failed: {e}")))?;
    }
    let deadline = Instant::now() + timeout;
    let mut frames: Vec<[u8; PACKET]> = Vec::new();
    let mut expected_len: Option<usize> = None;
    loop {
        if Instant::now() > deadline {
            return Err(Error::msg("Ledger timed out — confirm on the device, Solana app open"));
        }
        let mut buf = [0u8; PACKET];
        match file.read(&mut buf) {
            Ok(0) => {
                std::thread::sleep(Duration::from_millis(40));
                continue;
            }
            Ok(n) => {
                if n < 7 {
                    continue;
                }
                let mut pkt = [0u8; PACKET];
                pkt[..n].copy_from_slice(&buf[..n]);
                if frames.is_empty() {
                    expected_len = Some(u16::from_be_bytes([pkt[5], pkt[6]]) as usize);
                }
                frames.push(pkt);
                if let Some(len) = expected_len {
                    let got = unpack_hid_frames(&frames).unwrap_or_default();
                    if got.len() >= len {
                        let full = unpack_hid_frames(&frames)?;
                        return split_status(full);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(e) => return Err(Error::msg(format!("Ledger read failed: {e}"))),
        }
    }
}

fn libc_nonblock() -> i32 {
    0o4000 // O_NONBLOCK on Linux
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_is_hardened_be() {
        let raw = encode_derivation_path("m/44'/501'/0'").unwrap();
        assert_eq!(raw[0], 3);
        assert_eq!(&raw[1..5], &(44u32 | HARDENED).to_be_bytes());
        assert_eq!(&raw[5..9], &(501u32 | HARDENED).to_be_bytes());
        assert_eq!(&raw[9..13], &(0u32 | HARDENED).to_be_bytes());
    }

    #[test]
    fn hid_frames_roundtrip() {
        let apdu = vec![0xE0, 0x05, 0, 0, 3, 1, 2, 3];
        let frames = pack_hid_frames(&apdu);
        assert_eq!(frames[0][0], 0x01);
        assert_eq!(frames[0][1], 0x01);
        assert_eq!(frames[0][2], TAG_APDU);
        let unpacked = unpack_hid_frames(&frames).unwrap();
        assert_eq!(unpacked, apdu);
    }

    #[test]
    fn sign_apdu_chunks_single() {
        let apdus = build_sign_apdus("m/44'/501'/0'", &[7u8; 10]).unwrap();
        assert_eq!(apdus.len(), 1);
        assert_eq!(apdus[0][0], CLA);
        assert_eq!(apdus[0][1], INS_SIGN);
        assert_eq!(apdus[0][3], P2_LAST);
    }

    #[test]
    fn no_trezor_vendor() {
        assert_ne!(LEDGER_VID, 0x1209);
        assert_ne!(LEDGER_VID, 0x534c);
    }
}

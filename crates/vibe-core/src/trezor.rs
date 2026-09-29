// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto
// Portions derived from Cinder (https://github.com/gregnb/cinder),
// Copyright (c) 2026 Gregory Nowakowski, MIT License (see LICENSES/MIT-Cinder.txt).
//! Trezor Solana over Linux hidraw (One) and WebUSB (T / Safe 3 / Safe 5).
//! Protocol v1 protobuf. Codec is tested without a device. USB I/O is best-effort.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::hd;
use crate::signer::Signer;
use crate::tx;
use crate::wallet::WalletKeyType;

pub const TREZOR_ONE_VID: u16 = 0x534c;
pub const TREZOR_ONE_PID: u16 = 0x0001;
pub const TREZOR_T_VID: u16 = 0x1209;
pub const TREZOR_T_PID: u16 = 0x53c1;
pub const TREZOR_BOOTLOADER_PID: u16 = 0x53c0;
pub const DEFAULT_PATH: &str = "m/44'/501'/0'/0'";
const HARDENED: u32 = 0x8000_0000;
const PACKET: usize = 64;
const MSG_INITIALIZE: u16 = 0;
const MSG_FAILURE: u16 = 3;
const MSG_FEATURES: u16 = 17;
const MSG_PIN_MATRIX_REQUEST: u16 = 18;
const MSG_BUTTON_REQUEST: u16 = 26;
const MSG_BUTTON_ACK: u16 = 27;
const MSG_PASSPHRASE_REQUEST: u16 = 41;
const MSG_PASSPHRASE_ACK: u16 = 42;
const MSG_SOLANA_GET_ADDRESS: u16 = 902;
const MSG_SOLANA_ADDRESS: u16 = 903;
const MSG_SOLANA_SIGN_TX: u16 = 904;
const MSG_SOLANA_TX_SIGNATURE: u16 = 905;

#[derive(Clone, Debug)]
pub struct TrezorDevice {
    pub path: String,
    pub hidraw: String,
    pub model: String,
    pub serial: String,
    pub transport: String,
}

#[derive(Clone, Debug, Default)]
struct Features {
    model: String,
    internal_model: String,
    label: String,
    bootloader: bool,
    initialized: Option<bool>,
    passphrase_on_device: bool,
}

enum Handle {
    Hid(File),
    WebUsb { file: File, interface: u32 },
}

impl Drop for Handle {
    fn drop(&mut self) {
        if let Handle::WebUsb { file, interface } = self {
            let iface = *interface;
            unsafe {
                sys::ioctl(
                    file.as_raw_fd(),
                    sys::USBDEVFS_RELEASEINTERFACE,
                    &iface as *const u32 as *mut std::ffi::c_void,
                );
            }
        }
    }
}

pub fn encode_address_n(path: &str) -> Result<Vec<u32>> {
    let indexes = hd::path_indexes(path)?;
    if indexes.is_empty() || indexes.len() > 10 {
        return Err(Error::msg("Trezor derivation path must have 1–10 segments"));
    }
    Ok(indexes.into_iter().map(|i| i | HARDENED).collect())
}

pub fn encode_solana_get_address(path: &str, show_display: bool) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for idx in encode_address_n(path)? {
        proto_put_varint(&mut out, 1, idx as u64);
    }
    if show_display {
        proto_put_varint(&mut out, 2, 1);
    }
    Ok(out)
}

pub fn encode_solana_sign_tx(path: &str, message: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for idx in encode_address_n(path)? {
        proto_put_varint(&mut out, 1, idx as u64);
    }
    proto_put_bytes(&mut out, 2, message);
    Ok(out)
}

pub fn pack_v1(msg_type: u16, payload: &[u8]) -> Vec<[u8; PACKET]> {
    let mut frames = Vec::new();
    let mut pkt = [0u8; PACKET];
    pkt[0] = b'?';
    pkt[1] = b'#';
    pkt[2] = b'#';
    pkt[3..5].copy_from_slice(&msg_type.to_be_bytes());
    pkt[5..9].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    let first = payload.len().min(55);
    pkt[9..9 + first].copy_from_slice(&payload[..first]);
    frames.push(pkt);
    let mut offset = first;
    while offset < payload.len() {
        let mut cont = [0u8; PACKET];
        cont[0] = b'?';
        let take = (payload.len() - offset).min(63);
        cont[1..1 + take].copy_from_slice(&payload[offset..offset + take]);
        frames.push(cont);
        offset += take;
    }
    frames
}

pub fn unpack_v1(frames: &[[u8; PACKET]]) -> Result<(u16, Vec<u8>)> {
    if frames.is_empty() {
        return Err(Error::msg("Trezor returned no HID frames"));
    }
    let first = &frames[0];
    if first[0] != b'?' || first[1] != b'#' || first[2] != b'#' {
        return Err(Error::msg(
            "Trezor protocol mismatch — quit Trezor Suite, or this firmware needs Suite (v2)",
        ));
    }
    let msg_type = u16::from_be_bytes([first[3], first[4]]);
    let size = u32::from_be_bytes([first[5], first[6], first[7], first[8]]) as usize;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&first[9..]);
    for frame in frames.iter().skip(1) {
        if frame[0] != b'?' {
            return Err(Error::msg("Trezor HID continuation tag mismatch"));
        }
        out.extend_from_slice(&frame[1..]);
    }
    out.truncate(size);
    Ok((msg_type, out))
}

pub fn parse_solana_address(payload: &[u8]) -> Result<String> {
    for (field, value) in proto_fields(payload)? {
        if field == 1 {
            if let ProtoVal::Bytes(raw) = value {
                return String::from_utf8(raw.to_vec())
                    .map_err(|_| Error::msg("Trezor address was not UTF-8"));
            }
        }
    }
    Err(Error::msg("Trezor did not return a Solana address"))
}

pub fn parse_solana_signature(payload: &[u8]) -> Result<[u8; 64]> {
    for (field, value) in proto_fields(payload)? {
        if field == 1 {
            if let ProtoVal::Bytes(raw) = value {
                if raw.len() != 64 {
                    return Err(Error::msg(format!(
                        "Trezor signature length {} (expected 64)",
                        raw.len()
                    )));
                }
                let mut sig = [0u8; 64];
                sig.copy_from_slice(raw);
                return Ok(sig);
            }
        }
    }
    Err(Error::msg("Trezor did not return a signature"))
}

pub fn parse_failure(payload: &[u8]) -> String {
    let mut code = 0u64;
    let mut message = String::new();
    if let Ok(fields) = proto_fields(payload) {
        for (field, value) in fields {
            match (field, value) {
                (1, ProtoVal::Varint(v)) => code = v,
                (2, ProtoVal::Bytes(raw)) => {
                    message = String::from_utf8_lossy(raw).into_owned();
                }
                _ => {}
            }
        }
    }
    if !message.is_empty() {
        return format!("Trezor: {message}");
    }
    match code {
        4 => "Rejected on the Trezor".into(),
        5 | 6 | 7 => "Unlock the Trezor — enter PIN on the device".into(),
        11 => "Trezor has no seed — set it up in Trezor Suite first".into(),
        15 => "Trezor is busy — quit Trezor Suite and try again".into(),
        other => format!("Trezor error {other}"),
    }
}

fn parse_features(payload: &[u8]) -> Features {
    let mut feat = Features::default();
    if let Ok(fields) = proto_fields(payload) {
        for (field, value) in fields {
            match (field, value) {
                (5, ProtoVal::Varint(v)) => feat.bootloader = v != 0,
                (10, ProtoVal::Bytes(raw)) => feat.label = String::from_utf8_lossy(raw).into_owned(),
                (12, ProtoVal::Varint(v)) => feat.initialized = Some(v != 0),
                (21, ProtoVal::Bytes(raw)) => feat.model = String::from_utf8_lossy(raw).into_owned(),
                (36, ProtoVal::Varint(v)) => feat.passphrase_on_device = v != 0,
                (44, ProtoVal::Bytes(raw)) => {
                    feat.internal_model = String::from_utf8_lossy(raw).into_owned();
                }
                _ => {}
            }
        }
    }
    feat
}

fn model_from_ids(vid: u16, pid: u16, product: &str, feat: Option<&Features>) -> String {
    if pid == TREZOR_BOOTLOADER_PID {
        return "Trezor (bootloader)".into();
    }
    if let Some(feat) = feat {
        let key = feat.internal_model.to_ascii_uppercase();
        let named = match key.as_str() {
            "T1B1" => Some("Trezor One"),
            "T2T1" => Some("Trezor T"),
            "T2B1" => Some("Trezor Safe 3"),
            "T3B1" => Some("Trezor Safe 3"),
            "T3T1" => Some("Trezor Safe 5"),
            "T3W1" => Some("Trezor Safe 7"),
            _ => None,
        };
        if let Some(name) = named {
            return name.into();
        }
        if !feat.model.is_empty() {
            if feat.model.to_ascii_lowercase().starts_with("trezor") {
                return feat.model.clone();
            }
            return format!("Trezor {}", feat.model);
        }
        if !feat.label.is_empty() {
            return feat.label.clone();
        }
    }
    let p = product.to_ascii_lowercase();
    if p.contains("safe 7") {
        "Trezor Safe 7".into()
    } else if p.contains("safe 5") {
        "Trezor Safe 5".into()
    } else if p.contains("safe 3") || p.contains("safe3") {
        "Trezor Safe 3".into()
    } else if p.contains("model t") || p == "trezor t" || (vid == TREZOR_T_VID && p.contains("trezor t")) {
        "Trezor T".into()
    } else if vid == TREZOR_ONE_VID || p.contains("trezor one") {
        "Trezor One".into()
    } else if !product.is_empty() {
        product.to_string()
    } else {
        format!("Trezor {vid:04x}:{pid:04x}")
    }
}

pub fn scan_devices() -> Result<Vec<TrezorDevice>> {
    let mut out = Vec::new();
    scan_hidraw(&mut out);
    scan_webusb(&mut out);
    Ok(out)
}

fn scan_hidraw(out: &mut Vec<TrezorDevice>) {
    let class = PathBuf::from("/sys/class/hidraw");
    let Ok(entries) = fs::read_dir(&class) else {
        return;
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
        if vid != TREZOR_ONE_VID || pid != TREZOR_ONE_PID {
            continue;
        }
        let hidraw = format!("/dev/{name}");
        out.push(TrezorDevice {
            path: hidraw.clone(),
            hidraw,
            model: model_from_ids(vid, pid, &product, None),
            serial,
            transport: "hid".into(),
        });
    }
}

fn scan_webusb(out: &mut Vec<TrezorDevice>) {
    let bus = PathBuf::from("/sys/bus/usb/devices");
    let Ok(entries) = fs::read_dir(&bus) else {
        return;
    };
    for ent in entries.flatten() {
        let sys = ent.path();
        let Ok(vid_s) = fs::read_to_string(sys.join("idVendor")) else {
            continue;
        };
        let Ok(pid_s) = fs::read_to_string(sys.join("idProduct")) else {
            continue;
        };
        let vid = u16::from_str_radix(vid_s.trim(), 16).unwrap_or(0);
        let pid = u16::from_str_radix(pid_s.trim(), 16).unwrap_or(0);
        if vid != TREZOR_T_VID || (pid != TREZOR_T_PID && pid != TREZOR_BOOTLOADER_PID) {
            continue;
        }
        let busnum = fs::read_to_string(sys.join("busnum"))
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let devnum = fs::read_to_string(sys.join("devnum"))
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        if busnum == 0 || devnum == 0 {
            continue;
        }
        let product = fs::read_to_string(sys.join("product"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let serial = fs::read_to_string(sys.join("serial"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let path = format!("/dev/bus/usb/{busnum:03}/{devnum:03}");
        out.push(TrezorDevice {
            path: path.clone(),
            hidraw: path,
            model: model_from_ids(vid, pid, &product, None),
            serial,
            transport: "webusb".into(),
        });
    }
}

pub fn get_address(device_path: &str, derivation: &str, display: bool) -> Result<String> {
    let mut session = Session::open(device_path)?;
    session.initialize()?;
    let payload = encode_solana_get_address(derivation, display)?;
    let timeout = if display {
        Duration::from_secs(90)
    } else {
        Duration::from_secs(20)
    };
    let (ty, body) = session.call(MSG_SOLANA_GET_ADDRESS, &payload, timeout)?;
    if ty != MSG_SOLANA_ADDRESS {
        return Err(Error::msg(format!("Trezor unexpected message {ty}")));
    }
    parse_solana_address(&body)
}

pub fn sign_message(device_path: &str, derivation: &str, message: &[u8]) -> Result<[u8; 64]> {
    let mut session = Session::open(device_path)?;
    session.initialize()?;
    let payload = encode_solana_sign_tx(derivation, message)?;
    let (ty, body) = session.call(MSG_SOLANA_SIGN_TX, &payload, Duration::from_secs(180))?;
    if ty != MSG_SOLANA_TX_SIGNATURE {
        return Err(Error::msg(format!("Trezor unexpected message {ty}")));
    }
    parse_solana_signature(&body)
}

pub struct TrezorSigner {
    device_path: String,
    derivation: String,
    address: String,
    public: [u8; 32],
}

impl TrezorSigner {
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

impl Signer for TrezorSigner {
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
        WalletKeyType::Trezor
    }
}

struct Session {
    handle: Handle,
    passphrase_on_device: bool,
}

impl Session {
    fn open(device_path: &str) -> Result<Self> {
        let path = resolve_path(device_path)?;
        let handle = if path.contains("/bus/usb/") {
            open_webusb(&path)?
        } else {
            open_hid(&path)?
        };
        Ok(Self {
            handle,
            passphrase_on_device: false,
        })
    }

    fn initialize(&mut self) -> Result<Features> {
        let (ty, body) = self.call(MSG_INITIALIZE, &[], Duration::from_secs(8))?;
        if ty != MSG_FEATURES {
            return Err(Error::msg("Trezor did not return Features — unlock it and try again"));
        }
        let feat = parse_features(&body);
        if feat.bootloader {
            return Err(Error::msg("Trezor is in bootloader — open the wallet firmware"));
        }
        if feat.initialized == Some(false) {
            return Err(Error::msg("Trezor has no seed — set it up in Trezor Suite first"));
        }
        self.passphrase_on_device = feat.passphrase_on_device;
        Ok(feat)
    }

    fn call(&mut self, msg_type: u16, payload: &[u8], timeout: Duration) -> Result<(u16, Vec<u8>)> {
        write_frames(&mut self.handle, &pack_v1(msg_type, payload))?;
        let deadline = Instant::now() + timeout;
        loop {
            let remain = deadline.saturating_duration_since(Instant::now());
            if remain.is_zero() {
                return Err(Error::msg(
                    "Trezor timed out — confirm on the device, quit Trezor Suite if it is open",
                ));
            }
            let (ty, body) = read_message(&mut self.handle, remain)?;
            match ty {
                MSG_BUTTON_REQUEST => {
                    write_frames(&mut self.handle, &pack_v1(MSG_BUTTON_ACK, &[]))?;
                }
                MSG_PIN_MATRIX_REQUEST => {
                    return Err(Error::msg(
                        "Enter PIN on the Trezor. Trezor One PIN matrix is not in this build — use a Safe 3 / T, or unlock in Trezor Suite first.",
                    ));
                }
                MSG_PASSPHRASE_REQUEST => {
                    let ack = encode_passphrase_ack(self.passphrase_on_device);
                    write_frames(&mut self.handle, &pack_v1(MSG_PASSPHRASE_ACK, &ack))?;
                }
                MSG_FAILURE => return Err(Error::msg(parse_failure(&body))),
                other => return Ok((other, body)),
            }
        }
    }
}

fn encode_passphrase_ack(on_device: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if on_device {
        proto_put_varint(&mut out, 3, 1);
    } else {
        proto_put_bytes(&mut out, 1, b"");
    }
    out
}

fn resolve_path(device_path: &str) -> Result<String> {
    let want = device_path.trim();
    if !want.is_empty() && PathBuf::from(want).exists() {
        return Ok(want.to_string());
    }
    let devices = scan_devices()?;
    if let Some(hit) = devices.iter().find(|d| {
        !want.is_empty() && (d.path == want || d.hidraw == want || d.serial == want)
    }) {
        return Ok(hit.path.clone());
    }
    devices
        .into_iter()
        .next()
        .map(|d| d.path)
        .ok_or_else(|| {
            Error::msg("Plug in a Trezor Safe 3 / T, unlock it, and quit Trezor Suite if it has the device")
        })
}

fn open_hid(path: &str) -> Result<Handle> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(0o4000)
        .open(path)
        .or_else(|_| OpenOptions::new().read(true).write(true).open(path))
        .map_err(|e| Error::msg(format!("Cannot open Trezor at {path}: {e}")))?;
    Ok(Handle::Hid(file))
}

fn open_webusb(path: &str) -> Result<Handle> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| {
            Error::msg(format!(
                "Cannot open Trezor at {path}: {e}. Install the VIBE udev rules and unplug/replug, and quit Trezor Suite."
            ))
        })?;
    let iface: u32 = 0;
    let fd = file.as_raw_fd();
    let claim = unsafe {
        sys::ioctl(
            fd,
            sys::USBDEVFS_CLAIMINTERFACE,
            &iface as *const u32 as *mut std::ffi::c_void,
        )
    };
    if claim < 0 {
        let mut disc = sys::DisconnectClaim {
            interface: 0,
            flags: 0,
            driver: [0; 256],
        };
        let _ = unsafe {
            sys::ioctl(
                fd,
                sys::USBDEVFS_DISCONNECT_CLAIM,
                &mut disc as *mut _ as *mut std::ffi::c_void,
            )
        };
        let claim2 = unsafe {
            sys::ioctl(
                fd,
                sys::USBDEVFS_CLAIMINTERFACE,
                &iface as *const u32 as *mut std::ffi::c_void,
            )
        };
        if claim2 < 0 {
            return Err(Error::msg(
                "Trezor is in use — quit Trezor Suite / Bridge and try again",
            ));
        }
    }
    Ok(Handle::WebUsb {
        file,
        interface: iface,
    })
}

fn write_frames(handle: &mut Handle, frames: &[[u8; PACKET]]) -> Result<()> {
    for frame in frames {
        match handle {
            Handle::Hid(file) => {
                file.write_all(frame)
                    .map_err(|e| Error::msg(format!("Trezor write failed: {e}")))?;
            }
            Handle::WebUsb { file, .. } => {
                let mut pkt = *frame;
                usb_bulk(file, 0x01, &mut pkt, 800)
                    .map_err(|e| Error::msg(format!("Trezor write failed: {e}")))?;
            }
        }
    }
    Ok(())
}

fn read_message(handle: &mut Handle, timeout: Duration) -> Result<(u16, Vec<u8>)> {
    let deadline = Instant::now() + timeout;
    let mut frames: Vec<[u8; PACKET]> = Vec::new();
    let mut expected: Option<usize> = None;
    loop {
        if Instant::now() > deadline {
            return Err(Error::msg(
                "Trezor timed out — confirm on the device, quit Trezor Suite if it is open",
            ));
        }
        let remain = deadline.saturating_duration_since(Instant::now());
        let pkt = match handle {
            Handle::Hid(file) => read_hid_packet(file, remain)?,
            Handle::WebUsb { file, .. } => read_usb_packet(file, remain)?,
        };
        let Some(pkt) = pkt else {
            continue;
        };
        if frames.is_empty() {
            if pkt[0] != b'?' || pkt[1] != b'#' || pkt[2] != b'#' {
                continue;
            }
            expected = Some(u32::from_be_bytes([pkt[5], pkt[6], pkt[7], pkt[8]]) as usize);
        }
        frames.push(pkt);
        if let Some(size) = expected {
            let got = frames_payload_len(&frames);
            if got >= size {
                return unpack_v1(&frames);
            }
        }
    }
}

fn frames_payload_len(frames: &[[u8; PACKET]]) -> usize {
    if frames.is_empty() {
        return 0;
    }
    let mut n = 55.min(PACKET.saturating_sub(9));
    if frames.len() > 1 {
        n += (frames.len() - 1) * 63;
    }
    n
}

fn read_hid_packet(file: &mut File, _timeout: Duration) -> Result<Option<[u8; PACKET]>> {
    let mut buf = [0u8; 80];
    match file.read(&mut buf) {
        Ok(0) => {
            std::thread::sleep(Duration::from_millis(40));
            Ok(None)
        }
        Ok(n) => Ok(normalize_packet(&buf[..n])),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            std::thread::sleep(Duration::from_millis(40));
            Ok(None)
        }
        Err(e) => Err(Error::msg(format!("Trezor read failed: {e}"))),
    }
}

fn read_usb_packet(file: &mut File, timeout: Duration) -> Result<Option<[u8; PACKET]>> {
    let wait = timeout.min(Duration::from_millis(250)).as_millis() as u32;
    let mut buf = [0u8; PACKET];
    match usb_bulk(file, 0x81, &mut buf, wait.max(50)) {
        Ok(n) => Ok(normalize_packet(&buf[..n])),
        Err(e) if e.contains("timeout") || e.contains("110") => Ok(None),
        Err(e) => Err(Error::msg(format!("Trezor read failed: {e}"))),
    }
}

fn normalize_packet(raw: &[u8]) -> Option<[u8; PACKET]> {
    if raw.is_empty() {
        return None;
    }
    let slice = if raw[0] == 0 && raw.len() > 1 && raw[1] == b'?' {
        &raw[1..]
    } else {
        raw
    };
    if slice.len() < 3 {
        return None;
    }
    let mut pkt = [0u8; PACKET];
    let n = slice.len().min(PACKET);
    pkt[..n].copy_from_slice(&slice[..n]);
    Some(pkt)
}

fn usb_bulk(file: &File, ep: u32, buf: &mut [u8], timeout_ms: u32) -> std::result::Result<usize, String> {
    let mut xfer = sys::UsbdevfsBulkTransfer {
        ep,
        len: buf.len() as u32,
        timeout: timeout_ms,
        data: buf.as_mut_ptr(),
    };
    let rc = unsafe {
        sys::ioctl(
            file.as_raw_fd(),
            sys::USBDEVFS_BULK,
            &mut xfer as *mut _ as *mut std::ffi::c_void,
        )
    };
    if rc < 0 {
        let err = std::io::Error::last_os_error();
        return Err(err.to_string());
    }
    Ok(rc as usize)
}

mod sys {
    #[repr(C)]
    pub struct UsbdevfsBulkTransfer {
        pub ep: u32,
        pub len: u32,
        pub timeout: u32,
        pub data: *mut u8,
    }

    #[repr(C)]
    pub struct DisconnectClaim {
        pub interface: u32,
        pub flags: u32,
        pub driver: [u8; 256],
    }

    pub const USBDEVFS_BULK: u64 = 0xC018_5502;
    pub const USBDEVFS_CLAIMINTERFACE: u64 = 0x8004_550F;
    pub const USBDEVFS_RELEASEINTERFACE: u64 = 0x8004_5510;
    pub const USBDEVFS_DISCONNECT_CLAIM: u64 = 0x8108_551B;

    extern "C" {
        pub fn ioctl(fd: i32, request: u64, argp: *mut std::ffi::c_void) -> i32;
    }
}

fn proto_put_varint(out: &mut Vec<u8>, field: u32, value: u64) {
    put_varint(out, ((field as u64) << 3) | 0);
    put_varint(out, value);
}

fn proto_put_bytes(out: &mut Vec<u8>, field: u32, data: &[u8]) {
    put_varint(out, ((field as u64) << 3) | 2);
    put_varint(out, data.len() as u64);
    out.extend_from_slice(data);
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
            out.push(byte);
        } else {
            out.push(byte);
            break;
        }
    }
}

enum ProtoVal<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Skip,
}

fn proto_fields(data: &[u8]) -> Result<Vec<(u32, ProtoVal<'_>)>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        let (key, n) = read_varint(data, i)?;
        i += n;
        let field = (key >> 3) as u32;
        let wire = (key & 7) as u8;
        match wire {
            0 => {
                let (v, n) = read_varint(data, i)?;
                i += n;
                out.push((field, ProtoVal::Varint(v)));
            }
            1 => {
                i = i.saturating_add(8);
                out.push((field, ProtoVal::Skip));
            }
            2 => {
                let (len, n) = read_varint(data, i)?;
                i += n;
                let end = i + len as usize;
                if end > data.len() {
                    return Err(Error::msg("Trezor protobuf truncated"));
                }
                out.push((field, ProtoVal::Bytes(&data[i..end])));
                i = end;
            }
            5 => {
                i = i.saturating_add(4);
                out.push((field, ProtoVal::Skip));
            }
            _ => return Err(Error::msg("Trezor protobuf wire type")),
        }
    }
    Ok(out)
}

fn read_varint(data: &[u8], mut i: usize) -> Result<(u64, usize)> {
    let start = i;
    let mut value = 0u64;
    let mut shift = 0;
    while i < data.len() {
        let byte = data[i];
        i += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, i - start));
        }
        shift += 7;
        if shift > 63 {
            return Err(Error::msg("Trezor protobuf varint overflow"));
        }
    }
    Err(Error::msg("Trezor protobuf truncated"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_is_hardened() {
        let n = encode_address_n("m/44'/501'/0'/0'").unwrap();
        assert_eq!(n.len(), 4);
        assert_eq!(n[0], 44 | HARDENED);
        assert_eq!(n[1], 501 | HARDENED);
        assert_eq!(n[2], HARDENED);
        assert_eq!(n[3], HARDENED);
    }

    #[test]
    fn v1_roundtrip_empty() {
        let frames = pack_v1(MSG_INITIALIZE, &[]);
        assert_eq!(frames.len(), 1);
        assert_eq!(&frames[0][0..3], b"?##");
        assert_eq!(&frames[0][3..5], &0u16.to_be_bytes());
        assert_eq!(&frames[0][5..9], &0u32.to_be_bytes());
        let (ty, body) = unpack_v1(&frames).unwrap();
        assert_eq!(ty, MSG_INITIALIZE);
        assert!(body.is_empty());
    }

    #[test]
    fn v1_roundtrip_chunked() {
        let payload = vec![7u8; 80];
        let frames = pack_v1(MSG_SOLANA_SIGN_TX, &payload);
        assert!(frames.len() >= 2);
        assert_eq!(frames[1][0], b'?');
        let (ty, body) = unpack_v1(&frames).unwrap();
        assert_eq!(ty, MSG_SOLANA_SIGN_TX);
        assert_eq!(body, payload);
    }

    #[test]
    fn get_address_protobuf() {
        let raw = encode_solana_get_address("m/44'/501'/0'/0'", true).unwrap();
        assert_eq!(raw[0], 0x08);
        assert!(raw.contains(&0x10));
        let fields = proto_fields(&raw).unwrap();
        let nums: Vec<u64> = fields
            .iter()
            .filter(|(f, _)| *f == 1)
            .filter_map(|(_, v)| match v {
                ProtoVal::Varint(n) => Some(*n),
                _ => None,
            })
            .collect();
        assert_eq!(nums.len(), 4);
        assert_eq!(nums[0], (44 | HARDENED) as u64);
    }

    #[test]
    fn sign_tx_protobuf_has_bytes() {
        let msg = [9u8; 12];
        let raw = encode_solana_sign_tx("m/44'/501'/0'/0'", &msg).unwrap();
        let fields = proto_fields(&raw).unwrap();
        let bytes = fields.iter().find(|(f, _)| *f == 2);
        match bytes {
            Some((_, ProtoVal::Bytes(b))) => assert_eq!(*b, &msg),
            _ => panic!("missing serialized_tx"),
        }
    }

    #[test]
    fn parse_address_and_sig() {
        let mut addr = Vec::new();
        proto_put_bytes(&mut addr, 1, b"So11111111111111111111111111111111111111112");
        assert!(parse_solana_address(&addr).unwrap().starts_with("So11"));
        let mut sigp = Vec::new();
        proto_put_bytes(&mut sigp, 1, &[3u8; 64]);
        assert_eq!(parse_solana_signature(&sigp).unwrap(), [3u8; 64]);
    }

    #[test]
    fn parse_failure_cancelled() {
        let mut raw = Vec::new();
        proto_put_varint(&mut raw, 1, 4);
        proto_put_bytes(&mut raw, 2, b"Cancelled");
        assert!(parse_failure(&raw).contains("Cancelled"));
    }

    #[test]
    fn vendor_ids() {
        assert_eq!(TREZOR_ONE_VID, 0x534c);
        assert_eq!(TREZOR_T_VID, 0x1209);
        assert_eq!(TREZOR_T_PID, 0x53c1);
        assert_ne!(TREZOR_T_VID, crate::ledger::LEDGER_VID);
    }

    #[test]
    fn default_path_is_phantom() {
        assert_eq!(DEFAULT_PATH, "m/44'/501'/0'/0'");
    }

    #[test]
    fn usbdevfs_bulk_layout() {
        assert_eq!(std::mem::size_of::<sys::UsbdevfsBulkTransfer>(), 24);
    }
}

// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto
// Portions derived from Cinder (https://github.com/gregnb/cinder),
// Copyright (c) 2026 Gregory Nowakowski, MIT License (see LICENSES/MIT-Cinder.txt).

use crate::error::{Error, Result};
use crate::keypair::Keypair;
use crate::programs::ProgramIds;

#[derive(Clone, Debug)]
pub struct AccountMeta {
    pub pubkey: String,
    pub is_signer: bool,
    pub is_writable: bool,
}

impl AccountMeta {
    pub fn writable_signer(pubkey: impl Into<String>) -> Self {
        Self {
            pubkey: pubkey.into(),
            is_signer: true,
            is_writable: true,
        }
    }
    pub fn writable(pubkey: impl Into<String>) -> Self {
        Self {
            pubkey: pubkey.into(),
            is_signer: false,
            is_writable: true,
        }
    }
    pub fn readonly_signer(pubkey: impl Into<String>) -> Self {
        Self {
            pubkey: pubkey.into(),
            is_signer: true,
            is_writable: false,
        }
    }
    pub fn readonly(pubkey: impl Into<String>) -> Self {
        Self {
            pubkey: pubkey.into(),
            is_signer: false,
            is_writable: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Instruction {
    pub program_id: String,
    pub accounts: Vec<AccountMeta>,
    pub data: Vec<u8>,
}

pub fn encode_u32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}
pub fn encode_u64(v: u64) -> [u8; 8] {
    v.to_le_bytes()
}

pub fn compact_u16_encode(value: u16, out: &mut Vec<u8>) {
    if value < 0x80 {
        out.push(value as u8);
    } else if value < 0x4000 {
        out.push((value as u8 & 0x7f) | 0x80);
        out.push((value >> 7) as u8);
    } else {
        out.push((value as u8 & 0x7f) | 0x80);
        out.push((((value >> 7) as u8) & 0x7f) | 0x80);
        out.push((value >> 14) as u8);
    }
}

pub fn compact_u16_decode(data: &[u8], offset: &mut usize) -> Result<u16> {
    let mut value: u16 = 0;
    let mut shift = 0;
    for _ in 0..3 {
        if *offset >= data.len() {
            return Err(Error::msg("compact-u16 truncated"));
        }
        let byte = data[*offset];
        *offset += 1;
        value |= u16::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
    }
    Err(Error::msg("invalid compact-u16"))
}

pub fn decode_pubkey(s: &str) -> Result<[u8; 32]> {
    let bytes = bs58::decode(s)
        .into_vec()
        .map_err(|_| Error::InvalidAddress)?;
    if bytes.len() != 32 {
        return Err(Error::InvalidAddress);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

pub fn encode_pubkey(bytes: &[u8; 32]) -> String {
    bs58::encode(bytes).into_string()
}

fn is_on_curve(bytes: &[u8; 32]) -> bool {
    curve25519_dalek::edwards::CompressedEdwardsY(*bytes)
        .decompress()
        .is_some()
}

pub fn find_program_address(seeds: &[&[u8]], program_id: &[u8; 32]) -> Result<([u8; 32], u8)> {
    use sha2::{Digest, Sha256};
    for bump in (0u8..=255).rev() {
        let mut hasher = Sha256::new();
        for seed in seeds {
            hasher.update(seed);
        }
        hasher.update([bump]);
        hasher.update(program_id);
        hasher.update(b"ProgramDerivedAddress");
        let hash: [u8; 32] = hasher.finalize().into();
        if !is_on_curve(&hash) {
            return Ok((hash, bump));
        }
    }
    Err(Error::msg("unable to find program address"))
}

pub fn associated_token_address(owner: &str, mint: &str, token_program: &str) -> Result<String> {
    let owner_b = decode_pubkey(owner)?;
    let mint_b = decode_pubkey(mint)?;
    let token_b = decode_pubkey(token_program)?;
    let ata_prog = decode_pubkey(ProgramIds::ASSOCIATED_TOKEN)?;
    let (pda, _) = find_program_address(&[&owner_b, &token_b, &mint_b], &ata_prog)?;
    Ok(encode_pubkey(&pda))
}

/// Classic SPL mint account size (no extensions).
pub const SPL_MINT_LEN: usize = 82;
const METAPLEX_NAME_MAX: usize = 32;
const METAPLEX_SYMBOL_MAX: usize = 10;
const METAPLEX_URI_MAX: usize = 200;

pub fn metaplex_metadata_pda(mint: &str) -> Result<String> {
    let mint_b = decode_pubkey(mint)?;
    let prog = decode_pubkey(ProgramIds::METAPLEX_METADATA)?;
    let (pda, _) = find_program_address(&[b"metadata", &prog, &mint_b], &prog)?;
    Ok(encode_pubkey(&pda))
}

fn metaplex_fit(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn push_data_v2(data: &mut Vec<u8>, name: &str, symbol: &str, uri: &str) {
    borsh_string(metaplex_fit(name, METAPLEX_NAME_MAX), data);
    borsh_string(metaplex_fit(symbol, METAPLEX_SYMBOL_MAX), data);
    borsh_string(metaplex_fit(uri, METAPLEX_URI_MAX), data);
    data.extend_from_slice(&0u16.to_le_bytes());
    data.push(0); // creators None
    data.push(0); // collection None
    data.push(0); // uses None
}

/// Metaplex Token Metadata CreateMetadataAccountV3 (instruction 33).
pub fn create_metadata_account_v3(
    mint: &str,
    mint_authority: &str,
    payer: &str,
    update_authority: &str,
    name: &str,
    symbol: &str,
    uri: &str,
    is_mutable: bool,
) -> Result<Instruction> {
    let metadata = metaplex_metadata_pda(mint)?;
    let mut data = vec![33u8];
    push_data_v2(&mut data, name, symbol, uri);
    data.push(u8::from(is_mutable));
    data.push(0); // collection_details None
    Ok(Instruction {
        program_id: ProgramIds::METAPLEX_METADATA.to_string(),
        accounts: vec![
            AccountMeta::writable(metadata),
            AccountMeta::readonly(mint),
            AccountMeta::readonly_signer(mint_authority),
            AccountMeta::writable_signer(payer),
            AccountMeta::readonly(update_authority),
            AccountMeta::readonly(ProgramIds::SYSTEM),
            AccountMeta::readonly(ProgramIds::SYSVAR_RENT),
        ],
        data,
    })
}

/// Metaplex UpdateMetadataAccountV2 (instruction 15). `is_mutable` None leaves it.
pub fn update_metadata_account_v2(
    mint: &str,
    update_authority: &str,
    name: &str,
    symbol: &str,
    uri: &str,
    is_mutable: Option<bool>,
) -> Result<Instruction> {
    let metadata = metaplex_metadata_pda(mint)?;
    let mut data = vec![15u8];
    data.push(1); // Some(DataV2)
    push_data_v2(&mut data, name, symbol, uri);
    data.push(0); // update_authority None
    data.push(0); // primary_sale None
    match is_mutable {
        Some(flag) => {
            data.push(1);
            data.push(u8::from(flag));
        }
        None => data.push(0),
    }
    Ok(Instruction {
        program_id: ProgramIds::METAPLEX_METADATA.to_string(),
        accounts: vec![
            AccountMeta::writable(metadata),
            AccountMeta::readonly_signer(update_authority),
        ],
        data,
    })
}

/// Seal Metaplex metadata (`is_mutable = false`).
pub fn revoke_metaplex_mutable(mint: &str, update_authority: &str) -> Result<Instruction> {
    let metadata = metaplex_metadata_pda(mint)?;
    let data = vec![15u8, 0, 0, 0, 1, 0];
    Ok(Instruction {
        program_id: ProgramIds::METAPLEX_METADATA.to_string(),
        accounts: vec![
            AccountMeta::writable(metadata),
            AccountMeta::readonly_signer(update_authority),
        ],
        data,
    })
}

pub fn create_associated_token_account_idempotent(
    payer: &str,
    owner: &str,
    mint: &str,
    token_program: &str,
) -> Result<Instruction> {
    let ata = associated_token_address(owner, mint, token_program)?;
    Ok(Instruction {
        program_id: ProgramIds::ASSOCIATED_TOKEN.to_string(),
        accounts: vec![
            AccountMeta::writable_signer(payer),
            AccountMeta::writable(ata),
            AccountMeta::readonly(owner),
            AccountMeta::readonly(mint),
            AccountMeta::readonly(ProgramIds::SYSTEM),
            AccountMeta::readonly(token_program),
        ],
        data: vec![1],
    })
}

/// SPL Token / Token-2022 Burn (opcode 8).
pub fn token_burn(
    account: &str,
    mint: &str,
    owner: &str,
    amount: u64,
    token_program: &str,
) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(8);
    data.extend_from_slice(&encode_u64(amount));
    Instruction {
        program_id: token_program.to_string(),
        accounts: vec![
            AccountMeta::writable(account),
            AccountMeta::writable(mint),
            AccountMeta::readonly_signer(owner),
        ],
        data,
    }
}

/// SPL Token / Token-2022 CloseAccount (opcode 9). Rent returns to `dest`.
pub fn token_close_account(
    account: &str,
    dest: &str,
    owner: &str,
    token_program: &str,
) -> Instruction {
    Instruction {
        program_id: token_program.to_string(),
        accounts: vec![
            AccountMeta::writable(account),
            AccountMeta::writable(dest),
            AccountMeta::readonly_signer(owner),
        ],
        data: vec![9],
    }
}

/// SPL Token / Token-2022 ThawAccount (opcode 11). Needed before MintTo when
/// DefaultAccountState is Frozen — new ATAs start frozen.
pub fn token_thaw_account(
    account: &str,
    mint: &str,
    authority: &str,
    token_program: &str,
) -> Instruction {
    Instruction {
        program_id: token_program.to_string(),
        accounts: vec![
            AccountMeta::writable(account),
            AccountMeta::readonly(mint),
            AccountMeta::readonly_signer(authority),
        ],
        data: vec![11],
    }
}

/// SPL Token account base size. Token-2022 appends TLV extensions after this.
pub const TOKEN_ACCOUNT_LEN: usize = 165;
const EXT_TRANSFER_FEE_AMOUNT: u16 = 2;

/// Raw token amount at offset 64. Same layout for Token and Token-2022.
pub fn token_account_amount(data: &[u8]) -> Option<u64> {
    let bytes: [u8; 8] = data.get(64..72)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

/// AccountState::Frozen = 2 at offset 108.
pub fn token_account_frozen(data: &[u8]) -> bool {
    data.get(108).copied() == Some(2)
}

/// Token-2022 TransferFeeAmount.withheld_amount, or 0.
pub fn token_account_withheld(data: &[u8]) -> u64 {
    if data.len() <= TOKEN_ACCOUNT_LEN + 1 {
        return 0;
    }
    let mut i = TOKEN_ACCOUNT_LEN + 1;
    while i + 4 <= data.len() {
        let ty = u16::from_le_bytes([data[i], data[i + 1]]);
        let len = u16::from_le_bytes([data[i + 2], data[i + 3]]) as usize;
        i += 4;
        if ty == 0 {
            break;
        }
        if i + len > data.len() {
            break;
        }
        if ty == EXT_TRANSFER_FEE_AMOUNT && len >= 8 {
            if let Ok(bytes) = <[u8; 8]>::try_from(&data[i..i + 8]) {
                return u64::from_le_bytes(bytes);
            }
        }
        i += len;
    }
    0
}

/// Token-2022 HarvestWithheldTokensToMint (TransferFeeExtension / opcode 26, 3).
pub fn harvest_withheld_to_mint(mint: &str, account: &str, token_program: &str) -> Instruction {
    Instruction {
        program_id: token_program.to_string(),
        accounts: vec![
            AccountMeta::writable(mint),
            AccountMeta::writable(account),
        ],
        data: vec![26, 3],
    }
}

/// SPL Token / Token-2022 TransferChecked (opcode 12).
pub fn token_transfer_checked(
    source: &str,
    mint: &str,
    dest: &str,
    owner: &str,
    amount: u64,
    decimals: u8,
    token_program: &str,
) -> Instruction {
    let mut data = Vec::with_capacity(10);
    data.push(12);
    data.extend_from_slice(&encode_u64(amount));
    data.push(decimals);
    Instruction {
        program_id: token_program.to_string(),
        accounts: vec![
            AccountMeta::writable(source),
            AccountMeta::readonly(mint),
            AccountMeta::writable(dest),
            AccountMeta::readonly_signer(owner),
        ],
        data,
    }
}

/// Compute Budget: SetComputeUnitLimit (discriminator 2).
pub fn set_compute_unit_limit(units: u32) -> Instruction {
    let mut data = Vec::with_capacity(5);
    data.push(2);
    data.extend_from_slice(&units.to_le_bytes());
    Instruction {
        program_id: ProgramIds::COMPUTE_BUDGET.to_string(),
        accounts: vec![],
        data,
    }
}

/// Compute Budget: SetComputeUnitPrice (discriminator 3), micro-lamports per CU.
pub fn set_compute_unit_price(micro_lamports: u64) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(3);
    data.extend_from_slice(&micro_lamports.to_le_bytes());
    Instruction {
        program_id: ProgramIds::COMPUTE_BUDGET.to_string(),
        accounts: vec![],
        data,
    }
}

/// SPL Memo. Public UTF-8 note on the transaction.
pub fn memo(signer: &str, text: &str) -> Instruction {
    Instruction {
        program_id: ProgramIds::MEMO.to_string(),
        accounts: vec![AccountMeta::readonly_signer(signer)],
        data: text.as_bytes().to_vec(),
    }
}

pub fn system_transfer(from: &str, to: &str, lamports: u64) -> Instruction {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&encode_u32(2));
    data.extend_from_slice(&encode_u64(lamports));
    Instruction {
        program_id: ProgramIds::SYSTEM.to_string(),
        accounts: vec![
            AccountMeta::writable_signer(from),
            AccountMeta::writable(to),
        ],
        data,
    }
}

pub fn create_account(
    from: &str,
    new_account: &str,
    lamports: u64,
    space: u64,
    owner: &str,
) -> Result<Instruction> {
    let mut data = Vec::new();
    data.extend_from_slice(&encode_u32(0));
    data.extend_from_slice(&encode_u64(lamports));
    data.extend_from_slice(&encode_u64(space));
    data.extend_from_slice(&decode_pubkey(owner)?);
    Ok(Instruction {
        program_id: ProgramIds::SYSTEM.to_string(),
        accounts: vec![
            AccountMeta::writable_signer(from),
            AccountMeta::writable_signer(new_account),
        ],
        data,
    })
}

/// Durable nonce account is 80 bytes (version, state, authority, nonce hash, fee).
pub const NONCE_ACCOUNT_LEN: usize = 80;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NonceAccount {
    pub version: u32,
    pub state: u32,
    pub authority: String,
    pub nonce: String,
    pub lamports_per_signature: u64,
}

impl NonceAccount {
    pub fn is_initialized(&self) -> bool {
        self.state == 1
    }

    pub fn from_account_data(data: &[u8]) -> Result<Self> {
        if data.len() < NONCE_ACCOUNT_LEN {
            return Err(Error::msg("nonce account truncated"));
        }
        let version = u32::from_le_bytes(data[0..4].try_into().unwrap());
        let state = u32::from_le_bytes(data[4..8].try_into().unwrap());
        let mut auth = [0u8; 32];
        auth.copy_from_slice(&data[8..40]);
        let mut nonce = [0u8; 32];
        nonce.copy_from_slice(&data[40..72]);
        let fee = u64::from_le_bytes(data[72..80].try_into().unwrap());
        Ok(Self {
            version,
            state,
            authority: encode_pubkey(&auth),
            nonce: encode_pubkey(&nonce),
            lamports_per_signature: fee,
        })
    }
}

/// System AdvanceNonceAccount (index 4). Must be the first instruction of a nonce tx.
pub fn nonce_advance(nonce_account: &str, authority: &str) -> Instruction {
    let mut data = Vec::with_capacity(4);
    data.extend_from_slice(&encode_u32(4));
    Instruction {
        program_id: ProgramIds::SYSTEM.to_string(),
        accounts: vec![
            AccountMeta::writable(nonce_account),
            AccountMeta::readonly(ProgramIds::SYSVAR_RECENT_BLOCKHASHES),
            AccountMeta::readonly_signer(authority),
        ],
        data,
    }
}

/// System WithdrawNonceAccount (index 5).
pub fn nonce_withdraw(nonce_account: &str, authority: &str, dest: &str, lamports: u64) -> Instruction {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&encode_u32(5));
    data.extend_from_slice(&encode_u64(lamports));
    Instruction {
        program_id: ProgramIds::SYSTEM.to_string(),
        accounts: vec![
            AccountMeta::writable(nonce_account),
            AccountMeta::writable(dest),
            AccountMeta::readonly(ProgramIds::SYSVAR_RECENT_BLOCKHASHES),
            AccountMeta::readonly(ProgramIds::SYSVAR_RENT),
            AccountMeta::readonly_signer(authority),
        ],
        data,
    }
}

/// System InitializeNonceAccount (index 6).
pub fn nonce_initialize(nonce_account: &str, authority: &str) -> Result<Instruction> {
    let mut data = Vec::new();
    data.extend_from_slice(&encode_u32(6));
    data.extend_from_slice(&decode_pubkey(authority)?);
    Ok(Instruction {
        program_id: ProgramIds::SYSTEM.to_string(),
        accounts: vec![
            AccountMeta::writable(nonce_account),
            AccountMeta::readonly(ProgramIds::SYSVAR_RECENT_BLOCKHASHES),
            AccountMeta::readonly(ProgramIds::SYSVAR_RENT),
        ],
        data,
    })
}

/// System AuthorizeNonceAccount (index 7).
pub fn nonce_authorize(nonce_account: &str, authority: &str, new_authority: &str) -> Result<Instruction> {
    let mut data = Vec::new();
    data.extend_from_slice(&encode_u32(7));
    data.extend_from_slice(&decode_pubkey(new_authority)?);
    Ok(Instruction {
        program_id: ProgramIds::SYSTEM.to_string(),
        accounts: vec![
            AccountMeta::writable(nonce_account),
            AccountMeta::readonly_signer(authority),
        ],
        data,
    })
}

pub fn with_durable_nonce(
    nonce_account: &str,
    authority: &str,
    mut ixs: Vec<Instruction>,
) -> Vec<Instruction> {
    let mut out = vec![nonce_advance(nonce_account, authority)];
    out.append(&mut ixs);
    out
}

/// Token instruction COption<Pubkey>: 1-byte tag, then 32-byte key if Some.
/// Account-data PodCOption is 4-byte aligned — do not use that here.
fn coption_pubkey(pk: Option<&str>) -> Result<Vec<u8>> {
    match pk {
        Some(p) if !p.is_empty() => {
            let mut out = Vec::with_capacity(33);
            out.push(1);
            out.extend_from_slice(&decode_pubkey(p)?);
            Ok(out)
        }
        _ => Ok(vec![0u8]),
    }
}

fn optional_nonzero_pubkey(pk: Option<&str>) -> Result<[u8; 32]> {
    match pk {
        Some(p) if !p.is_empty() => decode_pubkey(p),
        _ => Ok([0u8; 32]),
    }
}

/// Token-2022 / SPL InitializeMint2 (opcode 20).
pub fn initialize_mint2(
    mint: &str,
    decimals: u8,
    mint_authority: &str,
    freeze_authority: Option<&str>,
    token_program: &str,
) -> Result<Instruction> {
    let mut data = Vec::new();
    data.push(20);
    data.push(decimals);
    data.extend_from_slice(&decode_pubkey(mint_authority)?);
    data.extend(coption_pubkey(freeze_authority)?);
    Ok(Instruction {
        program_id: token_program.to_string(),
        accounts: vec![AccountMeta::writable(mint)],
        data,
    })
}

/// SPL Token / Token-2022 MintToChecked (opcode 14).
pub fn mint_to_checked(
    mint: &str,
    dest: &str,
    authority: &str,
    amount: u64,
    decimals: u8,
    token_program: &str,
) -> Instruction {
    let mut data = Vec::with_capacity(10);
    data.push(14);
    data.extend_from_slice(&encode_u64(amount));
    data.push(decimals);
    Instruction {
        program_id: token_program.to_string(),
        accounts: vec![
            AccountMeta::writable(mint),
            AccountMeta::writable(dest),
            AccountMeta::readonly_signer(authority),
        ],
        data,
    }
}

/// Token-2022 SetAuthority (opcode 6). Authority type 0 = mint tokens, 1 = freeze.
pub fn set_authority(
    account: &str,
    current: &str,
    new_authority: Option<&str>,
    authority_type: u8,
    token_program: &str,
) -> Result<Instruction> {
    let mut data = Vec::new();
    data.push(6);
    data.push(authority_type);
    data.extend(coption_pubkey(new_authority)?);
    Ok(Instruction {
        program_id: token_program.to_string(),
        accounts: vec![
            AccountMeta::writable(account),
            AccountMeta::readonly_signer(current),
        ],
        data,
    })
}

/// TransferFeeConfig TLV payload size.
pub const TRANSFER_FEE_CONFIG_LEN: usize = 108;

pub fn token_2022_mint_space(transfer_fee: bool) -> usize {
    let mut len = 82 + 1; // mint + AccountType
    if transfer_fee {
        len += 4 + TRANSFER_FEE_CONFIG_LEN;
    }
    len
}

/// Token-2022 MetadataPointerExtension prefix (TokenInstruction variant 39).
pub const METADATA_POINTER_IX: u8 = 39;

/// SHA256("spl_token_metadata_interface:initialize_account")[..8]
pub const TOKEN_METADATA_INITIALIZE: [u8; 8] = [210, 225, 30, 162, 88, 184, 77, 141];
/// SHA256("spl_token_metadata_interface:updating_field")[..8]
pub const TOKEN_METADATA_UPDATE_FIELD: [u8; 8] = [221, 233, 49, 45, 181, 202, 220, 200];
/// SHA256("spl_token_metadata_interface:update_the_authority")[..8]
pub const TOKEN_METADATA_UPDATE_AUTHORITY: [u8; 8] = [215, 228, 166, 228, 84, 100, 86, 123];

fn borsh_string(s: &str, out: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

/// Extra Token-2022 mint extensions beyond metadata (+ optional transfer fee).
#[derive(Clone, Copy, Default)]
pub struct MintSpace {
    pub transfer_fee: bool,
    pub close_authority: bool,
    pub permanent_delegate: bool,
    pub non_transferable: bool,
    pub default_frozen: bool,
    pub interest: bool,
}

// Token-2022 mints with extensions pad the 82-byte mint to 165 (token-account
// size), then AccountType, then TLVs. Docs: 82 + 83 pad + 1 type + extensions.
// getMintLen([MetadataPointer]) == 234.
const EXTENDED_MINT_BASE_LEN: usize = 165;
const ACCOUNT_TYPE_LEN: usize = 1;
const TLV_HEADER_LEN: usize = 4;
const METADATA_POINTER_LEN: usize = 64;

/// CreateAccount space for a Token-2022 mint with MetadataPointer and optional
/// fixed-size extras. Do not include TokenMetadata — InitializeMint2 requires
/// the account length to match the extensions already initialized.
pub fn token_2022_mint_create_space(flags: MintSpace) -> usize {
    let mut len = EXTENDED_MINT_BASE_LEN + ACCOUNT_TYPE_LEN;
    len += TLV_HEADER_LEN + METADATA_POINTER_LEN;
    if flags.transfer_fee {
        len += TLV_HEADER_LEN + TRANSFER_FEE_CONFIG_LEN;
    }
    if flags.close_authority {
        len += TLV_HEADER_LEN + 32;
    }
    if flags.permanent_delegate {
        len += TLV_HEADER_LEN + 32;
    }
    if flags.non_transferable {
        len += TLV_HEADER_LEN;
    }
    if flags.default_frozen {
        len += TLV_HEADER_LEN + 1;
    }
    if flags.interest {
        len += TLV_HEADER_LEN + 52;
    }
    len
}

pub fn token_2022_mint_space_meta(transfer_fee: bool) -> usize {
    token_2022_mint_create_space(MintSpace {
        transfer_fee,
        ..MintSpace::default()
    })
}

pub fn token_2022_mint_space_ext(flags: MintSpace) -> usize {
    token_2022_mint_create_space(flags)
}

/// Packed TokenMetadata TLV size (header + borsh payload) after initialize
/// plus any extra key/value fields written with UpdateField.
pub fn token_metadata_tlv_len(
    name: &str,
    symbol: &str,
    uri: &str,
    extra: &[(&str, &str)],
) -> usize {
    let mut payload = 32 + 32;
    payload += 4 + name.len();
    payload += 4 + symbol.len();
    payload += 4 + uri.len();
    payload += 4;
    for (key, val) in extra {
        payload += 4 + key.len() + 4 + val.len();
    }
    TLV_HEADER_LEN + payload
}

pub fn token_2022_mint_rent_space(
    flags: MintSpace,
    name: &str,
    symbol: &str,
    uri: &str,
    extra: &[(&str, &str)],
) -> usize {
    token_2022_mint_create_space(flags) + token_metadata_tlv_len(name, symbol, uri, extra)
}

/// Initialize MetadataPointer on an uninitialized mint. Call before InitializeMint2.
pub fn initialize_metadata_pointer(
    mint: &str,
    authority: &str,
    metadata_address: &str,
) -> Result<Instruction> {
    let mut data = Vec::new();
    data.push(METADATA_POINTER_IX);
    data.push(0); // Initialize
    data.extend_from_slice(&decode_pubkey(authority)?);
    data.extend_from_slice(&decode_pubkey(metadata_address)?);
    Ok(Instruction {
        program_id: ProgramIds::TOKEN_2022.to_string(),
        accounts: vec![AccountMeta::writable(mint)],
        data,
    })
}

/// Initialize Token-2022 TokenMetadata on the mint (self-pointing).
pub fn initialize_token_metadata(
    mint: &str,
    update_authority: &str,
    mint_authority: &str,
    name: &str,
    symbol: &str,
    uri: &str,
) -> Result<Instruction> {
    let mut data = TOKEN_METADATA_INITIALIZE.to_vec();
    borsh_string(name, &mut data);
    borsh_string(symbol, &mut data);
    borsh_string(uri, &mut data);
    Ok(Instruction {
        program_id: ProgramIds::TOKEN_2022.to_string(),
        accounts: vec![
            AccountMeta::writable(mint),
            AccountMeta::readonly(update_authority),
            AccountMeta::readonly(mint),
            AccountMeta::readonly_signer(mint_authority),
        ],
        data,
    })
}

/// Update a TokenMetadata field. `key` None = name/symbol/uri via `field` 0/1/2.
pub fn update_token_metadata_field(
    mint: &str,
    update_authority: &str,
    field: u8,
    key: Option<&str>,
    value: &str,
) -> Result<Instruction> {
    let mut data = TOKEN_METADATA_UPDATE_FIELD.to_vec();
    data.push(field);
    if field == 3 {
        borsh_string(key.unwrap_or(""), &mut data);
    }
    borsh_string(value, &mut data);
    Ok(Instruction {
        program_id: ProgramIds::TOKEN_2022.to_string(),
        accounts: vec![
            AccountMeta::writable(mint),
            AccountMeta::readonly_signer(update_authority),
        ],
        data,
    })
}

/// Set TokenMetadata update authority. None revokes (seals metadata).
pub fn update_token_metadata_authority(
    mint: &str,
    current: &str,
    new_authority: Option<&str>,
) -> Result<Instruction> {
    let mut data = TOKEN_METADATA_UPDATE_AUTHORITY.to_vec();
    data.extend_from_slice(&optional_nonzero_pubkey(new_authority)?);
    Ok(Instruction {
        program_id: ProgramIds::TOKEN_2022.to_string(),
        accounts: vec![
            AccountMeta::writable(mint),
            AccountMeta::readonly_signer(current),
        ],
        data,
    })
}

/// Token-2022 TransferFee initialize (tag 0x1A, sub 0). Call before InitializeMint2.
pub fn initialize_transfer_fee_config(
    mint: &str,
    config_authority: Option<&str>,
    withdraw_authority: Option<&str>,
    basis_points: u16,
    max_fee: u64,
) -> Result<Instruction> {
    let mut data = Vec::new();
    data.push(0x1A);
    data.push(0x00);
    data.extend_from_slice(&optional_nonzero_pubkey(config_authority)?);
    data.extend_from_slice(&optional_nonzero_pubkey(withdraw_authority)?);
    data.extend_from_slice(&basis_points.to_le_bytes());
    data.extend_from_slice(&max_fee.to_le_bytes());
    Ok(Instruction {
        program_id: ProgramIds::TOKEN_2022.to_string(),
        accounts: vec![AccountMeta::writable(mint)],
        data,
    })
}

/// InitializeMintCloseAuthority (opcode 25). Before InitializeMint2.
/// Close authority is a COption pubkey (4-byte tag + 32).
pub fn initialize_mint_close_authority(mint: &str, authority: &str) -> Result<Instruction> {
    let mut data = vec![25u8];
    data.extend(coption_pubkey(Some(authority))?);
    Ok(Instruction {
        program_id: ProgramIds::TOKEN_2022.to_string(),
        accounts: vec![AccountMeta::writable(mint)],
        data,
    })
}

/// InitializePermanentDelegate (opcode 35). Before InitializeMint2.
pub fn initialize_permanent_delegate(mint: &str, delegate: &str) -> Result<Instruction> {
    let mut data = vec![35u8];
    data.extend_from_slice(&decode_pubkey(delegate)?);
    Ok(Instruction {
        program_id: ProgramIds::TOKEN_2022.to_string(),
        accounts: vec![AccountMeta::writable(mint)],
        data,
    })
}

/// InitializeNonTransferableMint (opcode 32). Before InitializeMint2.
pub fn initialize_non_transferable_mint(mint: &str) -> Instruction {
    Instruction {
        program_id: ProgramIds::TOKEN_2022.to_string(),
        accounts: vec![AccountMeta::writable(mint)],
        data: vec![32u8],
    }
}

/// DefaultAccountState Initialize (opcode 28, sub 0). Frozen = 2. Before InitializeMint2.
pub fn initialize_default_account_state(mint: &str, state: u8) -> Instruction {
    Instruction {
        program_id: ProgramIds::TOKEN_2022.to_string(),
        accounts: vec![AccountMeta::writable(mint)],
        data: vec![28u8, 0, state],
    }
}

/// InterestBearingMint Initialize (opcode 33, sub 0). Rate is signed bps (100 = 1%).
pub fn initialize_interest_bearing(
    mint: &str,
    rate_authority: &str,
    rate_bps: i16,
) -> Result<Instruction> {
    let mut data = vec![33u8, 0];
    data.extend_from_slice(&decode_pubkey(rate_authority)?);
    data.extend_from_slice(&rate_bps.to_le_bytes());
    Ok(Instruction {
        program_id: ProgramIds::TOKEN_2022.to_string(),
        accounts: vec![AccountMeta::writable(mint)],
        data,
    })
}

pub fn stake_initialize(stake: &str, staker: &str, withdrawer: &str) -> Result<Instruction> {
    let mut data = Vec::new();
    data.extend_from_slice(&encode_u32(0));
    data.extend_from_slice(&decode_pubkey(staker)?);
    data.extend_from_slice(&decode_pubkey(withdrawer)?);
    data.extend_from_slice(&[0u8; 48]);
    Ok(Instruction {
        program_id: ProgramIds::STAKE.to_string(),
        accounts: vec![
            AccountMeta::writable(stake),
            AccountMeta::readonly(ProgramIds::SYSVAR_RENT),
        ],
        data,
    })
}

pub fn stake_delegate(stake: &str, vote: &str, authority: &str) -> Instruction {
    Instruction {
        program_id: ProgramIds::STAKE.to_string(),
        accounts: vec![
            AccountMeta::writable(stake),
            AccountMeta::readonly(vote),
            AccountMeta::readonly(ProgramIds::SYSVAR_CLOCK),
            AccountMeta::readonly(ProgramIds::SYSVAR_STAKE_HISTORY),
            AccountMeta::readonly(ProgramIds::STAKE_CONFIG),
            AccountMeta::readonly_signer(authority),
        ],
        data: encode_u32(2).to_vec(),
    }
}

pub fn stake_deactivate(stake: &str, authority: &str) -> Instruction {
    Instruction {
        program_id: ProgramIds::STAKE.to_string(),
        accounts: vec![
            AccountMeta::writable(stake),
            AccountMeta::readonly(ProgramIds::SYSVAR_CLOCK),
            AccountMeta::readonly_signer(authority),
        ],
        data: encode_u32(5).to_vec(),
    }
}

pub fn stake_withdraw(stake: &str, to: &str, authority: &str, lamports: u64) -> Instruction {
    let mut data = Vec::new();
    data.extend_from_slice(&encode_u32(4));
    data.extend_from_slice(&encode_u64(lamports));
    Instruction {
        program_id: ProgramIds::STAKE.to_string(),
        accounts: vec![
            AccountMeta::writable(stake),
            AccountMeta::writable(to),
            AccountMeta::readonly(ProgramIds::SYSVAR_CLOCK),
            AccountMeta::readonly(ProgramIds::SYSVAR_STAKE_HISTORY),
            AccountMeta::readonly_signer(authority),
        ],
        data,
    }
}

pub fn compile_legacy_message(
    fee_payer: &str,
    recent_blockhash: &str,
    ixs: &[Instruction],
) -> Result<Vec<u8>> {
    if ixs.is_empty() {
        return Err(Error::msg("no instructions"));
    }
    #[derive(Clone)]
    struct Entry {
        pubkey: String,
        is_signer: bool,
        is_writable: bool,
    }
    let mut accounts: Vec<Entry> = Vec::new();
    let push = |accounts: &mut Vec<Entry>, pk: &str, signer: bool, writable: bool| {
        if let Some(e) = accounts.iter_mut().find(|e| e.pubkey == pk) {
            e.is_signer |= signer;
            e.is_writable |= writable;
        } else {
            accounts.push(Entry {
                pubkey: pk.to_string(),
                is_signer: signer,
                is_writable: writable,
            });
        }
    };
    push(&mut accounts, fee_payer, true, true);
    for ix in ixs {
        push(&mut accounts, &ix.program_id, false, false);
        for m in &ix.accounts {
            push(&mut accounts, &m.pubkey, m.is_signer, m.is_writable);
        }
    }

    let mut writable_signers = Vec::new();
    let mut readonly_signers = Vec::new();
    let mut writable_nonsigners = Vec::new();
    let mut readonly_nonsigners = Vec::new();
    for e in accounts.iter().filter(|e| e.pubkey != fee_payer) {
        match (e.is_signer, e.is_writable) {
            (true, true) => writable_signers.push(e.clone()),
            (true, false) => readonly_signers.push(e.clone()),
            (false, true) => writable_nonsigners.push(e.clone()),
            (false, false) => readonly_nonsigners.push(e.clone()),
        }
    }

    let mut ordered = Vec::new();
    ordered.push(
        accounts
            .iter()
            .find(|e| e.pubkey == fee_payer)
            .cloned()
            .unwrap(),
    );
    ordered.extend(writable_signers);
    let n_ro_signed = readonly_signers.len();
    ordered.extend(readonly_signers);
    ordered.extend(writable_nonsigners);
    let n_ro_unsigned = readonly_nonsigners.len();
    ordered.extend(readonly_nonsigners);

    let n_signers = ordered.iter().filter(|e| e.is_signer).count();
    let mut index = std::collections::HashMap::new();
    let mut keys_bytes = Vec::new();
    for (i, e) in ordered.iter().enumerate() {
        index.insert(e.pubkey.clone(), i as u8);
        keys_bytes.push(decode_pubkey(&e.pubkey)?);
    }
    let blockhash = decode_pubkey(recent_blockhash).map_err(|_| Error::msg("invalid blockhash"))?;

    let mut out = Vec::new();
    out.push(n_signers as u8);
    out.push(n_ro_signed as u8);
    out.push(n_ro_unsigned as u8);
    compact_u16_encode(ordered.len() as u16, &mut out);
    for k in &keys_bytes {
        out.extend_from_slice(k);
    }
    out.extend_from_slice(&blockhash);
    compact_u16_encode(ixs.len() as u16, &mut out);
    for ix in ixs {
        let prog = *index
            .get(&ix.program_id)
            .ok_or_else(|| Error::msg("program id missing"))?;
        out.push(prog);
        compact_u16_encode(ix.accounts.len() as u16, &mut out);
        for m in &ix.accounts {
            let idx = *index
                .get(&m.pubkey)
                .ok_or_else(|| Error::msg("account missing"))?;
            out.push(idx);
        }
        compact_u16_encode(ix.data.len() as u16, &mut out);
        out.extend_from_slice(&ix.data);
    }
    Ok(out)
}

/// Legacy transaction with zeroed signatures for `simulateTransaction` (sigVerify=false).
pub fn unsigned_legacy(
    fee_payer: &str,
    recent_blockhash: &str,
    ixs: &[Instruction],
) -> Result<Vec<u8>> {
    let message = compile_legacy_message(fee_payer, recent_blockhash, ixs)?;
    let n_signers = message.first().copied().unwrap_or(0) as usize;
    let mut out = Vec::new();
    compact_u16_encode(n_signers as u16, &mut out);
    out.extend(std::iter::repeat(0u8).take(n_signers.saturating_mul(64)));
    out.extend_from_slice(&message);
    Ok(out)
}

pub fn build_signed_legacy(
    fee_payer: &str,
    recent_blockhash: &str,
    ixs: &[Instruction],
    signers: &[&Keypair],
) -> Result<Vec<u8>> {
    let pubs: Vec<[u8; 32]> = signers.iter().map(|k| k.public_bytes()).collect();
    build_signed_legacy_with(fee_payer, recent_blockhash, ixs, &pubs, |i, msg| {
        Ok(signers[i].sign(msg))
    })
}

pub fn build_signed_legacy_with<F>(
    fee_payer: &str,
    recent_blockhash: &str,
    ixs: &[Instruction],
    signer_pubs: &[[u8; 32]],
    mut sign: F,
) -> Result<Vec<u8>>
where
    F: FnMut(usize, &[u8]) -> Result<[u8; 64]>,
{
    let message = compile_legacy_message(fee_payer, recent_blockhash, ixs)?;
    let n_signers = message[0] as usize;
    if signer_pubs.len() != n_signers {
        return Err(Error::msg(format!(
            "need {n_signers} signatures, got {}",
            signer_pubs.len()
        )));
    }
    let mut out = Vec::new();
    compact_u16_encode(n_signers as u16, &mut out);
    for i in 0..n_signers {
        out.extend_from_slice(&sign(i, &message)?);
    }
    out.extend_from_slice(&message);
    Ok(out)
}

fn message_signer_keys(message: &[u8], n_sigs: usize) -> Result<Vec<[u8; 32]>> {
    if message.is_empty() {
        return Err(Error::msg("transaction message truncated"));
    }
    let mut offset = if message[0] & 0x80 != 0 { 1 } else { 0 };
    if offset + 3 > message.len() {
        return Err(Error::msg("transaction message truncated"));
    }
    let n_required = message[offset] as usize;
    offset += 3;
    let n_keys = compact_u16_decode(message, &mut offset)? as usize;
    let take = n_sigs.min(n_required).min(n_keys);
    if take == 0 {
        return Err(Error::msg("transaction has no signers"));
    }
    let mut keys = Vec::with_capacity(take);
    for _ in 0..take {
        if offset + 32 > message.len() {
            return Err(Error::msg("transaction keys truncated"));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&message[offset..offset + 32]);
        keys.push(key);
        offset += 32;
    }
    Ok(keys)
}

/// Sign a serialized (legacy or v0) transaction in place.
///
/// Jupiter DFlow / RFQ orders often have more than one signature slot. The
/// wallet must sign the slot that matches its pubkey — not always slot 0.
pub fn sign_wire_transaction(tx: &mut [u8], keypair: &Keypair) -> Result<()> {
    let pk = keypair.public_bytes();
    sign_wire_transaction_with(tx, &pk, |msg| Ok(keypair.sign(msg)))
}

pub fn sign_wire_transaction_with<F>(
    tx: &mut [u8],
    pubkey: &[u8; 32],
    mut sign: F,
) -> Result<()>
where
    F: FnMut(&[u8]) -> Result<[u8; 64]>,
{
    let mut offset = 0;
    let n_sigs = compact_u16_decode(tx, &mut offset)? as usize;
    if n_sigs == 0 {
        return Err(Error::msg("transaction has no signature slots"));
    }
    let sigs_start = offset;
    let msg_start = offset + n_sigs * 64;
    if msg_start > tx.len() {
        return Err(Error::msg("transaction truncated"));
    }
    let message = tx[msg_start..].to_vec();
    let keys = message_signer_keys(&message, n_sigs)?;
    let slot = keys
        .iter()
        .position(|k| k == pubkey)
        .ok_or_else(|| Error::msg("this wallet is not a signer on the swap transaction"))?;
    let sig = sign(&message)?;
    let start = sigs_start + slot * 64;
    let end = start + 64;
    if end > msg_start {
        return Err(Error::msg("transaction signature slot out of range"));
    }
    tx[start..end].copy_from_slice(&sig);
    Ok(())
}

pub fn first_signature(tx: &[u8]) -> Result<[u8; 64]> {
    let mut offset = 0;
    let n_sigs = compact_u16_decode(tx, &mut offset)? as usize;
    if n_sigs == 0 {
        return Err(Error::msg("transaction has no signature slots"));
    }
    let end = offset + 64;
    if end > tx.len() {
        return Err(Error::msg("transaction truncated"));
    }
    let mut sig = [0u8; 64];
    sig.copy_from_slice(&tx[offset..end]);
    Ok(sig)
}

pub fn decode_b64(text: &str) -> Result<Vec<u8>> {
    let mut raw = text.trim().to_string();
    while raw.len() % 4 != 0 {
        raw.push('=');
    }
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, raw.as_bytes())
        .map_err(|e| Error::msg(e.to_string()))
}

/// Sign a base64 wire transaction. Returns (signature base58, signed tx base64, bytes).
pub fn sign_wire_b64(tx_b64: &str, keypair: &Keypair) -> Result<(String, String, Vec<u8>)> {
    let pk = keypair.public_bytes();
    sign_wire_b64_with(tx_b64, &pk, |msg| Ok(keypair.sign(msg)))
}

pub fn sign_wire_b64_with<F>(
    tx_b64: &str,
    pubkey: &[u8; 32],
    sign: F,
) -> Result<(String, String, Vec<u8>)>
where
    F: FnMut(&[u8]) -> Result<[u8; 64]>,
{
    let mut bytes = decode_b64(tx_b64)?;
    sign_wire_transaction_with(&mut bytes, pubkey, sign)?;
    let sig = first_signature(&bytes)?;
    let sig_b58 = bs58::encode(sig).into_string();
    let signed_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    Ok((sig_b58, signed_b64, bytes))
}

/// Sign a WalletConnect solana_signMessage payload (base58 bytes, else UTF-8).
pub fn sign_message_b58(message: &str, keypair: &Keypair) -> Result<String> {
    let text = message.trim();
    let bytes = match bs58::decode(text).into_vec() {
        Ok(raw) if !raw.is_empty() => raw,
        _ => text.as_bytes().to_vec(),
    };
    Ok(sign_message_bytes(&bytes, keypair))
}

/// Sign exact message bytes (plugin Wallet Standard / SIWS).
pub fn sign_message_bytes(bytes: &[u8], keypair: &Keypair) -> String {
    bs58::encode(keypair.sign(bytes)).into_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keypair::Keypair;

    #[test]
    fn sign_message_bytes_is_64() {
        let kp = Keypair::generate();
        let sig = sign_message_bytes(b"hello pump", &kp);
        let raw = bs58::decode(&sig).into_vec().unwrap();
        assert_eq!(raw.len(), 64);
    }

    #[test]
    fn unsigned_has_zero_sigs() {
        let from = Keypair::generate();
        let to = Keypair::generate();
        let bh = Keypair::generate().address();
        let ix = system_transfer(&from.address(), &to.address(), 1_000);
        let blob = unsigned_legacy(&from.address(), &bh, &[ix]).unwrap();
        let mut off = 0;
        let n = compact_u16_decode(&blob, &mut off).unwrap();
        assert_eq!(n, 1);
        assert!(blob[off..off + 64].iter().all(|b| *b == 0));
    }

    #[test]
    fn transfer_builds() {
        let from = Keypair::generate();
        let to = Keypair::generate();
        let bh = Keypair::generate().address();
        let ix = system_transfer(&from.address(), &to.address(), 1_000);
        let tx = build_signed_legacy(&from.address(), &bh, &[ix], &[&from]).unwrap();
        assert!(tx.len() > 64);
        let mut off = 0;
        let n = compact_u16_decode(&tx, &mut off).unwrap();
        assert_eq!(n, 1);
        let sig = first_signature(&tx).unwrap();
        assert_eq!(sig.len(), 64);
    }

    #[test]
    fn memo_and_budget_encode() {
        let kp = Keypair::generate();
        let note = memo(&kp.address(), "hello vibe");
        assert_eq!(note.program_id, ProgramIds::MEMO);
        assert_eq!(note.data, b"hello vibe");
        assert_eq!(note.accounts.len(), 1);
        let limit = set_compute_unit_limit(80_000);
        assert_eq!(limit.program_id, ProgramIds::COMPUTE_BUDGET);
        assert_eq!(limit.data[0], 2);
        assert_eq!(&limit.data[1..5], &80_000u32.to_le_bytes());
        let tip = set_compute_unit_price(25_000);
        assert_eq!(tip.data[0], 3);
        assert_eq!(&tip.data[1..9], &25_000u64.to_le_bytes());
    }

    #[test]
    fn token_account_amount_and_withheld() {
        let mut data = vec![0u8; TOKEN_ACCOUNT_LEN + 1];
        data[64..72].copy_from_slice(&42u64.to_le_bytes());
        data[108] = 1;
        data[TOKEN_ACCOUNT_LEN] = 1;
        assert_eq!(token_account_amount(&data), Some(42));
        assert!(!token_account_frozen(&data));
        assert_eq!(token_account_withheld(&data), 0);
        data[108] = 2;
        assert!(token_account_frozen(&data));
        data.extend_from_slice(&2u16.to_le_bytes());
        data.extend_from_slice(&8u16.to_le_bytes());
        data.extend_from_slice(&7u64.to_le_bytes());
        assert_eq!(token_account_withheld(&data), 7);
        let harvest = harvest_withheld_to_mint("mint", "acct", ProgramIds::TOKEN_2022);
        assert_eq!(harvest.program_id, ProgramIds::TOKEN_2022);
        assert_eq!(harvest.data, vec![26, 3]);
        assert_eq!(harvest.accounts.len(), 2);
    }

    #[test]
    fn token_burn_and_close_encode() {
        let owner = Keypair::generate();
        let mint = Keypair::generate();
        let acct = Keypair::generate();
        let burn = token_burn(
            &acct.address(),
            &mint.address(),
            &owner.address(),
            1,
            ProgramIds::TOKEN,
        );
        assert_eq!(burn.program_id, ProgramIds::TOKEN);
        assert_eq!(burn.data[0], 8);
        assert_eq!(&burn.data[1..9], &1u64.to_le_bytes());
        assert_eq!(burn.accounts.len(), 3);
        let close = token_close_account(
            &acct.address(),
            &owner.address(),
            &owner.address(),
            ProgramIds::TOKEN_2022,
        );
        assert_eq!(close.program_id, ProgramIds::TOKEN_2022);
        assert_eq!(close.data, vec![9]);
        assert_eq!(close.accounts.len(), 3);
    }

    #[test]
    fn token_transfer_checked_encodes() {
        let from = Keypair::generate();
        let mint = Keypair::generate();
        let dest = Keypair::generate();
        let src = Keypair::generate();
        let ix = token_transfer_checked(
            &src.address(),
            &mint.address(),
            &dest.address(),
            &from.address(),
            1_500_000,
            6,
            ProgramIds::TOKEN,
        );
        assert_eq!(ix.program_id, ProgramIds::TOKEN);
        assert_eq!(ix.data[0], 12);
        assert_eq!(&ix.data[1..9], &1_500_000u64.to_le_bytes());
        assert_eq!(ix.data[9], 6);
        assert_eq!(ix.accounts.len(), 4);
    }

    #[test]
    fn ata_is_stable_and_program_specific() {
        let owner = Keypair::generate().address();
        let mint = Keypair::generate().address();
        let a = associated_token_address(&owner, &mint, ProgramIds::TOKEN).unwrap();
        let b = associated_token_address(&owner, &mint, ProgramIds::TOKEN).unwrap();
        assert_eq!(a, b);
        assert_eq!(decode_pubkey(&a).unwrap().len(), 32);
        let t22 = associated_token_address(&owner, &mint, ProgramIds::TOKEN_2022).unwrap();
        assert_ne!(a, t22);
        let other = associated_token_address(&Keypair::generate().address(), &mint, ProgramIds::TOKEN)
            .unwrap();
        assert_ne!(a, other);
    }

    #[test]
    fn sign_wire_matches_fee_payer_slot() {
        let from = Keypair::generate();
        let to = Keypair::generate();
        let bh = Keypair::generate().address();
        let ix = system_transfer(&from.address(), &to.address(), 1_000);
        let mut tx = build_signed_legacy(&from.address(), &bh, &[ix], &[&from]).unwrap();
        let mut off = 0;
        let n = compact_u16_decode(&tx, &mut off).unwrap() as usize;
        assert_eq!(n, 1);
        for b in &mut tx[off..off + 64] {
            *b = 0;
        }
        sign_wire_transaction(&mut tx, &from).unwrap();
        let sig = first_signature(&tx).unwrap();
        let msg_start = off + 64;
        assert!(Keypair::verify(&from.public_bytes(), &tx[msg_start..], &sig));
    }

    #[test]
    fn sign_wire_fills_second_signer_slot() {
        let fee = Keypair::generate();
        let user = Keypair::generate();
        let mut msg = Vec::new();
        msg.push(0x80); // v0
        msg.push(2); // required signatures
        msg.push(0);
        msg.push(0);
        compact_u16_encode(2, &mut msg);
        msg.extend_from_slice(&fee.public_bytes());
        msg.extend_from_slice(&user.public_bytes());
        msg.extend_from_slice(&[7u8; 32]); // blockhash
        compact_u16_encode(0, &mut msg); // instructions
        compact_u16_encode(0, &mut msg); // lookups
        let mut tx = Vec::new();
        compact_u16_encode(2, &mut tx);
        tx.extend_from_slice(&[1u8; 64]); // existing first signature
        tx.extend_from_slice(&[0u8; 64]);
        tx.extend_from_slice(&msg);
        let first = tx[1..65].to_vec();
        sign_wire_transaction(&mut tx, &user).unwrap();
        assert_eq!(&tx[1..65], first.as_slice());
        let mut user_sig = [0u8; 64];
        user_sig.copy_from_slice(&tx[65..129]);
        assert!(Keypair::verify(&user.public_bytes(), &msg, &user_sig));
        assert_ne!(user_sig, [0u8; 64]);
    }

    #[test]
    fn sign_wire_rejects_foreign_wallet() {
        let from = Keypair::generate();
        let to = Keypair::generate();
        let other = Keypair::generate();
        let bh = Keypair::generate().address();
        let ix = system_transfer(&from.address(), &to.address(), 1_000);
        let mut tx = build_signed_legacy(&from.address(), &bh, &[ix], &[&from]).unwrap();
        let err = sign_wire_transaction(&mut tx, &other).unwrap_err().to_string();
        assert!(err.contains("not a signer"));
    }

    #[test]
    fn nonce_account_roundtrip() {
        let auth = Keypair::generate();
        let nonce = Keypair::generate();
        let mut data = [0u8; 80];
        data[4..8].copy_from_slice(&1u32.to_le_bytes());
        data[8..40].copy_from_slice(&auth.public_bytes());
        data[40..72].copy_from_slice(&nonce.public_bytes());
        data[72..80].copy_from_slice(&5000u64.to_le_bytes());
        let parsed = NonceAccount::from_account_data(&data).unwrap();
        assert!(parsed.is_initialized());
        assert_eq!(parsed.authority, auth.address());
        assert_eq!(parsed.nonce, nonce.address());
        let ix = nonce_advance(&nonce.address(), &auth.address());
        assert_eq!(ix.data, 4u32.to_le_bytes());
        assert_eq!(ix.accounts[2].pubkey, auth.address());
        assert_eq!(token_2022_mint_space(false), 83);
        assert!(token_2022_mint_space(true) > token_2022_mint_space(false));
    }

    #[test]
    fn initialize_mint2_encodes() {
        let mint = Keypair::generate();
        let auth = Keypair::generate();
        let ix = initialize_mint2(
            &mint.address(),
            6,
            &auth.address(),
            None,
            ProgramIds::TOKEN_2022,
        )
        .unwrap();
        assert_eq!(ix.data[0], 20);
        assert_eq!(ix.data[1], 6);
        assert_eq!(&ix.data[2..34], &auth.public_bytes());
        assert_eq!(ix.data[34], 0);
        assert_eq!(ix.data.len(), 35);
        let some = initialize_mint2(
            &mint.address(),
            6,
            &auth.address(),
            Some(&auth.address()),
            ProgramIds::TOKEN_2022,
        )
        .unwrap();
        assert_eq!(some.data[34], 1);
        assert_eq!(&some.data[35..67], &auth.public_bytes());
        let revoke = set_authority(
            &mint.address(),
            &auth.address(),
            None,
            1,
            ProgramIds::TOKEN_2022,
        )
        .unwrap();
        assert_eq!(revoke.data, vec![6, 1, 0]);
    }

    #[test]
    fn metadata_pointer_and_token_metadata_encode() {
        let mint = Keypair::generate();
        let auth = Keypair::generate();
        let ptr = initialize_metadata_pointer(&mint.address(), &auth.address(), &mint.address())
            .unwrap();
        assert_eq!(ptr.data[0], METADATA_POINTER_IX);
        assert_eq!(ptr.data[1], 0);
        assert_eq!(&ptr.data[2..34], &auth.public_bytes());
        assert_eq!(&ptr.data[34..66], &mint.public_bytes());
        let meta = initialize_token_metadata(
            &mint.address(),
            &auth.address(),
            &auth.address(),
            "Vibe",
            "VIBE",
            "https://example.com/meta.json",
        )
        .unwrap();
        assert_eq!(&meta.data[..8], &TOKEN_METADATA_INITIALIZE);
        assert_eq!(meta.accounts.len(), 4);
        assert!(meta.accounts[3].is_signer);
        let field = update_token_metadata_field(
            &mint.address(),
            &auth.address(),
            3,
            Some("twitter"),
            "https://x.com/VibeWalletSol",
        )
        .unwrap();
        assert_eq!(&field.data[..8], &TOKEN_METADATA_UPDATE_FIELD);
        assert_eq!(field.data[8], 3);
        let seal =
            update_token_metadata_authority(&mint.address(), &auth.address(), None).unwrap();
        assert_eq!(&seal.data[..8], &TOKEN_METADATA_UPDATE_AUTHORITY);
        assert_eq!(&seal.data[8..], &[0u8; 32]);
        assert_eq!(
            token_2022_mint_create_space(MintSpace::default()),
            165 + 1 + 4 + 64
        );
        assert!(
            token_metadata_tlv_len("Vibe", "VIBE", "https://example.com/meta.json", &[])
                > 4 + 64
        );
        assert!(
            token_2022_mint_rent_space(
                MintSpace::default(),
                "Vibe",
                "VIBE",
                "https://example.com/meta.json",
                &[("twitter", "https://x.com/VibeWalletSol")],
            ) > token_2022_mint_create_space(MintSpace::default())
        );
        let close = initialize_mint_close_authority(&mint.address(), &auth.address()).unwrap();
        assert_eq!(close.data[0], 25);
        assert_eq!(close.data[1], 1);
        assert_eq!(&close.data[2..34], &auth.public_bytes());
        assert_eq!(close.data.len(), 1 + 1 + 32);
        let thaw = token_thaw_account(
            &mint.address(),
            &mint.address(),
            &auth.address(),
            ProgramIds::TOKEN_2022,
        );
        assert_eq!(thaw.data, vec![11]);
        let del = initialize_permanent_delegate(&mint.address(), &auth.address()).unwrap();
        assert_eq!(del.data[0], 35);
        let soul = initialize_non_transferable_mint(&mint.address());
        assert_eq!(soul.data[0], 32);
        let frozen = initialize_default_account_state(&mint.address(), 2);
        assert_eq!(frozen.data, vec![28, 0, 2]);
        let pda = metaplex_metadata_pda(&mint.address()).unwrap();
        assert_ne!(pda, mint.address());
        let mpl = create_metadata_account_v3(
            &mint.address(),
            &auth.address(),
            &auth.address(),
            &auth.address(),
            "Vibe Coin",
            "VIBE",
            "https://example.com/meta.json",
            true,
        )
        .unwrap();
        assert_eq!(mpl.data[0], 33);
        assert_eq!(mpl.program_id, ProgramIds::METAPLEX_METADATA);
        assert_eq!(mpl.accounts[0].pubkey, pda);
        assert_eq!(mpl.accounts.len(), 7);
        let seal = revoke_metaplex_mutable(&mint.address(), &auth.address()).unwrap();
        assert_eq!(seal.data, vec![15, 0, 0, 0, 1, 0]);
        assert_eq!(SPL_MINT_LEN, 82);
        let rate = initialize_interest_bearing(&mint.address(), &auth.address(), 500).unwrap();
        assert_eq!(rate.data[0], 33);
        assert!(
            token_2022_mint_space_ext(MintSpace {
                transfer_fee: true,
                close_authority: true,
                permanent_delegate: true,
                non_transferable: true,
                default_frozen: true,
                interest: true,
            }) > token_2022_mint_space_meta(true)
        );
    }
}

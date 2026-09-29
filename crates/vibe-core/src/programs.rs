//! Well-known Solana program IDs.

pub struct ProgramIds;

impl ProgramIds {
    pub const SYSTEM: &'static str = "11111111111111111111111111111111";
    pub const COMPUTE_BUDGET: &'static str = "ComputeBudget111111111111111111111111111111";
    pub const TOKEN: &'static str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
    pub const TOKEN_2022: &'static str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
    pub const ASSOCIATED_TOKEN: &'static str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
    pub const MEMO: &'static str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
    pub const ADDRESS_LOOKUP_TABLE: &'static str = "AddressLookupTab1e1111111111111111111111111";
    pub const STAKE: &'static str = "Stake11111111111111111111111111111111111111";
    pub const VOTE: &'static str = "Vote111111111111111111111111111111111111111";
    pub const METAPLEX_METADATA: &'static str = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";
    pub const BPF_LOADER: &'static str = "BPFLoader2111111111111111111111111111111111";
    pub const BPF_UPGRADEABLE_LOADER: &'static str = "BPFLoaderUpgradeab1e11111111111111111111111";
    pub const SYSVAR_RECENT_BLOCKHASHES: &'static str = "SysvarRecentB1ockHashes11111111111111111111";
    pub const SYSVAR_RENT: &'static str = "SysvarRent111111111111111111111111111111111";
    pub const SYSVAR_CLOCK: &'static str = "SysvarC1ock11111111111111111111111111111111";
    pub const SYSVAR_STAKE_HISTORY: &'static str = "SysvarStakeHistory1111111111111111111111111";
    pub const STAKE_CONFIG: &'static str = "StakeConfig11111111111111111111111111111111";

    pub fn is_token_program(program_id: &str) -> bool {
        program_id == Self::TOKEN || program_id == Self::TOKEN_2022
    }

    pub fn is_token_2022(program_id: &str) -> bool {
        program_id == Self::TOKEN_2022
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_helpers() {
        assert!(ProgramIds::is_token_program(ProgramIds::TOKEN));
        assert!(ProgramIds::is_token_program(ProgramIds::TOKEN_2022));
        assert!(ProgramIds::is_token_2022(ProgramIds::TOKEN_2022));
        assert!(!ProgramIds::is_token_2022(ProgramIds::TOKEN));
        assert!(!ProgramIds::is_token_program(ProgramIds::SYSTEM));
    }
}

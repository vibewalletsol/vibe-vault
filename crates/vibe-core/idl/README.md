# Bundled Anchor IDLs (display only)

Instruction names, discriminators and account names for ten common Solana programs, used by
the vault Confirm (`vibe_core::anchor_idl`) when a program has no on-chain IDL or the RPC
can't fetch it. They never affect what is signed or any txcheck rule.

Source: Cinder `resources/idl/` (https://github.com/gregnb/cinder, MIT License,
Copyright (c) 2026 Gregory Nowakowski), minified. The interfaces themselves describe the
public on-chain programs they are named after (Jupiter v6, Raydium CLMM / CP-Swap, Meteora
DLMM, Drift v2, Marinade, Tensor TSwap, Magic Eden M2, Squads v4, Pump.fun).

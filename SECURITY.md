# Security policy

Please report vulnerabilities in vibe-vault or VIBE Wallet privately by email
to **vibewalletsol@gmail.com**. Do not open a public issue for a security
problem. We will acknowledge your report and keep you updated on the fix.

Never send seed phrases, private keys or wallet files to anyone, including us.

## Release signing key

Release checksums (`SHA256SUMS`) are signed with the Old Growth Crypto release
signing key:

    58B0 78BE 5C1D B227 8043  67A7 28EB 45CC 198C 2D3D

The public key is attached to each VIBE Wallet release as
`vibewallet-release-signing-key.asc`
(https://github.com/vibewalletsol/VibeWallet/releases). Only trust binaries
whose `SHA256SUMS` carries a good signature from this fingerprint.

// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 Old Growth Crypto

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Msg(String),
    #[error("wrong password")]
    WrongPassword,
    #[error("invalid mnemonic")]
    InvalidMnemonic,
    #[error("invalid address")]
    InvalidAddress,
    #[error("rpc: {0}")]
    Rpc(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub fn msg(m: impl Into<String>) -> Self {
        Error::Msg(m.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

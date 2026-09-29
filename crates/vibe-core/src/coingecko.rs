//! CoinGecko client — same endpoints the Portfolio app uses
//! (`/coins/markets`, `/coins/solana/contract/{mint}`, icon download).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::format::parse_raw_u64;
use crate::rpc::TokenBalance;
use crate::tokens;
use crate::{LAMPORTS_PER_SOL, WSOL_MINT};

const UA: &str = "VIBEWallet/1.0 (GTK4; +https://oldgrowthcrypto.com)";
const DEMO_BASE: &str = "https://api.coingecko.com/api/v3";
const PRO_BASE: &str = "https://pro-api.coingecko.com/api/v3";

#[derive(Clone, Debug)]
pub struct Client {
    api_key: String,
    pro: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Market {
    pub coin_id: String,
    pub name: String,
    pub symbol: String,
    pub price: f64,
    pub change_1h: Option<f64>,
    pub change_24h: Option<f64>,
    pub change_7d: Option<f64>,
    pub image: String,
    pub volume_24h: Option<f64>,
    pub market_cap: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Holding {
    pub mint: String,
    pub coin_id: String,
    pub name: String,
    pub symbol: String,
    pub amount_raw: u64,
    pub decimals: u8,
    pub ui_amount: String,
    pub price_usd: f64,
    pub value_usd: f64,
    pub change_1h: Option<f64>,
    pub change_24h: Option<f64>,
    pub change_7d: Option<f64>,
    pub image_url: String,
    pub icon_path: Option<PathBuf>,
}

impl Client {
    pub fn new(api_key: impl Into<String>, pro: bool) -> Self {
        Self {
            api_key: api_key.into().trim().to_string(),
            pro,
        }
    }

    fn base(&self) -> &'static str {
        if self.pro && !self.api_key.is_empty() {
            PRO_BASE
        } else {
            DEMO_BASE
        }
    }

    fn get(&self, url: &str) -> Result<Value> {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(25))
            .user_agent(UA)
            .build();
        let mut req = agent.get(url).set("Accept", "application/json");
        if !self.api_key.is_empty() {
            if self.pro {
                req = req.set("x-cg-pro-api-key", &self.api_key);
            } else {
                req = req.set("x-cg-demo-api-key", &self.api_key);
            }
        }
        let resp = req.call().map_err(|e| Error::msg(format!("coingecko: {e}")))?;
        resp.into_json()
            .map_err(|e| Error::msg(format!("coingecko json: {e}")))
    }

    pub fn fetch_markets(&self, ids: &[String]) -> Result<HashMap<String, Market>> {
        let mut unique = Vec::new();
        for id in ids {
            let id = id.trim().to_lowercase();
            if id.is_empty() || unique.contains(&id) {
                continue;
            }
            unique.push(id);
        }
        let mut out = HashMap::new();
        for chunk in unique.chunks(80) {
            let url = format!(
                "{}/coins/markets?vs_currency=usd&ids={}&order=market_cap_desc&per_page={}&page=1&sparkline=false&price_change_percentage=1h,24h,7d&locale=en",
                self.base(),
                chunk.join("%2C"),
                chunk.len()
            );
            let v = self.get(&url)?;
            let Some(arr) = v.as_array() else { continue };
            for row in arr {
                let Some(id) = row.get("id").and_then(|s| s.as_str()) else {
                    continue;
                };
                let price = row.get("current_price").and_then(|n| n.as_f64()).unwrap_or(0.0);
                if price < 0.0 {
                    continue;
                }
                out.insert(
                    id.to_string(),
                    Market {
                        coin_id: id.to_string(),
                        name: row
                            .get("name")
                            .and_then(|s| s.as_str())
                            .unwrap_or(id)
                            .to_string(),
                        symbol: row
                            .get("symbol")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_uppercase(),
                        price,
                        change_1h: row
                            .get("price_change_percentage_1h_in_currency")
                            .and_then(|n| n.as_f64()),
                        change_24h: row
                            .get("price_change_percentage_24h_in_currency")
                            .or_else(|| row.get("price_change_percentage_24h"))
                            .and_then(|n| n.as_f64()),
                        change_7d: row
                            .get("price_change_percentage_7d_in_currency")
                            .and_then(|n| n.as_f64()),
                        image: row
                            .get("image")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string(),
                        volume_24h: row.get("total_volume").and_then(|n| n.as_f64()),
                        market_cap: row.get("market_cap").and_then(|n| n.as_f64()),
                    },
                );
            }
        }
        Ok(out)
    }

    pub fn lookup_solana_mint(&self, mint: &str) -> Option<(String, String, String, String)> {
        // id, symbol, name, image
        let mint = mint.trim();
        if mint.is_empty() {
            return None;
        }
        if let Some(hit) = cache_get(mint) {
            return Some(hit);
        }
        let url = format!("{}/coins/solana/contract/{mint}", self.base());
        let v = self.get(&url).ok()?;
        let id = v.get("id")?.as_str()?.to_string();
        let symbol = v
            .get("symbol")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_uppercase();
        let name = v
            .get("name")
            .and_then(|s| s.as_str())
            .unwrap_or(&id)
            .to_string();
        let image = v
            .get("image")
            .and_then(|img| {
                img.get("small")
                    .or_else(|| img.get("large"))
                    .or_else(|| img.get("thumb"))
                    .and_then(|s| s.as_str())
            })
            .unwrap_or("")
            .to_string();
        cache_put(mint, &id, &symbol, &name, &image);
        Some((id, symbol, name, image))
    }

    pub fn download_icon(&self, coin_id: &str, url: &str) -> Option<PathBuf> {
        if coin_id.is_empty() || url.is_empty() {
            return None;
        }
        let path = icon_path(coin_id, url);
        if path.exists() && path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            return Some(path);
        }
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(20))
            .user_agent(UA)
            .build();
        let mut reader = agent.get(url).call().ok()?.into_reader();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut buf).ok()?;
        if buf.is_empty() {
            return None;
        }
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, &buf).ok()?;
        fs::rename(&tmp, &path).ok()?;
        Some(path)
    }

    pub fn enrich(
        &self,
        lamports: u64,
        tokens: &[TokenBalance],
    ) -> Vec<Holding> {
        let mut mint_ids: Vec<(String, String, String, u8, u64, String)> = Vec::new();
        // mint, coin_id, symbol_hint, decimals, raw, ui
        mint_ids.push((
            WSOL_MINT.to_string(),
            "solana".into(),
            "SOL".into(),
            9,
            lamports,
            crate::format_sol(lamports),
        ));
        for t in tokens {
            if t.mint == WSOL_MINT {
                continue;
            }
            let raw = parse_raw_u64(&t.amount_raw);
            if raw == 0 {
                continue;
            }
            let gecko = tokens::gecko_id_for_mint(&t.mint)
                .map(|s| s.to_string())
                .or_else(|| {
                    self.lookup_solana_mint(&t.mint)
                        .map(|(id, _, _, _)| id)
                })
                .unwrap_or_default();
            mint_ids.push((
                t.mint.clone(),
                gecko,
                tokens::display_symbol(&t.mint),
                t.decimals,
                raw,
                t.ui_amount.clone(),
            ));
        }

        let ids: Vec<String> = mint_ids
            .iter()
            .map(|(_, id, _, _, _, _)| id.clone())
            .filter(|id| !id.is_empty())
            .collect();
        let markets = self.fetch_markets(&ids).unwrap_or_default();

        let mut holdings = Vec::new();
        for (mint, coin_id, symbol_hint, decimals, raw, ui) in mint_ids {
            let mkt = markets.get(&coin_id);
            let known = tokens::resolve_known_token(&mint);
            let name = mkt
                .map(|m| m.name.clone())
                .or_else(|| known.map(|k| k.name.to_string()))
                .unwrap_or_else(|| symbol_hint.clone());
            let symbol = mkt
                .map(|m| m.symbol.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or(symbol_hint);
            let price = mkt.map(|m| m.price).unwrap_or(0.0);
            let amount = if decimals == 9 && mint == WSOL_MINT {
                raw as f64 / LAMPORTS_PER_SOL as f64
            } else {
                raw as f64 / 10f64.powi(decimals as i32)
            };
            let image = mkt.map(|m| m.image.clone()).unwrap_or_default();
            let icon_path = if !coin_id.is_empty() && !image.is_empty() {
                self.download_icon(&coin_id, &image)
            } else {
                None
            };
            holdings.push(Holding {
                mint,
                coin_id,
                name,
                symbol,
                amount_raw: raw,
                decimals,
                ui_amount: ui,
                price_usd: price,
                value_usd: amount * price,
                change_1h: mkt.and_then(|m| m.change_1h),
                change_24h: mkt.and_then(|m| m.change_24h),
                change_7d: mkt.and_then(|m| m.change_7d),
                image_url: image,
                icon_path,
            });
        }
        holdings.sort_by(|a, b| {
            b.value_usd
                .partial_cmp(&a.value_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        holdings
    }
}

pub fn icon_cache_dir() -> PathBuf {
    crate::store::user_data_dir().join("icons")
}

fn icon_path(coin_id: &str, url: &str) -> PathBuf {
    let safe: String = coin_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let ext = if url.split('?').next().unwrap_or("").to_ascii_lowercase().ends_with(".jpg")
        || url.split('?').next().unwrap_or("").to_ascii_lowercase().ends_with(".jpeg")
    {
        "jpg"
    } else if url.split('?').next().unwrap_or("").to_ascii_lowercase().ends_with(".webp") {
        "webp"
    } else if url.split('?').next().unwrap_or("").to_ascii_lowercase().ends_with(".svg") {
        "svg"
    } else {
        "png"
    };
    icon_cache_dir().join(format!("{safe}.{ext}"))
}

#[derive(Serialize, Deserialize, Default)]
struct MintCache {
    mints: HashMap<String, [String; 4]>,
}

fn mint_cache_path() -> PathBuf {
    crate::store::user_data_dir().join("gecko-mints.json")
}

fn cache_get(mint: &str) -> Option<(String, String, String, String)> {
    let text = fs::read_to_string(mint_cache_path()).ok()?;
    let cache: MintCache = serde_json::from_str(&text).ok()?;
    cache
        .mints
        .get(mint)
        .map(|v| (v[0].clone(), v[1].clone(), v[2].clone(), v[3].clone()))
}

fn cache_put(mint: &str, id: &str, symbol: &str, name: &str, image: &str) {
    let path = mint_cache_path();
    let mut cache: MintCache = fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    cache.mints.insert(
        mint.to_string(),
        [
            id.to_string(),
            symbol.to_string(),
            name.to_string(),
            image.to_string(),
        ],
    );
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(&cache) {
        let _ = fs::write(path, json);
    }
}

pub fn weighted_change_24h(holdings: &[Holding]) -> Option<f64> {
    let mut w = 0.0;
    let mut t = 0.0;
    for h in holdings {
        if let Some(ch) = h.change_24h {
            if h.value_usd > 0.0 {
                w += h.value_usd * ch;
                t += h.value_usd;
            }
        }
    }
    if t > 0.0 {
        Some(w / t)
    } else {
        None
    }
}

pub fn total_usd(holdings: &[Holding]) -> f64 {
    holdings.iter().map(|h| h.value_usd).sum()
}

#[allow(dead_code)]
fn _path_exists(p: &Path) -> bool {
    p.exists()
}

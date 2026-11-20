//! Currencies: the fiat codes and crypto tickers calc knows, and the stored exchange rates
//! a conversion reads. Pure: the rates arrive in the [`Context`](super::Context).

use std::collections::HashMap;

use super::units::{Dim, Unit};

/// ISO 4217 codes that open.er-api.com serves (Task 6 checks them against a captured answer).
pub const FIAT: &[&str] = &[
    "USD", "AED", "AFN", "ALL", "AMD", "ANG", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD",
    "BDT", "BGN", "BHD", "BIF", "BMD", "BND", "BOB", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD",
    "CAD", "CDF", "CHF", "CLF", "CLP", "CNH", "CNY", "COP", "CRC", "CUP", "CVE", "CZK", "DJF",
    "DKK", "DOP", "DZD", "EGP", "ERN", "ETB", "EUR", "FJD", "FKP", "FOK", "GBP", "GEL", "GGP",
    "GHS", "GIP", "GMD", "GNF", "GTQ", "GYD", "HKD", "HNL", "HRK", "HTG", "HUF", "IDR", "ILS",
    "IMP", "INR", "IQD", "IRR", "ISK", "JEP", "JMD", "JOD", "JPY", "KES", "KGS", "KHR", "KID",
    "KMF", "KRW", "KWD", "KYD", "KZT", "LAK", "LBP", "LKR", "LRD", "LSL", "LYD", "MAD", "MDL",
    "MGA", "MKD", "MMK", "MNT", "MOP", "MRU", "MUR", "MVR", "MWK", "MXN", "MYR", "MZN", "NAD",
    "NGN", "NIO", "NOK", "NPR", "NZD", "OMR", "PAB", "PEN", "PGK", "PHP", "PKR", "PLN", "PYG",
    "QAR", "RON", "RSD", "RUB", "RWF", "SAR", "SBD", "SCR", "SDG", "SEK", "SGD", "SHP", "SLE",
    "SLL", "SOS", "SRD", "SSP", "STN", "SYP", "SZL", "THB", "TJS", "TMT", "TND", "TOP", "TRY",
    "TTD", "TVD", "TWD", "TZS", "UAH", "UGX", "UYU", "UZS", "VES", "VND", "VUV", "WST", "XAF",
    "XCD", "XCG", "XDR", "XOF", "XPF", "YER", "ZAR", "ZMW", "ZWG", "ZWL",
];

/// (ticker, CoinGecko id).
pub const CRYPTO: &[(&str, &str)] = &[
    ("BTC", "bitcoin"),
    ("ETH", "ethereum"),
    ("USDT", "tether"),
    ("BNB", "binancecoin"),
    ("SOL", "solana"),
    ("USDC", "usd-coin"),
    ("XRP", "ripple"),
    ("DOGE", "dogecoin"),
    ("ADA", "cardano"),
    ("TRX", "tron"),
    ("TON", "the-open-network"),
    ("AVAX", "avalanche-2"),
    ("SHIB", "shiba-inu"),
    ("DOT", "polkadot"),
    ("LINK", "chainlink"),
    ("BCH", "bitcoin-cash"),
    ("LTC", "litecoin"),
    ("XLM", "stellar"),
    ("XMR", "monero"),
    ("ETC", "ethereum-classic"),
    ("ATOM", "cosmos"),
    ("UNI", "uniswap"),
    ("NEAR", "near"),
    ("APT", "aptos"),
    ("SUI", "sui"),
    ("POL", "polygon-ecosystem-token"),
    ("ARB", "arbitrum"),
    ("OP", "optimism"),
    ("FIL", "filecoin"),
    ("ICP", "internet-computer"),
    ("PEPE", "pepe"),
    ("DAI", "dai"),
];

/// Currency symbols and the code each stands for.
pub const SYMBOLS: &[(&str, &str)] = &[
    ("$", "USD"),
    ("€", "EUR"),
    ("£", "GBP"),
    ("¥", "JPY"),
    ("₽", "RUB"),
    ("₹", "INR"),
    ("₩", "KRW"),
    ("₺", "TRY"),
    ("₴", "UAH"),
    ("₪", "ILS"),
    ("₿", "BTC"),
];

/// The canonical code of `name`: a code or ticker in any case, or a symbol.
fn code(name: &str) -> Option<&'static str> {
    if let Some((_, code)) = SYMBOLS.iter().find(|(symbol, _)| *symbol == name) {
        return Some(code);
    }
    FIAT.iter()
        .copied()
        .chain(CRYPTO.iter().map(|(ticker, _)| *ticker))
        .find(|code| code.eq_ignore_ascii_case(name))
}

/// A code or ticker in any case, or a symbol, as a unit. The unit carries no factor: a
/// currency converts through the rates, not through the unit table.
pub fn lookup(name: &str) -> Option<Unit> {
    code(name).map(|name| Unit {
        name,
        dim: Dim::Currency,
        factor: 0.0,
        offset: 0.0,
    })
}

/// Whether `name` is one of the symbols in [`SYMBOLS`].
pub fn is_symbol(name: &str) -> bool {
    SYMBOLS.iter().any(|(symbol, _)| *symbol == name)
}

/// Whether the canonical `code` is a crypto ticker.
pub fn is_crypto(code: &str) -> bool {
    CRYPTO.iter().any(|(ticker, _)| *ticker == code)
}

/// The rates of one feed, as calc received them.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// When calc received them, in unix seconds.
    pub fetched: i64,
    pub values: HashMap<&'static str, f64>,
}

/// The stored rates. `fiat` values are units per 1 USD; `crypto` values are the USD price of
/// 1 coin.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rates {
    pub fiat: Option<Table>,
    pub crypto: Option<Table>,
}

impl Rates {
    /// How many USD one unit of `code` is worth. USD is 1 with no table. Missing table: "no
    /// exchange rates yet"; missing code: "no rate for <CODE>".
    pub fn usd_worth(&self, code: &str) -> Result<f64, String> {
        if code == "USD" {
            return Ok(1.0);
        }
        let crypto = is_crypto(code);
        let table = if crypto { &self.crypto } else { &self.fiat };
        let table = table.as_ref().ok_or("no exchange rates yet")?;
        match table.values.get(code) {
            Some(value) if crypto => Ok(*value),
            Some(per_usd) => Ok(1.0 / per_usd),
            None => Err(format!("no rate for {code}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::units;

    #[test]
    fn codes_are_unique_and_found() {
        let mut all: Vec<&str> = FIAT
            .iter()
            .copied()
            .chain(CRYPTO.iter().map(|(ticker, _)| *ticker))
            .collect();
        let total = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), total, "a code or ticker is listed twice");
        for code in &all {
            for name in [code.to_string(), code.to_lowercase()] {
                assert_eq!(lookup(&name).map(|u| u.name), Some(*code), "{name}");
                assert_eq!(lookup(&name).map(|u| u.dim), Some(Dim::Currency));
            }
        }
        // Only `cup`, the volume unit, and `ton`, the metric tonne, shadow a code in lower
        // case (the peso is `CUP`, the coin `TON`).
        for name in units::all_names() {
            if let Some(code) = all.iter().find(|c| c.eq_ignore_ascii_case(name)) {
                assert!(
                    [("cup", "CUP"), ("ton", "TON")].contains(&(name, *code)),
                    "{name} shadows {code}"
                );
            }
        }
        for (symbol, code) in SYMBOLS {
            assert_eq!(lookup(symbol).map(|u| u.name), Some(*code), "{symbol}");
        }
        assert!(lookup("xyz").is_none() && lookup("").is_none());
    }

    #[test]
    fn usd_worth_reads_the_tables() {
        let table = |pairs: &[(&'static str, f64)]| Table {
            fetched: 0,
            values: pairs.iter().copied().collect(),
        };
        let rates = Rates {
            fiat: Some(table(&[("EUR", 0.5)])),
            crypto: None,
        };
        assert_eq!(rates.usd_worth("USD"), Ok(1.0));
        assert_eq!(rates.usd_worth("EUR"), Ok(2.0));
        assert_eq!(rates.usd_worth("RUB"), Err("no rate for RUB".into()));
        assert_eq!(rates.usd_worth("BTC"), Err("no exchange rates yet".into()));
        assert_eq!(
            Rates::default().usd_worth("EUR"),
            Err("no exchange rates yet".into())
        );
    }
}

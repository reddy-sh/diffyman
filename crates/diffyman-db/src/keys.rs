//! API keys and the admin token: random 256-bit secrets.
//!
//! A secret is a kind prefix (`dmk_` for API keys, `dma_` for the admin
//! token) and 64 lowercase hex digits from the operating system's random
//! source. The database keeps only [`Secret::prefix`] (the first 12
//! characters, to find the row) and [`Secret::hash`]: BLAKE3 in its
//! key-derivation mode under a fixed context string, a keyed hash with domain
//! separation. A slow password hash such as Argon2 adds nothing for a 256-bit
//! random value, which cannot be guessed, and would make every request cost
//! tens of milliseconds of CPU. [`KeyHash`] comparisons take constant time.

use std::fmt;

/// Context string of the key hash. Changing it invalidates every stored key.
const HASH_CONTEXT: &str = "diffyman 2026-11-08 api key hash v1";

/// Length of the stored lookup prefix: the kind prefix and 8 hex digits.
pub const PREFIX_LEN: usize = 12;

/// Which kind of secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretKind {
    /// An API key, `dmk_...`.
    ApiKey,
    /// The bootstrap admin token, `dma_...`.
    Admin,
}

impl SecretKind {
    fn tag(self) -> &'static str {
        match self {
            SecretKind::ApiKey => "dmk_",
            SecretKind::Admin => "dma_",
        }
    }
}

/// A secret in its text form. `Debug` and `Display` show only the prefix.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret {
    kind: SecretKind,
    text: String,
}

impl Secret {
    /// A new random secret.
    pub fn generate(kind: SecretKind) -> Result<Secret, getrandom::Error> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes)?;
        let mut text = String::with_capacity(68);
        text.push_str(kind.tag());
        for byte in bytes {
            text.push(char::from(HEX[usize::from(byte >> 4)]));
            text.push(char::from(HEX[usize::from(byte & 0xf)]));
        }
        Ok(Secret { kind, text })
    }

    /// Parses a presented secret. Returns `None` unless it has the exact
    /// shape of `kind`; surrounding whitespace is not accepted.
    pub fn parse(text: &str, kind: SecretKind) -> Option<Secret> {
        let hex = text.strip_prefix(kind.tag())?;
        let valid = hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        valid.then(|| Secret {
            kind,
            text: text.to_owned(),
        })
    }

    /// The kind of secret.
    pub fn kind(&self) -> SecretKind {
        self.kind
    }

    /// The full secret, to show once or write to the token file.
    pub fn expose(&self) -> &str {
        &self.text
    }

    /// The lookup prefix stored in the database.
    pub fn prefix(&self) -> &str {
        &self.text[..PREFIX_LEN]
    }

    /// The hash stored in the database.
    pub fn hash(&self) -> KeyHash {
        let mut hasher = blake3::Hasher::new_derive_key(HASH_CONTEXT);
        hasher.update(self.text.as_bytes());
        KeyHash(hasher.finalize())
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret({}…)", self.prefix())
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}…", self.prefix())
    }
}

/// The stored hash of a secret. Equality is constant-time.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct KeyHash(blake3::Hash);

impl KeyHash {
    /// The 32 bytes stored in the database.
    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }

    /// Reads stored bytes; `None` unless there are exactly 32.
    pub fn from_slice(bytes: &[u8]) -> Option<KeyHash> {
        let array: [u8; 32] = bytes.try_into().ok()?;
        Some(KeyHash(blake3::Hash::from_bytes(array)))
    }

    /// Whether `secret` hashes to this value, in constant time.
    pub fn verify(&self, secret: &Secret) -> bool {
        // blake3::Hash implements PartialEq in constant time.
        self.0 == secret.hash().0
    }
}

impl fmt::Debug for KeyHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeyHash(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_secrets_have_the_documented_shape() {
        let key = Secret::generate(SecretKind::ApiKey).unwrap();
        assert_eq!(key.expose().len(), 68);
        assert!(key.expose().starts_with("dmk_"));
        assert_eq!(key.prefix(), &key.expose()[..12]);
        assert_eq!(
            Secret::parse(key.expose(), SecretKind::ApiKey),
            Some(key.clone())
        );
        assert_eq!(Secret::parse(key.expose(), SecretKind::Admin), None);
        let admin = Secret::generate(SecretKind::Admin).unwrap();
        assert!(admin.expose().starts_with("dma_"));
        assert_ne!(key.expose()[4..], admin.expose()[4..]);
    }

    #[test]
    fn parse_rejects_near_misses() {
        let good = format!("dmk_{}", "0123456789abcdef".repeat(4));
        assert!(Secret::parse(&good, SecretKind::ApiKey).is_some());
        for bad in [
            format!("{good} "),
            format!(" {good}"),
            good.to_uppercase(),
            good[..67].to_owned(),
            format!("{good}0"),
            good.replace('a', "g"),
            format!("dmx_{}", &good[4..]),
            String::new(),
        ] {
            assert!(Secret::parse(&bad, SecretKind::ApiKey).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn hashes_verify_and_never_print_the_secret() {
        let key = Secret::generate(SecretKind::ApiKey).unwrap();
        let other = Secret::generate(SecretKind::ApiKey).unwrap();
        let hash = key.hash();
        assert!(hash.verify(&key));
        assert!(!hash.verify(&other));
        assert_eq!(KeyHash::from_slice(hash.as_bytes()), Some(hash));
        assert_eq!(KeyHash::from_slice(&[0; 31]), None);
        let shown = format!("{key} {key:?} {hash:?}");
        assert!(!shown.contains(&key.expose()[12..]), "{shown}");
        // The hash is fixed by the context string: a known answer.
        let fixed = Secret::parse(&format!("dmk_{}", "0".repeat(64)), SecretKind::ApiKey).unwrap();
        let expected = blake3::derive_key(HASH_CONTEXT, fixed.expose().as_bytes());
        assert_eq!(fixed.hash().as_bytes(), &expected);
    }
}

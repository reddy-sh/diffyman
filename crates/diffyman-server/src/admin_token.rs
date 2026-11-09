//! The bootstrap admin token: written once, read on every start.
//!
//! On first start [`ensure`] writes a random 256-bit token (`dma_` and 64 hex
//! digits) to the token file, created exclusively with mode 0600 in a folder
//! created with mode 0700. Later starts read it back and refuse a file that
//! group or others can read. The token is never logged or stored in the
//! database; only the admin holding the file can create API keys: the
//! `diffyman-server keys` subcommands refuse to run unless they can [`load`]
//! it.

use std::io::Write;
use std::path::Path;

use diffyman_db::keys::{KeyHash, Secret, SecretKind};

/// Errors from [`ensure`]. Messages never contain the token.
#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    /// The file or its folder could not be read or written.
    #[error("admin token file: {0}")]
    Io(#[from] std::io::Error),
    /// The file is readable by group or others.
    #[error("admin token file must not be readable by group or others (chmod 600)")]
    Permissions,
    /// The file does not hold a token.
    #[error("admin token file does not hold a valid token")]
    Malformed,
    /// The random source failed.
    #[error("no random source for the admin token")]
    Random,
}

/// The admin token as the server keeps it: only its hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdminToken {
    hash: KeyHash,
    /// `true` when this call wrote the file.
    pub created: bool,
}

impl AdminToken {
    /// Whether `presented` is the admin token, in constant time.
    pub fn verify(&self, presented: &str) -> bool {
        Secret::parse(presented, SecretKind::Admin).is_some_and(|s| self.hash.verify(&s))
    }
}

/// Reads the token at `path`, writing a new one first when the file does not
/// exist.
pub fn ensure(path: &Path) -> Result<AdminToken, TokenError> {
    match load(path) {
        Ok(token) => return Ok(token),
        Err(TokenError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(other) => return Err(other),
    }
    if let Some(parent) = path.parent() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(parent)?;
    }
    let secret = Secret::generate(SecretKind::Admin).map_err(|_| TokenError::Random)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(secret.expose().as_bytes())?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            Ok(AdminToken {
                hash: secret.hash(),
                created: true,
            })
        }
        // Another process wrote it first.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => load(path),
        Err(e) => Err(e.into()),
    }
}

/// Reads the token at `path` without ever writing it.
pub fn load(path: &Path) -> Result<AdminToken, TokenError> {
    let text = std::fs::read_to_string(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(path)?.permissions().mode() & 0o077 != 0 {
            return Err(TokenError::Permissions);
        }
    }
    let secret = Secret::parse(text.trim_end_matches(['\n', '\r']), SecretKind::Admin)
        .ok_or(TokenError::Malformed)?;
    Ok(AdminToken {
        hash: secret.hash(),
        created: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_once_then_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/admin-token");
        let first = ensure(&path).unwrap();
        assert!(first.created);
        let text = std::fs::read_to_string(&path).unwrap();
        let token = text.trim_end();
        assert!(token.starts_with("dma_") && token.len() == 68);
        assert!(first.verify(token));
        assert!(!first.verify(&token.replace("dma_", "dmk_")));
        assert!(!first.verify("dma_0"));

        let second = ensure(&path).unwrap();
        assert!(!second.created);
        assert!(second.verify(token));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            text,
            "never rewritten"
        );
        assert!(!format!("{first:?}").contains(&token[4..]));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(path.parent().unwrap()), 0o700);
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(matches!(ensure(&path), Err(TokenError::Permissions)));
        }
    }

    #[test]
    fn refuses_a_file_without_a_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("admin-token");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&path)
            .unwrap()
            .write_all(b"not a token\n")
            .unwrap();
        assert!(matches!(ensure(&path), Err(TokenError::Malformed)));
    }
}

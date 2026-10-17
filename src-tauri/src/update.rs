//! Update checking and installation.
//!
//! Stubbed out for the TUI version — previously used Tauri's AppHandle to
//! download and install MSI/NSIS installers. The TUI does not handle
//! updates; this module is kept for API compatibility.

const DOWNLOAD_PREFIX: &str =
    "https://github.com/gmiddlecote/hdmi-media-controller/releases/download/";
const MAX_INSTALLER_SIZE: u64 = 2_000_000_000;

pub fn start_install(
    _version: &str,
    _url: &str,
    _sha256: &str,
    _size: u64,
) -> Result<(), String> {
    Err("Updates are not supported in the terminal UI.".into())
}

fn validate_version(version: &str) -> Result<String, String> {
    if version.is_empty() {
        return Err("Version is empty.".into());
    }
    Ok(version.to_string())
}

fn validate_digest(digest: &str) -> Result<(), String> {
    if digest.len() != 64 {
        return Err("SHA-256 digest must be 64 hex characters.".into());
    }
    if !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("SHA-256 digest contains non-hex characters.".into());
    }
    Ok(())
}

fn validate_url(url: &str) -> Result<(), String> {
    if !url.starts_with(DOWNLOAD_PREFIX) {
        return Err("Update URL is not from the project releases domain.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricts_release_urls() {
        assert!(validate_url(
            "https://github.com/gmiddlecote/hdmi-media-controller/releases/download/v0.23.0/hdmi-media-controller.msi"
        ).is_ok());
        assert!(validate_url("https://evil.example.com/x.msi").is_err());
    }

    #[test]
    fn validates_release_digests() {
        assert!(validate_digest("a".repeat(64)).is_ok());
        assert!(validate_digest("zzzz").is_err());
        assert!(validate_digest("g".repeat(64)).is_err());
    }

    #[test]
    fn validates_release_versions() {
        assert!(validate_version("0.23.0").is_ok());
        assert!(validate_version("").is_err());
    }
}

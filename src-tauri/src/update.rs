#[cfg(target_os = "windows")]
use std::fs;
#[cfg(target_os = "windows")]
use std::path::{Path, PathBuf};

const DOWNLOAD_PREFIX: &str =
    "https://github.com/gmiddlecote/hdmi-media-controller/releases/download/";
const MAX_INSTALLER_SIZE: u64 = 2_000_000_000;

pub fn start_install(
    app: &tauri::AppHandle,
    version: &str,
    url: &str,
    sha256: &str,
    size: u64,
) -> Result<(), String> {
    let safe_version = validate_version(version)?;
    let digest = validate_digest(sha256)?;
    validate_url(url)?;
    if size == 0 || size > MAX_INSTALLER_SIZE {
        return Err("The update installer has an invalid size.".into());
    }

    #[cfg(target_os = "windows")]
    {
        start_install_windows(app, &safe_version, url, &digest, size)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, safe_version, digest);
        Err("Automatic updates are only supported on Windows.".into())
    }
}

fn validate_version(version: &str) -> Result<String, String> {
    let value = version.strip_prefix('v').unwrap_or(version);
    if value.is_empty()
        || value.len() > 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err("The update version is invalid.".into());
    }
    Ok(value.to_string())
}

fn validate_digest(sha256: &str) -> Result<String, String> {
    let value = sha256
        .split_once(':')
        .filter(|(prefix, _)| prefix.eq_ignore_ascii_case("sha256"))
        .map_or(sha256, |(_, digest)| digest);
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("The update checksum is invalid.".into());
    }
    Ok(value.to_ascii_lowercase())
}

fn validate_url(url: &str) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    if !url.starts_with(DOWNLOAD_PREFIX)
        || !lower.ends_with(".exe")
        || url.contains("..")
        || url.contains('?')
        || url.contains('#')
        || url.contains('\r')
        || url.contains('\n')
    {
        return Err("The update download URL is not trusted.".into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn start_install_windows(
    app: &tauri::AppHandle,
    version: &str,
    url: &str,
    sha256: &str,
    size: u64,
) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    let installer =
        std::env::temp_dir().join(format!("duetplay-{version}-{}.exe", std::process::id()));
    let script_path =
        std::env::temp_dir().join(format!("duetplay-update-{}.ps1", std::process::id()));
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("Could not locate DuetPlay: {error}"))?
        .to_string_lossy()
        .into_owned();
    let script = build_script(
        &installer,
        url,
        sha256,
        size,
        &current_exe,
        std::process::id(),
    );
    fs::write(&script_path, script)
        .map_err(|error| format!("Could not create updater: {error}"))?;

    let system_root = std::env::var_os("SystemRoot")
        .ok_or_else(|| "Could not locate Windows PowerShell.".to_string())?;
    let powershell =
        PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut command = Command::new(powershell);
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script_path)
        .creation_flags(0x08000000 | 0x00000200);

    match command.spawn() {
        Ok(_) => {
            app.exit(0);
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(&script_path);
            Err(format!("Could not start the updater: {error}"))
        }
    }
}

#[cfg(target_os = "windows")]
fn build_script(
    installer: &Path,
    url: &str,
    sha256: &str,
    size: u64,
    current_exe: &str,
    app_process_id: u32,
) -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$installer = '{installer}'
$url = '{url}'
$expectedHash = '{sha256}'
$expectedSize = {size}
$currentExe = '{current_exe}'
$appProcessId = {app_process_id}
Remove-Item -LiteralPath $installer -Force -ErrorAction SilentlyContinue
Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $installer
$actualSize = (Get-Item -LiteralPath $installer).Length
if ($actualSize -ne $expectedSize) {{ throw 'Downloaded installer size does not match.' }}
$actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $installer).Hash.ToLowerInvariant()
if ($actualHash -ne $expectedHash) {{ throw 'Downloaded installer hash does not match.' }}
Wait-Process -Id $appProcessId -ErrorAction SilentlyContinue
$installProcess = Start-Process -FilePath $installer -ArgumentList '/S' -Verb RunAs -Wait -PassThru
if ($installProcess.ExitCode -ne 0) {{ throw 'The update installer failed.' }}
Start-Process -FilePath $currentExe
Remove-Item -LiteralPath $PSCommandPath -Force -ErrorAction SilentlyContinue
"#,
        installer = powershell_quote(&installer.to_string_lossy()),
        url = powershell_quote(url),
        sha256 = powershell_quote(sha256),
        size = size,
        current_exe = powershell_quote(current_exe),
        app_process_id = app_process_id,
    )
}

#[cfg(target_os = "windows")]
fn powershell_quote(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_release_versions() {
        assert_eq!(validate_version("v0.8.0").unwrap(), "0.8.0");
        assert!(validate_version("../bad").is_err());
    }

    #[test]
    fn validates_release_digests() {
        assert_eq!(
            validate_digest(
                "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            )
            .unwrap(),
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        assert!(validate_digest("not-a-digest").is_err());
    }

    #[test]
    fn restricts_release_urls() {
        assert!(validate_url(
            "https://github.com/gmiddlecote/hdmi-media-controller/releases/download/v0.8.0/DuetPlay_0.8.0_x64-setup.exe"
        )
        .is_ok());
        assert!(validate_url("https://example.com/update.exe").is_err());
        assert!(validate_url(
            "https://github.com/gmiddlecote/hdmi-media-controller/releases/download/v0.8.0/update.msi"
        )
        .is_err());
    }
}

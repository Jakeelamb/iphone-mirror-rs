//! Read existing pymobiledevice3 credentials without changing their ownership or contents.
use anyhow::{Context, Result, bail};
use ed25519_dalek::{SigningKey, VerifyingKey};
use idevice::remote_pairing::RpPairingFile;
use std::path::{Path, PathBuf};

pub fn load(path: &Path) -> Result<RpPairingFile> {
    // Do not call RpPairingFile::from_bytes: upstream debug logging includes key data.
    let value = plist::Value::from_file(path).context("read saved CoreDevice pairing")?;
    parse(&value)
}

fn parse(value: &plist::Value) -> Result<RpPairingFile> {
    let dict = value
        .as_dictionary()
        .context("pairing record must be a dictionary")?;
    let private: [u8; 32] = dict
        .get("private_key")
        .and_then(plist::Value::as_data)
        .context("pairing record has no private key")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("pairing private key must contain 32 bytes"))?;
    let public: [u8; 32] = dict
        .get("public_key")
        .and_then(plist::Value::as_data)
        .context("pairing record has no public key")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("pairing public key must contain 32 bytes"))?;
    let signing = SigningKey::from_bytes(&private);
    let verifying = VerifyingKey::from_bytes(&public).context("invalid pairing public key")?;
    if signing.verifying_key() != verifying {
        bail!("pairing key pair does not match");
    }
    let identifier = match dict.get("identifier").and_then(plist::Value::as_string) {
        Some(id) => id.to_owned(),
        None => {
            let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
                .context("read hostname used by pymobiledevice3 pairing")?;
            uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_DNS, hostname.trim_end().as_bytes())
                .to_string()
                .to_uppercase()
        }
    };
    Ok(RpPairingFile {
        e_private_key: signing,
        e_public_key: verifying,
        identifier,
        alt_irk: dict
            .get("alt_irk")
            .and_then(plist::Value::as_data)
            .map(ToOwned::to_owned),
    })
}

pub fn find(explicit: Option<&Path>, serial: Option<&str>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_owned());
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .context("HOME or XDG_DATA_HOME is required to find saved pairing")?;
    let directory = data.join("pymobiledevice3");
    let normalized = serial.map(|s| s.replace('-', ""));
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(directory)
        .context("no pymobiledevice3 pairing directory; pair the iPhone first")?
    {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !entry.file_type()?.is_file() || !name.starts_with("remote_") {
            continue;
        }
        let id = name
            .trim_start_matches("remote_")
            .split('.')
            .next()
            .unwrap_or_default()
            .replace('-', "");
        if normalized.as_ref().is_none_or(|wanted| wanted == &id) {
            candidates.push(entry.path());
        }
    }
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => bail!("no matching saved CoreDevice pairing; pair the iPhone first"),
        _ => bail!("several saved CoreDevice pairings; choose --serial or --pairing-file"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> plist::Value {
        let signing = SigningKey::from_bytes(&[42; 32]);
        let mut dict = plist::Dictionary::new();
        dict.insert(
            "private_key".into(),
            plist::Value::Data(signing.to_bytes().to_vec()),
        );
        dict.insert(
            "public_key".into(),
            plist::Value::Data(signing.verifying_key().to_bytes().to_vec()),
        );
        dict.insert("identifier".into(), "test-host-identifier".into());
        plist::Value::Dictionary(dict)
    }

    #[test]
    fn imports_existing_keys_without_changing_identifier() -> Result<()> {
        let loaded = parse(&record())?;
        assert_eq!(loaded.identifier, "test-host-identifier");
        assert_eq!(loaded.e_private_key.to_bytes(), [42; 32]);
        Ok(())
    }

    #[test]
    fn rejects_mismatched_keys_without_including_key_bytes() -> Result<()> {
        let mut value = record();
        value
            .as_dictionary_mut()
            .context("fixture dictionary")?
            .insert("private_key".into(), plist::Value::Data(vec![43; 32]));
        let error = parse(&value).err().context("mismatch must fail")?;
        assert_eq!(error.to_string(), "pairing key pair does not match");
        Ok(())
    }

    #[test]
    fn hostname_identifier_matches_python_uuid3() {
        assert_eq!(
            uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_DNS, b"test-host")
                .to_string()
                .to_uppercase(),
            "F095EC99-C175-33A2-B410-6E76752A1F40"
        );
    }
}

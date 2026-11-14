//! The Windows Credential Manager: generic credentials under the editor's own target names
//! (`GrizzlyEditor:GitHub`, `GrizzlyEditor:VirusTotal`, `...:qa` in a QA run), the secret stored
//! as its raw bytes - so a token either editor saved is the other's too. Blocking: call on the
//! IO pool.

use zeroize::Zeroizing;

#[cfg(windows)]
mod imp {
    use std::collections::HashMap;

    use keyring_core::api::CredentialStoreApi;
    use zeroize::Zeroizing;

    fn entry(target: &str) -> Option<keyring_core::Entry> {
        let store = windows_native_keyring_store::Store::new().ok()?;
        // CRED_PERSIST_LOCAL_MACHINE, as it was always written
        let modifiers = HashMap::from([("target", target), ("persistence", "Local")]);
        store.build("GrizzlyEditor", "", Some(&modifiers)).ok()
    }

    pub(super) fn read(target: &str) -> Option<Zeroizing<Vec<u8>>> {
        entry(target)?.get_secret().ok().map(Zeroizing::new)
    }

    pub(super) fn write(target: &str, secret: &[u8]) -> Result<(), String> {
        let entry = entry(target).ok_or_else(|| "the Credential Manager is not available".to_owned())?;
        entry.set_secret(secret).map_err(|e| e.to_string())
    }

    pub(super) fn delete(target: &str) {
        if let Some(entry) = entry(target) {
            let _ = entry.delete_credential();
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use zeroize::Zeroizing;

    pub(super) fn read(_target: &str) -> Option<Zeroizing<Vec<u8>>> {
        None
    }

    pub(super) fn write(_target: &str, _secret: &[u8]) -> Result<(), String> {
        Err("the Credential Manager is a Windows service".to_owned())
    }

    pub(super) fn delete(_target: &str) {}
}

/// The secret of a credential as text; `None` when there is none or it is not UTF-8.
pub(crate) fn read(target: &str) -> Option<Zeroizing<String>> {
    let bytes = imp::read(target)?;
    let text = std::str::from_utf8(&bytes).ok()?;
    Some(Zeroizing::new(text.to_owned()))
}

pub(crate) fn write(target: &str, secret: &str) -> Result<(), String> {
    imp::write(target, secret.as_bytes())
}

pub(crate) fn delete(target: &str) {
    imp::delete(target);
}

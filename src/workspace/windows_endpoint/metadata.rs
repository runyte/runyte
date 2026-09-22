// SPDX-License-Identifier: MPL-2.0

use super::*;
use serde::{Deserialize, Serialize};
use std::os::windows::ffi::OsStrExt;
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
};

pub const MAX_METADATA_BYTES: usize = 64 * 1024;
pub const MAX_PERSISTED_PATH_BYTES: usize = 4 * 1024;
const PIPE_PREFIX: &str = r"\\.\pipe\runyte-v1-";

/// A local native transport address, never a filesystem publication path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PipeAddress(String);

impl PipeAddress {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PipeAddress {
    type Error = io::Error;
    fn try_from(value: String) -> io::Result<Self> {
        let token = value
            .strip_prefix(PIPE_PREFIX)
            .ok_or_else(|| invalid("invalid local host pipe address"))?;
        validate_token(token)?;
        Ok(Self(value))
    }
}

impl From<PipeAddress> for String {
    fn from(value: PipeAddress) -> String {
        value.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointMetadata {
    pub protocol: u32,
    pub id: String,
    pub name: Option<String>,
    pub project_root_bytes: Vec<u8>,
    pub process: ProcessIdentity,
    pub incarnation: String,
    pub address: PipeAddress,
}

impl EndpointMetadata {
    pub(super) fn new(project: &Path, name: Option<String>) -> io::Result<Self> {
        let value = Self {
            protocol: crate::protocol::VERSION,
            id: crate::workspace::workspace_id(project),
            name,
            project_root_bytes: encode_path(project),
            process: ProcessIdentity::current()?,
            incarnation: random_token()?,
            address: PipeAddress(format!("{PIPE_PREFIX}{}", random_token()?)),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn project_root(&self) -> io::Result<PathBuf> {
        persisted_path(&self.project_root_bytes)
    }

    pub fn validate(&self) -> io::Result<()> {
        if self.protocol == 0 {
            return Err(invalid("invalid host protocol"));
        }
        self.process.validate()?;
        validate_token(&self.incarnation)?;
        PipeAddress::try_from(self.address.0.clone())?;
        let project = self.project_root()?;
        if self.id != crate::workspace::workspace_id(&project) {
            return Err(invalid(
                "host workspace identity does not match its exact path",
            ));
        }
        if let Some(name) = &self.name
            && (name.is_empty()
                || name.len() > 64
                || name != name.trim()
                || name.chars().any(char::is_control))
        {
            return Err(invalid("invalid session name"));
        }
        Ok(())
    }

    pub fn from_json(bytes: &[u8]) -> io::Result<Self> {
        bounded(bytes)?;
        let value: Self = serde_json::from_slice(bytes).map_err(invalid_json)?;
        value.validate()?;
        Ok(value)
    }

    pub(super) fn to_json(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(invalid_json)?;
        bounded(&bytes)?;
        Ok(bytes)
    }
}

/// Discovery candidate only: neither metadata nor its PID grants peer authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryRecord {
    pub host: EndpointMetadata,
    pub ready_record_bytes: Vec<u8>,
}

impl RegistryRecord {
    pub fn ready_record(&self) -> io::Result<PathBuf> {
        persisted_path(&self.ready_record_bytes)
    }

    pub fn from_json(bytes: &[u8]) -> io::Result<Self> {
        bounded(bytes)?;
        let value: Self = serde_json::from_slice(bytes).map_err(invalid_json)?;
        value.host.validate()?;
        value.ready_record()?;
        Ok(value)
    }

    pub(super) fn to_json(&self) -> io::Result<Vec<u8>> {
        self.host.validate()?;
        self.ready_record()?;
        let bytes = serde_json::to_vec(self).map_err(invalid_json)?;
        bounded(&bytes)?;
        Ok(bytes)
    }
}

pub(super) fn persisted_path(bytes: &[u8]) -> io::Result<PathBuf> {
    if bytes.is_empty() || bytes.len() > MAX_PERSISTED_PATH_BYTES {
        return Err(invalid("host metadata path exceeds its 4 KiB byte limit"));
    }
    let path = decode_path(bytes.to_vec())?;
    if !path.is_absolute() || path.as_os_str().encode_wide().any(|unit| unit == 0) {
        return Err(invalid(
            "host metadata requires an absolute path without NUL units",
        ));
    }
    Ok(path)
}

pub(super) fn validate_id(id: &str) -> io::Result<()> {
    if id.len() != crate::workspace::WORKSPACE_ID_LENGTH || !lower_hex(id) {
        return Err(invalid("invalid workspace identity"));
    }
    Ok(())
}

fn validate_token(token: &str) -> io::Result<()> {
    if token.len() != 64 || !lower_hex(token) {
        return Err(invalid("invalid host incarnation or pipe token"));
    }
    Ok(())
}

fn lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn random_token() -> io::Result<String> {
    use std::fmt::Write;
    let mut bytes = [0u8; 32];
    if unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    } < 0
    {
        return Err(io::Error::other("system random generator failed"));
    }
    let mut value = String::with_capacity(64);
    for byte in bytes {
        write!(value, "{byte:02x}").expect("writing into String cannot fail");
    }
    Ok(value)
}

fn bounded(bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(invalid("host metadata exceeds 64 KiB"));
    }
    Ok(())
}

fn invalid_json(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

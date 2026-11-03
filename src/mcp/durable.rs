// SPDX-License-Identifier: MPL-2.0

//! Explicit, pre-provisioned profiles for isolated durable-channel trials.
use super::{Failure, Result, client::Bridge};
use crate::{private_storage::Directory, workspace::context::durable_protocol::Operation};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs::File, io::Read, path::Path};

pub(super) struct Profile {
    _file: File,
    workspace: String,
    store: String,
    secret: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    workspace: String,
    store: String,
    secret: String,
}
impl Profile {
    pub(super) fn open(path: &Path) -> anyhow::Result<Self> {
        anyhow::ensure!(path.is_absolute(), "channel profile path must be absolute");
        let directory = Directory::open_existing(
            path.parent()
                .ok_or_else(|| anyhow::anyhow!("invalid profile path"))?,
            true,
        )?;
        let mut file = directory.open_read(
            path.file_name()
                .ok_or_else(|| anyhow::anyhow!("invalid profile path"))?,
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            anyhow::ensure!(
                file.metadata()?.permissions().mode() & 0o077 == 0,
                "channel profile must be owner-private"
            );
        }
        lock(&file)?;
        let mut bytes = Vec::new();
        (&mut file).take(2049).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 2048, "channel profile exceeds limit");
        let profile: ProfileFile = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("invalid channel profile"))?;
        let hex = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
        anyhow::ensure!(
            hex(&profile.secret)
                && hex(&profile.store)
                && profile.workspace.len() == crate::workspace::WORKSPACE_ID_LENGTH
                && profile.workspace.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid channel profile binding"
        );
        Ok(Self {
            _file: file,
            workspace: profile.workspace,
            store: profile.store,
            secret: profile.secret,
        })
    }
}
impl Bridge {
    pub(super) async fn durable_call(&mut self, mut args: Value) -> Result<Value> {
        if self.durable_profile.is_none() {
            return Err(Failure::new(
                "unavailable",
                "Durable channels require an explicit --channel-profile file",
            ));
        }
        let workspace = self
            .channel_workspace(args["workspace"].as_str().unwrap_or(""))
            .await?;
        if !self.durable_supported(&workspace) {
            return Err(Failure::new(
                "unsupported",
                "Workspace has not enabled durable channels",
            ));
        }
        let profile = self.durable_profile.as_ref().unwrap();
        if self.workspace_id(&workspace) != Some(profile.workspace.as_str()) {
            return Err(Failure::new(
                "capability_denied",
                "Profile belongs to another workspace",
            ));
        }
        args.as_object_mut().unwrap().remove("workspace");
        match args["action"].as_str().unwrap_or("") {
            "create" => {
                if args.get("fixed").is_none() {
                    args["fixed"] = json!(false);
                }
                if args.get("members").is_none() {
                    args["members"] = json!([]);
                }
            }
            "read" | "history" => {
                if args.get("limit").is_none() {
                    args["limit"] = json!(20);
                }
                if args["action"] == "history" && args.get("before").is_none() {
                    args["before"] = Value::Null;
                }
            }
            "post" => {
                // Explicit immutable location, not recomputed Git metadata.
                // Retrying the same tool arguments reproduces the full payload.
                let directory = args
                    .as_object_mut()
                    .unwrap()
                    .remove("directory")
                    .ok_or_else(|| Failure::invalid("post requires directory"))?;
                args["location"] =
                    json!({"directory":directory,"repository":null,"worktree":null,"branch":null});
            }
            _ => (),
        }
        let operation: Operation = serde_json::from_value(args)
            .map_err(|_| Failure::invalid("Invalid fields for durable action"))?;
        let params =
            json!({"credential":profile.secret,"store":profile.store,"operation":operation});
        self.request(&workspace, "channel.durable", params).await
    }
}

#[cfg(unix)]
fn lock(file: &File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: the descriptor is live and flock takes no pointers.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn lock(file: &File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx},
        System::IO::OVERLAPPED,
    };
    // SAFETY: zero initializes the unused event and byte offset; the file handle
    // and OVERLAPPED pointer remain live throughout this synchronous call.
    let mut offset: OVERLAPPED = unsafe { std::mem::zeroed() };
    if unsafe {
        LockFileEx(
            file.as_raw_handle(),
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut offset,
        )
    } != 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_is_explicit_private_exclusive_and_reopenable() {
        let root = crate::test_support::TestRuntimeRoot::new("durable-live").unwrap();
        let directory = Directory::open(&root.path().join("profiles"), true).unwrap();
        let path = root.path().join("profiles/agent.json");
        assert!(Profile::open(Path::new("relative")).is_err());
        #[cfg(unix)]
        assert!(Profile::open(Path::new("/")).is_err());
        assert!(Profile::open(&path).is_err());
        use std::io::Write;
        let mut file = directory
            .create_new(std::ffi::OsStr::new("agent.json"))
            .unwrap();
        write!(
            file,
            "{}",
            json!({"workspace":"a".repeat(32),"store":"b".repeat(64),"secret":"c".repeat(64)})
        )
        .unwrap();
        drop(file);
        let profile = Profile::open(&path).unwrap();
        assert!(Profile::open(&path).is_err());
        assert_eq!(profile.workspace, "a".repeat(32));
        drop(profile);
        assert!(Profile::open(&path).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(Profile::open(&path).is_err());
        }
    }
    #[tokio::test]
    async fn durable_tool_without_profile_does_not_discover_or_create_storage() {
        let mut bridge =
            Bridge::new(None, "agent".into(), std::time::Duration::from_secs(1)).unwrap();
        assert!(
            bridge
                .call("durable_channel", json!({"action":"resume"}))
                .await
                .is_err()
        );
    }
}

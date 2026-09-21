// SPDX-License-Identifier: MPL-2.0

//! Private overlapped parent pipes with synchronous, isolated child endpoints.
use std::{
    future::Future,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
    task::{Context, Poll, Waker},
};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
    Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
    Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING},
    System::{
        Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
        Threading::GetCurrentProcessId,
    },
};

pub(crate) fn pipe(parent_reads: bool) -> io::Result<(NamedPipeServer, OwnedHandle)> {
    let mut nonce = [0u8; 16];
    let status = unsafe {
        BCryptGenRandom(
            ptr::null_mut(),
            nonce.as_mut_ptr(),
            nonce.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(io::Error::other(
            "cannot allocate a private child-process pipe name",
        ));
    }
    let nonce = nonce
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let name = format!(r"\\.\pipe\runyte-stdio-{}-{nonce}", std::process::id());
    let server = crate::windows_fs::with_private_security(|security| unsafe {
        ServerOptions::new()
            .access_inbound(parent_reads)
            .access_outbound(!parent_reads)
            .first_pipe_instance(true)
            .max_instances(1)
            .reject_remote_clients(true)
            .in_buffer_size(64 * 1024)
            .out_buffer_size(64 * 1024)
            .create_with_security_attributes_raw(
                &name,
                (security as *const windows_sys::Win32::Security::SECURITY_ATTRIBUTES)
                    .cast_mut()
                    .cast(),
            )
    })?;
    let name: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
    // Only the child endpoint is synchronous. It stays non-inheritable here;
    // windows_process duplicates it into its isolated inheritance surrogate.
    let client = unsafe {
        CreateFileW(
            name.as_ptr(),
            if parent_reads {
                GENERIC_WRITE
            } else {
                GENERIC_READ
            },
            0,
            ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if client == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let client = unsafe { OwnedHandle::from_raw_handle(client) };
    // Our synchronous client open has already connected. Tokio owns any
    // overlapped connect state, including cancellation on the failure path.
    let connected = {
        let mut connect = std::pin::pin!(server.connect());
        connect
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
    };
    match connected {
        Poll::Ready(result) => result?,
        Poll::Pending => {
            return Err(io::Error::other(
                "private child-process pipe did not connect",
            ));
        }
    }
    let mut client_pid = 0;
    let mut server_pid = 0;
    if unsafe { GetNamedPipeClientProcessId(server.as_raw_handle(), &mut client_pid) } == 0
        || unsafe { GetNamedPipeServerProcessId(client.as_raw_handle(), &mut server_pid) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let own_pid = unsafe { GetCurrentProcessId() };
    if client_pid != own_pid || server_pid != own_pid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private child-process pipe peer is not the launching process",
        ));
    }
    Ok((server, client))
}

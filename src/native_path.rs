// SPDX-License-Identifier: MPL-2.0

//! Lossless, platform-local path bytes used by workspace identity and the
//! bundled frontend protocol. Unix retains native bytes; Windows uses UTF-16LE
//! code units, including unpaired surrogates. These are not cross-platform paths.
//! Framing, size limits, and filesystem policy belong to the caller.

use std::{
    io,
    path::{Path, PathBuf},
};

pub fn encode_path(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect()
    }
}

pub fn decode_path(bytes: Vec<u8>) -> io::Result<PathBuf> {
    validate_encoding(&bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(std::ffi::OsString::from_vec(bytes).into())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let units: Vec<_> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        Ok(std::ffi::OsString::from_wide(&units).into())
    }
}

pub(crate) fn validate_encoding(bytes: &[u8]) -> io::Result<()> {
    #[cfg(windows)]
    if !bytes.len().is_multiple_of(2) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows path has an incomplete UTF-16 code unit",
        ));
    }
    #[cfg(unix)]
    let _ = bytes;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_paths_round_trip_without_a_codec_size_or_empty_path_policy() {
        for path in [
            Path::new(""),
            Path::new("literal [brackets] café 😀"),
            Path::new("a/../b"),
            Path::new(r"C:\workspace\file"),
            Path::new(r"\\server\share\file"),
            Path::new(r"\\?\C:\workspace\file"),
        ] {
            assert_eq!(decode_path(encode_path(path)).unwrap(), path);
        }
        let long = PathBuf::from("x".repeat(40_000));
        assert_eq!(decode_path(encode_path(&long)).unwrap(), long);
    }

    #[test]
    #[cfg(unix)]
    fn unix_raw_byte_encoding_keeps_existing_wire_values() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let bytes = vec![b'/', b'a', 0xff, 0x80, 0];
        let path = PathBuf::from(OsString::from_vec(bytes.clone()));
        assert_eq!(encode_path(&path), bytes);
        assert_eq!(decode_path(bytes).unwrap(), path);
        assert_eq!(
            encode_path(Path::new("/workspace/example")),
            b"/workspace/example"
        );
    }

    #[test]
    #[cfg(windows)]
    fn windows_codec_is_little_endian_and_preserves_every_code_unit() {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        let units = [67, 58, 92, 0xd800, 32, 0xdc00, 0xd83d, 0xde00, 0];
        let path = PathBuf::from(OsString::from_wide(&units));
        let expected = vec![
            67, 0, 58, 0, 92, 0, 0, 216, 32, 0, 0, 220, 61, 216, 0, 222, 0, 0,
        ];
        assert_eq!(encode_path(&path), expected);
        assert_eq!(decode_path(expected).unwrap(), path);
        for bytes in [vec![0], vec![65, 0, 66]] {
            assert_eq!(
                decode_path(bytes).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }
}

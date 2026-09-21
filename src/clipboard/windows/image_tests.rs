// SPDX-License-Identifier: MPL-2.0

use super::*;

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn dib(v5: bool) -> Vec<u8> {
    let header = if v5 { 124 } else { 40 };
    let mut bytes = vec![0; header + 4];
    put_u32(&mut bytes, 0, header as u32);
    put_u32(&mut bytes, 4, 1);
    put_u32(&mut bytes, 8, 1);
    bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&(if v5 { 32u16 } else { 24u16 }).to_le_bytes());
    put_u32(&mut bytes, 20, 4);
    if v5 {
        put_u32(&mut bytes, 16, 3);
        put_u32(&mut bytes, 40, 0x00ff_0000);
        put_u32(&mut bytes, 44, 0x0000_ff00);
        put_u32(&mut bytes, 48, 0x0000_00ff);
        put_u32(&mut bytes, 52, 0xff00_0000);
        put_u32(&mut bytes, 56, 0x7352_4742);
    }
    bytes[header..].copy_from_slice(&[3, 2, 1, 128]);
    bytes
}

fn set(format: u32, bytes: &[u8], length: usize) {
    assert!(bytes.len() <= length);
    let mut memory = Allocation(unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, length) });
    assert!(!memory.0.is_null(), "{}", io::Error::last_os_error());
    let pointer = unsafe { GlobalLock(memory.0) };
    assert!(!pointer.is_null());
    {
        let _locked = Locked(memory.0);
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
        }
    }
    assert!(
        !unsafe { SetClipboardData(format, memory.0) }.is_null(),
        "{}",
        io::Error::last_os_error()
    );
    memory.0 = ptr::null_mut();
}

fn publish(formats: &[(u32, &[u8])]) {
    let _clipboard = Clipboard::open().unwrap();
    assert_ne!(unsafe { EmptyClipboard() }, 0);
    for (format, bytes) in formats {
        set(*format, bytes, bytes.len());
    }
}

fn assert_pixels(bytes: Vec<u8>, expected: &[u8]) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((info.width, info.height), (1, 1));
    assert_eq!(&pixels[..info.buffer_size()], expected);
}

#[test]
fn native_images_use_the_bounded_worker_on_an_isolated_desktop() {
    const CHILD: &str = "RUNYTE_ISOLATED_IMAGE_CLIPBOARD_TEST";
    if std::env::var_os(CHILD).is_none() {
        let root = crate::test_support::TestRuntimeRoot::new("clipboard-image-child").unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "clipboard::windows::image_tests::native_images_use_the_bounded_worker_on_an_isolated_desktop",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("XDG_CACHE_HOME", root.path().join("cache"))
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    isolate_test_desktop();
    // All reads below exercise the production worker. Its test-only desktop
    // association runs before the production Clipboard::open creates a HWND.
    let png = png_format().unwrap();
    let info = dib(false);
    let v5 = dib(true);
    let encoded = image::dib_to_png(&info, false).unwrap();
    publish(&[]);
    assert!(read_image().unwrap().is_none());

    publish(&[(png, &encoded), (DIB, &info)]);
    let sequence = unsafe { GetClipboardSequenceNumber() };
    let expected = {
        let _clipboard = Clipboard::open().unwrap();
        // SetClipboardData transfers ownership. Reacquire the current handle
        // after reopening; Windows may have replaced the published allocation.
        let memory = unsafe { GetClipboardData(png) };
        assert!(!memory.is_null());
        let length = unsafe { GlobalSize(memory) };
        let pointer = unsafe { GlobalLock(memory) };
        assert!(!pointer.is_null());
        let _locked = Locked(memory);
        unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) }.to_vec()
    };
    assert_eq!(
        read_image().unwrap().unwrap(),
        expected,
        "PNG was re-encoded or lost precedence"
    );
    {
        let _clipboard = Clipboard::open().unwrap();
        assert_eq!(unsafe { GetClipboardSequenceNumber() }, sequence);
        let memory = unsafe { GetClipboardData(png) };
        assert!(!memory.is_null());
        // The low byte of GlobalFlags is the allocation's lock count.
        assert_eq!(
            unsafe { GlobalFlags(memory) } & 0xff,
            0,
            "clipboard allocation remained locked"
        );
    }
    // A second read proves that the first did not free the clipboard's data.
    assert_eq!(read_image().unwrap().unwrap(), expected);

    // CF_TEXT synthesizes CF_UNICODETEXT. A text-plus-bitmap clipboard must
    // take the same text path as the documented Unix clipboard behavior.
    publish(&[(1, b"plain text\0"), (png, &encoded)]);
    assert!(read_image().unwrap().is_none());
    assert_eq!(read().unwrap(), "plain text");

    publish(&[(DIB_V5, &v5)]);
    assert_pixels(read_image().unwrap().unwrap(), &[1, 2, 3, 128]);
    publish(&[(DIB, &info)]);
    assert_pixels(read_image().unwrap().unwrap(), &[1, 2, 3, 255]);

    publish(&[(png, b"not a PNG"), (DIB, &info)]);
    assert!(read_image().unwrap_err().to_string().contains("signature"));
    {
        let _clipboard = Clipboard::open().unwrap();
        assert_ne!(unsafe { IsClipboardFormatAvailable(DIB) }, 0);
    }
    let mut invalid = v5.clone();
    put_u32(&mut invalid, 4, 0);
    publish(&[(DIB_V5, &invalid)]);
    assert!(
        read_image().is_err(),
        "malformed advertised image became text/empty"
    );
    {
        let _clipboard = Clipboard::open().unwrap();
        assert_ne!(unsafe { EmptyClipboard() }, 0);
        set(
            png,
            b"\x89PNG\r\n\x1a\n",
            crate::pasted_image::MAX_IMAGE_BYTES + 1,
        );
    }
    assert!(read_image().unwrap_err().to_string().contains("64 MiB"));
    publish(&[]);
    assert!(
        read_image().unwrap().is_none(),
        "failed reads left the worker busy"
    );
}

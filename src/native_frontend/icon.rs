// SPDX-License-Identifier: MPL-2.0

//! Desktop identity is shared with the launcher and macOS bundle. GPUI 0.2.2
//! has no icon setter, and its X11 raw-window-handle methods panic. Match only
//! this process's named window through bounded standard X11 properties.

pub const APP_ID: &str = "com.runyte.Runyte";

pub fn install() {
    #[cfg(target_os = "linux")]
    {
        // GPUI chooses Wayland whenever WAYLAND_DISPLAY is nonempty. That
        // backend uses the matching desktop entry, never an X11 connection.
        if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty())
            || std::env::var_os("DISPLAY").is_none_or(|value| value.is_empty())
        {
            return;
        }
        let _ = std::thread::Builder::new()
            .name("runyte-window-icon".into())
            .spawn(|| {
                if let Err(error) = x11::install() {
                    eprintln!("Runyte window icon: {error}");
                }
            });
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use super::APP_ID;
    use x11rb::{
        connection::Connection,
        protocol::xproto::{AtomEnum, ConnectionExt, PropMode},
        wrapper::ConnectionExt as _,
    };

    const MAX_WINDOWS: usize = 2048;
    const MAX_DEPTH: usize = 4;

    fn matches_class(value: &[u8]) -> bool {
        // ICCCM normally terminates both strings; GPUI 0.2.2 omits the final
        // NUL. Accept that exact spelling and the standard terminated form.
        let value = value.strip_suffix(&[0]).unwrap_or(value);
        value == format!("{APP_ID}\0{APP_ID}").as_bytes()
    }

    fn pixels() -> anyhow::Result<Vec<u32>> {
        let image = image::load_from_memory_with_format(
            include_bytes!("../../contrib/native/icons/runyte-128.png"),
            image::ImageFormat::Png,
        )?
        .into_rgba8();
        let mut data = Vec::with_capacity(2 + image.len() / 4);
        data.extend([image.width(), image.height()]);
        data.extend(image.pixels().map(|pixel| {
            let [red, green, blue, alpha] = pixel.0;
            u32::from_be_bytes([alpha, red, green, blue])
        }));
        Ok(data)
    }

    pub(super) fn install() -> anyhow::Result<()> {
        let (connection, screen) = x11rb::connect(None)?;
        let pid_atom = connection.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
        let icon_atom = connection
            .intern_atom(false, b"_NET_WM_ICON")?
            .reply()?
            .atom;
        let mut pending =
            std::collections::VecDeque::from([(connection.setup().roots[screen].root, 0)]);
        let mut visited = 0;
        let icon = pixels()?;
        while let Some((window, depth)) = pending.pop_front() {
            visited += 1;
            if visited > MAX_WINDOWS {
                break;
            }
            // Other applications may close windows during the scan. A failed
            // property or tree query is a vanished candidate, not an error.
            let pid = connection
                .get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1)?
                .reply();
            if pid
                .ok()
                .and_then(|reply| reply.value32().and_then(|mut values| values.next()))
                == Some(std::process::id())
            {
                let class = connection
                    .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 128)?
                    .reply();
                if class.is_ok_and(|reply| reply.bytes_after == 0 && matches_class(&reply.value)) {
                    connection
                        .change_property32(
                            PropMode::REPLACE,
                            window,
                            icon_atom,
                            AtomEnum::CARDINAL,
                            &icon,
                        )?
                        .check()?;
                    connection.flush()?;
                    return Ok(());
                }
            }
            if depth < MAX_DEPTH
                && let Ok(tree) = connection.query_tree(window)?.reply()
            {
                let room = MAX_WINDOWS.saturating_sub(visited + pending.len());
                pending.extend(
                    tree.children
                        .into_iter()
                        .take(room)
                        .map(|child| (child, depth + 1)),
                );
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn class_matching_accepts_gpui_and_icccm_but_not_other_windows() {
            assert!(super::matches_class(
                b"com.runyte.Runyte\0com.runyte.Runyte"
            ));
            assert!(super::matches_class(
                b"com.runyte.Runyte\0com.runyte.Runyte\0"
            ));
            for value in [
                b"com.runyte.Runyte".as_slice(),
                b"other\0com.runyte.Runyte\0",
                b"com.runyte.Runyte\0other\0",
                b"com.runyte.Runyte\0com.runyte.Runyte\0extra",
            ] {
                assert!(!super::matches_class(value));
            }
        }

        #[test]
        fn desktop_icon_has_argb_pixels_and_a_transparent_margin() {
            let pixels = super::pixels().unwrap();
            assert_eq!(&pixels[..2], &[128, 128]);
            assert_eq!(pixels.len(), 2 + 128 * 128);
            assert_eq!(pixels[2] >> 24, 0);
            assert!(pixels[2..].contains(&0xff323232), "authored logo fill");
            assert!(pixels[2..].contains(&0xfff3f1eb), "light backplate");
        }
    }
}

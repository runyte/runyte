// SPDX-License-Identifier: MPL-2.0

use cocoa::{
    appkit::{NSApplication, NSImage},
    base::nil,
    foundation::{NSData, NSUInteger},
};
use objc::rc::StrongPtr;

const ICON: &[u8] = include_bytes!("../../../../contrib/packaging/icons/Runyte.icns");

/// Called on GPUI's main thread after AppKit has created the application.
pub fn install_application_icon() {
    if let Some(image) = image(ICON) {
        // SAFETY: GPUI's launch callback owns the main thread and an
        // autorelease pool. AppKit retains the image; our owned reference is
        // released when `image` drops, including for an unbundled launch.
        unsafe {
            NSApplication::sharedApplication(nil).setApplicationIconImage_(*image);
        }
    }
}

fn image(bytes: &[u8]) -> Option<StrongPtr> {
    // SAFETY: NSData copies the supplied bytes and is autoreleased in the
    // caller's pool. Successful alloc/init returns an owned NSImage reference,
    // which StrongPtr releases. A failed initializer releases its allocation.
    unsafe {
        let data =
            NSData::dataWithBytes_length_(nil, bytes.as_ptr().cast(), bytes.len() as NSUInteger);
        let image = NSImage::initWithData_(NSImage::alloc(nil), data);
        (image != nil).then(|| StrongPtr::new(image))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cocoa::foundation::NSAutoreleasePool;

    #[test]
    fn appkit_decodes_embedded_icon_and_rejects_invalid_data() {
        // Decode only: the test never creates an application or changes the
        // user's Dock, and needs no bundle or user configuration directories.
        unsafe {
            let pool = NSAutoreleasePool::new(nil);
            {
                let icon = image(ICON).expect("AppKit must decode the embedded ICNS");
                let size = (*icon).size();
                assert!(size.width > 0.0 && size.height > 0.0);
                assert!(image(b"invalid ICNS").is_none());
            }
            pool.drain();
        }
    }
}

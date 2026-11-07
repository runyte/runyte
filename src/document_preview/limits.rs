// SPDX-License-Identifier: MPL-2.0
//! Shared capture and private helper budgets; no renderer dependencies.

pub const MAX_BYTES: usize = 1024 * 1024;
/// JSON can encode one source byte as six bytes (`\u0000`). Leave separate
/// room for paths, the theme palette, geometry and the framing newline.
pub const MAX_HELPER_MESSAGE_BYTES: usize = 6 * MAX_BYTES + 256 * 1024;

// SPDX-License-Identifier: MPL-2.0

use super::{Bounds, Error, ErrorCode, MAX_IO_BYTES, MAX_OUTPUT_BYTES, invalid};
use std::collections::VecDeque;

#[derive(Debug)]
pub(crate) struct Ring {
    bytes: VecDeque<u8>,
    capacity: usize,
    end: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Chunk {
    pub offset: u64,
    pub bytes: Vec<u8>,
    pub next: u64,
    pub eof: bool,
}

impl Ring {
    pub fn new(capacity: usize) -> Self {
        assert!((1..=MAX_OUTPUT_BYTES).contains(&capacity));
        Self {
            bytes: VecDeque::with_capacity(capacity),
            capacity,
            end: 0,
        }
    }
    pub fn bounds(&self) -> Bounds {
        Bounds {
            start: self.end - self.bytes.len() as u64,
            end: self.end,
        }
    }
    pub fn append(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let end = self.end.checked_add(bytes.len() as u64).ok_or_else(|| {
            Error::new(ErrorCode::LimitExceeded, "Process output offset exhausted")
        })?;
        if bytes.len() >= self.capacity {
            self.bytes.clear();
            self.bytes.extend(&bytes[bytes.len() - self.capacity..]);
        } else {
            let discard = self
                .bytes
                .len()
                .saturating_add(bytes.len())
                .saturating_sub(self.capacity);
            self.bytes.drain(..discard);
            self.bytes.extend(bytes);
        }
        self.end = end;
        Ok(())
    }
    pub fn read(&self, offset: u64, limit: usize, exited: bool) -> Result<Chunk, Error> {
        if !(1..=MAX_IO_BYTES).contains(&limit) {
            return Err(invalid("Invalid process read limit"));
        }
        let bounds = self.bounds();
        if offset < bounds.start {
            return Err(Error::new(ErrorCode::Stale, "Process output was truncated"));
        }
        if offset > bounds.end {
            return Err(invalid("Process output offset is in the future"));
        }
        let start = (offset - bounds.start) as usize;
        let bytes: Vec<_> = self
            .bytes
            .range(start..self.bytes.len().min(start + limit))
            .copied()
            .collect();
        let next = offset + bytes.len() as u64;
        Ok(Chunk {
            offset,
            bytes,
            next,
            eof: exited && next == bounds.end,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exhausted_offsets_refuse_without_changing_retained_bytes() {
        let mut ring = Ring::new(8);
        ring.append(b"kept").unwrap();
        ring.end = u64::MAX;
        let before = ring.bounds();
        assert_eq!(
            ring.append(b"x").unwrap_err().code,
            ErrorCode::LimitExceeded
        );
        assert_eq!(ring.bounds(), before);
        assert_eq!(ring.read(before.start, 8, true).unwrap().bytes, b"kept");
    }
}

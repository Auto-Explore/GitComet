use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::Result;
use std::path::Path;

pub(super) fn invalid(path: &Path, offset: usize, reason: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Backend(format!(
        "reftable {}@{offset}: {reason}",
        path.display()
    )))
}

pub(super) struct Cursor<'a> {
    pub data: &'a [u8],
    pub pos: usize,
    pub base: usize,
    pub path: &'a Path,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8], path: &'a Path, base: usize) -> Self {
        Self {
            data,
            pos: 0,
            base,
            path,
        }
    }

    pub fn error(&self, reason: impl std::fmt::Display) -> Error {
        invalid(self.path, self.base.saturating_add(self.pos), reason)
    }

    pub fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(count)
            .ok_or_else(|| self.error("offset overflow"))?;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| self.error("truncated record"))?;
        self.pos = end;
        Ok(bytes)
    }

    pub fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    pub fn u24(&mut self) -> Result<usize> {
        let b = self.take(3)?;
        Ok((usize::from(b[0]) << 16) | (usize::from(b[1]) << 8) | usize::from(b[2]))
    }
    pub fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    pub fn varint(&mut self) -> Result<u64> {
        let mut byte = self.byte()?;
        let mut value = u64::from(byte & 0x7f);
        for _ in 0..10 {
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            byte = self.byte()?;
            value = value
                .checked_add(1)
                .and_then(|v| v.checked_mul(128))
                .and_then(|v| v.checked_add(u64::from(byte & 0x7f)))
                .ok_or_else(|| self.error("varint overflow"))?;
        }
        Err(self.error("varint overflow"))
    }
    pub fn sized(&mut self) -> Result<&'a [u8]> {
        let size = usize::try_from(self.varint()?).map_err(|_| self.error("length overflow"))?;
        self.take(size)
    }
    pub fn oid(&mut self, hash: gix::hash::Kind) -> Result<gix::ObjectId> {
        gix::ObjectId::try_from(self.take(hash.len_in_bytes())?).map_err(|e| self.error(e))
    }
}

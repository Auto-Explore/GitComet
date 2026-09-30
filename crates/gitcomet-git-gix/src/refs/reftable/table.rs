use super::bytes::{Cursor, invalid};
use gitcomet_core::services::{CancellationToken, Result};
use gix::bstr::ByteSlice as _;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) struct Record {
    pub target: gix::refs::Target,
    pub peeled: Option<gix::ObjectId>,
}

pub(super) struct Table {
    pub path: PathBuf,
    pub bytes: Arc<[u8]>,
    pub hash: gix::hash::Kind,
    pub header_len: usize,
    pub min: u64,
    pub max: u64,
    pub log_start: usize,
    pub log_end: usize,
    pub refs: BTreeMap<Vec<u8>, Option<Record>>,
}

impl Table {
    pub fn parse(
        path: PathBuf,
        bytes: Arc<[u8]>,
        hash: gix::hash::Kind,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        let mut c = Cursor::new(&bytes, &path, 0);
        if c.take(4)? != b"REFT" {
            return Err(c.error("invalid magic"));
        }
        let version = c.byte()?;
        let header_len = match version {
            1 => 24,
            2 => 28,
            _ => return Err(c.error("unsupported version")),
        };
        let alignment = c.u24()?;
        let min = c.u64()?;
        let max = c.u64()?;
        if min > max {
            return Err(c.error("inverted update-index bounds"));
        }
        let encoded_hash = if version == 1 {
            b"sha1".as_slice()
        } else {
            c.take(4)?
        };
        let table_hash = match encoded_hash {
            b"sha1" => gix::hash::Kind::Sha1,
            b"s256" => gix::hash::Kind::Sha256,
            _ => return Err(c.error("unsupported object hash")),
        };
        if hash != table_hash {
            return Err(c.error("repository/table object format mismatch"));
        }
        let footer_len = header_len + 44;
        let footer_start = bytes
            .len()
            .checked_sub(footer_len)
            .filter(|n| *n >= header_len)
            .ok_or_else(|| c.error("truncated footer"))?;
        let footer = &bytes[footer_start..];
        if footer[..header_len] != bytes[..header_len] {
            return Err(c.error("header/footer mismatch"));
        }
        let mut f = Cursor::new(footer, &path, footer_start);
        f.take(header_len)?;
        let ref_index = f.u64()?;
        let object = f.u64()? >> 5;
        let object_index = f.u64()?;
        let log = f.u64()?;
        let log_index = f.u64()?;
        let crc = f.u32()?;
        if crc32fast::hash(&footer[..footer_len - 4]) != crc {
            return Err(f.error("invalid footer CRC"));
        }
        let mut positions = [0usize; 5];
        for (out, pos) in
            positions
                .iter_mut()
                .zip([ref_index, object, object_index, log, log_index])
        {
            *out = usize::try_from(pos).map_err(|_| f.error("section offset overflow"))?;
            if *out != 0 && (*out < header_len || *out >= footer_start) {
                return Err(f.error("section outside table"));
            }
        }
        let nonzero: Vec<_> = positions.iter().copied().filter(|v| *v != 0).collect();
        if nonzero.windows(2).any(|v| v[0] >= v[1]) {
            return Err(f.error("out-of-order sections"));
        }
        let refs_end = nonzero.first().copied().unwrap_or(footer_start);
        let mut out = Self {
            path,
            bytes,
            hash,
            header_len,
            min,
            max,
            log_start: positions[3],
            log_end: if positions[4] == 0 {
                footer_start
            } else {
                positions[4]
            },
            refs: BTreeMap::new(),
        };
        // A log-only table's first block starts at zero and shares the header.
        if out.bytes.get(header_len) == Some(&b'g') {
            out.log_start = header_len;
            return Ok(out);
        }
        let mut base = 0usize;
        let mut prior_name = Vec::new();
        while base < refs_end && (base != 0 || header_len < refs_end) {
            cancel.check_cancelled()?;
            let header = if base == 0 { header_len } else { base };
            let mut b = Cursor::new(&out.bytes[header..refs_end], &out.path, header);
            let kind = b.byte()?;
            // The footer points at the root of a possibly multi-level index,
            // so lower index blocks can precede that offset.
            if kind == b'i' && ref_index != 0 {
                break;
            }
            if kind != b'r' {
                return Err(b.error("expected ref block"));
            }
            let len = b.u24()?;
            let end = base
                .checked_add(len)
                .filter(|n| *n <= refs_end)
                .ok_or_else(|| b.error("ref block outside section"))?;
            if end < header + 6 {
                return Err(b.error("invalid ref block length"));
            }
            for (name, record) in out.decode_refs(base, header + 4, end)? {
                if !prior_name.is_empty() && name <= prior_name {
                    return Err(b.error("unsorted or duplicate ref"));
                }
                prior_name.clone_from(&name);
                out.refs.insert(name, record);
            }
            if end == refs_end {
                break;
            }
            let next = if alignment == 0 {
                end
            } else {
                end.checked_add(alignment - 1)
                    .map(|n| n / alignment * alignment)
                    .ok_or_else(|| b.error("block alignment overflow"))?
                    .min(refs_end)
            };
            if next <= base || out.bytes[end..next].iter().any(|v| *v != 0) {
                return Err(b.error("invalid block padding"));
            }
            base = next;
        }
        Ok(out)
    }

    fn decode_refs(
        &self,
        base: usize,
        start: usize,
        end: usize,
    ) -> Result<Vec<(Vec<u8>, Option<Record>)>> {
        let (records_end, restarts) =
            restart_table(&self.bytes[base..end], start - base, &self.path, base)?;
        let mut c = Cursor::new(&self.bytes[start..base + records_end], &self.path, start);
        let mut prior = Vec::new();
        let mut entries = Vec::new();
        let mut next_restart = 0;
        while c.pos < c.data.len() {
            let offset = start - base + c.pos;
            let prefix = usize::try_from(c.varint()?).map_err(|_| c.error("prefix overflow"))?;
            let suffix = c.varint()?;
            if restarts.get(next_restart) == Some(&offset) {
                if prefix != 0 {
                    return Err(c.error("compressed restart key"));
                }
                next_restart += 1;
            } else if restarts.get(next_restart).is_some_and(|r| *r < offset) {
                return Err(c.error("restart is not a record boundary"));
            }
            let mut name = prior
                .get(..prefix)
                .ok_or_else(|| c.error("prefix exceeds previous key"))?
                .to_vec();
            let suffix_len =
                usize::try_from(suffix >> 3).map_err(|_| c.error("suffix overflow"))?;
            name.extend_from_slice(c.take(suffix_len)?);
            let full_name =
                gix::refs::FullName::try_from(name.as_bstr()).map_err(|e| c.error(e))?;
            let update = self
                .min
                .checked_add(c.varint()?)
                .ok_or_else(|| c.error("update-index overflow"))?;
            if update > self.max {
                return Err(c.error("update index outside bounds"));
            }
            let record = match suffix & 7 {
                0 => None,
                1 => Some(Record {
                    target: gix::refs::Target::Object(c.oid(self.hash)?),
                    peeled: None,
                }),
                2 => Some(Record {
                    target: gix::refs::Target::Object(c.oid(self.hash)?),
                    peeled: Some(c.oid(self.hash)?),
                }),
                3 => {
                    let target = gix::refs::FullName::try_from(c.sized()?.as_bstr())
                        .map_err(|e| c.error(e))?;
                    Some(Record {
                        target: gix::refs::Target::Symbolic(target),
                        peeled: None,
                    })
                }
                _ => return Err(c.error("unsupported ref value type")),
            };
            let _ = full_name;
            prior.clone_from(&name);
            entries.push((name, record));
        }
        if next_restart != restarts.len() {
            return Err(c.error("restart outside records"));
        }
        Ok(entries)
    }

    pub fn resident_bytes(&self) -> usize {
        self.bytes.len().saturating_add(
            self.refs
                .iter()
                .map(|(name, record)| {
                    name.len()
                        + std::mem::size_of::<Record>()
                        + 96
                        + record.as_ref().map_or(0, |r| match &r.target {
                            gix::refs::Target::Symbolic(n) => n.as_bstr().len(),
                            _ => 0,
                        })
                })
                .sum::<usize>(),
        )
    }
}

pub(super) fn restart_table(
    data: &[u8],
    first: usize,
    path: &Path,
    base: usize,
) -> Result<(usize, Vec<usize>)> {
    let count_pos = data
        .len()
        .checked_sub(2)
        .ok_or_else(|| invalid(path, base, "missing restart count"))?;
    let mut tail = Cursor::new(&data[count_pos..], path, base + count_pos);
    let count = usize::from(tail.u16()?);
    if count == 0 {
        return Err(tail.error("empty restart table"));
    }
    let start = count_pos
        .checked_sub(count * 3)
        .filter(|n| *n > first)
        .ok_or_else(|| tail.error("restart table overlaps records"))?;
    let mut c = Cursor::new(&data[start..count_pos], path, base + start);
    let mut restarts = Vec::with_capacity(count);
    for _ in 0..count {
        let offset = c.u24()?;
        if offset < first || offset >= start || restarts.last().is_some_and(|p| *p >= offset) {
            return Err(c.error("invalid restart offset"));
        }
        restarts.push(offset);
    }
    if restarts[0] != first {
        return Err(c.error("first record is not a restart"));
    }
    Ok((start, restarts))
}

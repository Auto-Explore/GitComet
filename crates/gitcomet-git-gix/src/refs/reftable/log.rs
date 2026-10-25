use super::bytes::Cursor;
use super::table::{Table, restart_table};
use gitcomet_core::services::{CancellationToken, Result};

impl Table {
    pub fn logs(
        &self,
        name: &[u8],
        cancel: &CancellationToken,
    ) -> Result<Vec<(u64, Option<gix::refs::log::Line>)>> {
        if self.log_start == 0 {
            return Ok(Vec::new());
        }
        let mut pos = self.log_start;
        let mut entries = Vec::new();
        let mut previous_key = Vec::new();
        while pos < self.log_end {
            cancel.check_cancelled()?;
            let mut header = Cursor::new(&self.bytes[pos..self.log_end], &self.path, pos);
            let kind = header.byte()?;
            // Footer indexes point to the root; lower levels immediately follow
            // the data blocks and aren't needed for a sequential read.
            if kind == b'i' && self.log_end < self.bytes.len() - (self.header_len + 44) {
                break;
            }
            if kind != b'g' {
                return Err(header.error("expected log block"));
            }
            let block_len = header.u24()?;
            let header_offset = if pos == self.header_len {
                self.header_len
            } else {
                0
            };
            let first_record = header_offset + 4;
            let inflated_len = block_len
                .checked_sub(first_record)
                .ok_or_else(|| header.error("invalid log block length"))?;
            let mut block = vec![0; block_len];
            let mut inflater = flate2::Decompress::new(true);
            let status = inflater
                .decompress(
                    &header.data[4..],
                    &mut block[first_record..],
                    flate2::FlushDecompress::Finish,
                )
                .map_err(|e| header.error(e))?;
            if status != flate2::Status::StreamEnd || inflater.total_out() != inflated_len as u64 {
                return Err(header.error("invalid inflated log size or incomplete stream"));
            }
            let consumed = usize::try_from(inflater.total_in())
                .map_err(|_| header.error("compressed size overflow"))?;
            if consumed == 0 {
                return Err(header.error("empty zlib stream"));
            }
            let (records_end, restarts) = restart_table(&block, first_record, &self.path, pos)?;
            let mut c = Cursor::new(&block[first_record..records_end], &self.path, pos + 4);
            let mut prior = Vec::new();
            let mut next_restart = 0;
            while c.pos < c.data.len() {
                let offset = first_record + c.pos;
                let prefix =
                    usize::try_from(c.varint()?).map_err(|_| c.error("prefix overflow"))?;
                let suffix = c.varint()?;
                if restarts.get(next_restart) == Some(&offset) {
                    if prefix != 0 {
                        return Err(c.error("compressed log restart"));
                    }
                    next_restart += 1;
                } else if restarts.get(next_restart).is_some_and(|r| *r < offset) {
                    return Err(c.error("log restart is not a record boundary"));
                }
                let mut key = prior
                    .get(..prefix)
                    .ok_or_else(|| c.error("log prefix exceeds key"))?
                    .to_vec();
                let suffix_len =
                    usize::try_from(suffix >> 3).map_err(|_| c.error("log suffix overflow"))?;
                key.extend_from_slice(c.take(suffix_len)?);
                let split = key
                    .len()
                    .checked_sub(9)
                    .ok_or_else(|| c.error("short log key"))?;
                if key[split] != 0 || key[..split].contains(&0) {
                    return Err(c.error("invalid log key"));
                }
                // Keys are sorted by raw ref name before reverse update index.
                // In particular a HEAD read needn't inflate branch reflogs.
                if &key[..split] > name {
                    return Ok(entries);
                }
                if !previous_key.is_empty() && key <= previous_key {
                    return Err(c.error("unsorted log keys"));
                }
                let mut k = Cursor::new(&key[split + 1..], &self.path, pos);
                let update = u64::MAX - k.u64()?;
                // Reflog expiry/drop writes tombstones for older update indexes
                // into a newer table; header bounds describe ref updates only.
                let line = match suffix & 7 {
                    0 => None,
                    1 => {
                        let previous_oid = c.oid(self.hash)?;
                        let new_oid = c.oid(self.hash)?;
                        let actor = c.sized()?.to_vec();
                        let email = c.sized()?.to_vec();
                        let seconds = i64::try_from(c.varint()?)
                            .map_err(|_| c.error("timestamp overflow"))?;
                        // Git's writer stores signed HHMM, despite the format document's minutes.
                        let hhmm = i32::from(c.u16()? as i16);
                        if hhmm.abs() % 100 >= 60 {
                            return Err(c.error("invalid timezone minutes"));
                        }
                        let offset = (hhmm / 100 * 60 + hhmm % 100) * 60;
                        let message = c.sized()?;
                        let message = message.strip_suffix(b"\n").unwrap_or(message).into();
                        Some(gix::refs::log::Line {
                            previous_oid,
                            new_oid,
                            signature: gix::actor::Signature {
                                name: actor.into(),
                                email: email.into(),
                                time: gix::date::Time { seconds, offset },
                            },
                            message,
                        })
                    }
                    _ => return Err(c.error("unsupported log type")),
                };
                if &key[..split] == name {
                    entries.push((update, line));
                }
                previous_key.clone_from(&key);
                prior = key;
            }
            if next_restart != restarts.len() {
                return Err(c.error("log restart outside records"));
            }
            pos = pos
                .checked_add(4)
                .and_then(|p| p.checked_add(consumed))
                .ok_or_else(|| header.error("log offset overflow"))?;
        }
        Ok(entries)
    }
}

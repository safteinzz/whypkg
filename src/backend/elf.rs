//! Which system libraries a binary loads, found without running it.
//!
//! `ldd` would answer this, but it can execute the binary it is asked about, so
//! the `DT_NEEDED` names are read straight out of the ELF file, and each name is
//! placed where the loader would find it through `ldconfig -p`.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The libraries `path` asks the loader for (`libc.so.6`), in the order the
/// file lists them. Empty for anything that is not a 64-bit little-endian ELF
/// file, and for a static binary, which asks for none.
pub fn needed(path: &Path) -> Vec<String> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    parse_needed(|offset, len| {
        let mut buf = vec![0; len];
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut buf).ok()?;
        Some(buf)
    })
    .unwrap_or_default()
}

/// [`needed`] over any reader of `(offset, len)` byte ranges, so a test can
/// hand it a buffer instead of a file.
pub fn parse_needed(mut read: impl FnMut(u64, usize) -> Option<Vec<u8>>) -> Option<Vec<String>> {
    const PT_LOAD: u32 = 1;
    const PT_DYNAMIC: u32 = 2;
    const DT_NULL: u64 = 0;
    const DT_NEEDED: u64 = 1;
    const DT_STRTAB: u64 = 5;
    const DT_STRSZ: u64 = 10;
    // Bounds that no real binary reaches, so a corrupt header cannot make this
    // allocate gigabytes.
    const MAX_HEADERS: usize = 4096;
    const MAX_TABLE: u64 = 16 << 20;

    let header = read(0, 64)?;
    // 64-bit (class 2), little-endian (data 1): every target cargo installs
    // to on the distros whypkg reads.
    if header[..4] != *b"\x7fELF" || header[4] != 2 || header[5] != 1 {
        return None;
    }
    let phoff = u64_at(&header, 0x20)?;
    let phentsize = u16_at(&header, 0x36)? as usize;
    let phnum = u16_at(&header, 0x38)? as usize;
    if phentsize < 56 || phnum > MAX_HEADERS {
        return None;
    }
    let table = read(phoff, phentsize * phnum)?;

    // (vaddr, offset, filesz) of every loaded segment, to turn the string
    // table's address into a place in the file.
    let mut loads = Vec::new();
    let mut dynamic = None;
    for entry in table.chunks_exact(phentsize) {
        let kind = u32_at(entry, 0)?;
        let offset = u64_at(entry, 8)?;
        let vaddr = u64_at(entry, 16)?;
        let filesz = u64_at(entry, 32)?;
        match kind {
            PT_LOAD => loads.push((vaddr, offset, filesz)),
            PT_DYNAMIC => dynamic = Some((offset, filesz)),
            _ => {}
        }
    }
    let Some((dyn_offset, dyn_size)) = dynamic else {
        return Some(Vec::new());
    };
    if dyn_size > MAX_TABLE {
        return None;
    }
    let entries = read(dyn_offset, dyn_size as usize)?;

    let mut names = Vec::new();
    let (mut strtab, mut strsz) = (None, None);
    for entry in entries.as_chunks::<16>().0 {
        let tag = u64_at(entry, 0)?;
        let value = u64_at(entry, 8)?;
        match tag {
            DT_NULL => break,
            DT_NEEDED => names.push(value),
            DT_STRTAB => strtab = Some(value),
            DT_STRSZ => strsz = Some(value),
            _ => {}
        }
    }
    let (Some(addr), Some(size)) = (strtab, strsz) else {
        return Some(Vec::new());
    };
    if size > MAX_TABLE {
        return None;
    }
    let (vaddr, offset, _) = loads
        .iter()
        .find(|(vaddr, _, filesz)| (*vaddr..vaddr + filesz).contains(&addr))?;
    let strings = read(addr - vaddr + offset, size as usize)?;
    Some(
        names
            .into_iter()
            .filter_map(|at| {
                let rest = strings.get(at as usize..)?;
                let end = rest.iter().position(|b| *b == 0)?;
                Some(String::from_utf8_lossy(&rest[..end]).into_owned())
            })
            .collect(),
    )
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

/// Where the loader finds libraries by name, with symlinks resolved: the
/// loader's own cache (`ldconfig -p`) first, then the usual directories for a
/// name the cache does not hold.
pub struct Libraries {
    cache: HashMap<String, PathBuf>,
}

impl Libraries {
    /// Read the loader's cache. `ldconfig` lives in `/sbin` on distros that
    /// keep it off a user's `PATH`.
    pub fn load() -> Libraries {
        let text = ["ldconfig", "/sbin/ldconfig", "/usr/sbin/ldconfig"]
            .iter()
            .find_map(|bin| {
                Command::new(bin)
                    .arg("-p")
                    .env("LC_ALL", "C")
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
            })
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        Libraries {
            cache: parse_ldconfig(&text, abi_marker()),
        }
    }

    /// The file the loader would map for `soname`, or `None` when it would
    /// fail to find one.
    pub fn find(&self, soname: &str) -> Option<PathBuf> {
        let path = self.cache.get(soname).cloned().or_else(|| {
            let arch = std::env::consts::ARCH;
            [
                format!("/usr/lib/{arch}-linux-gnu"),
                format!("/lib/{arch}-linux-gnu"),
                "/usr/lib64".to_string(),
                "/lib64".to_string(),
                "/usr/lib".to_string(),
                "/lib".to_string(),
            ]
            .into_iter()
            .map(|dir| Path::new(&dir).join(soname))
            .find(|p| p.exists())
        })?;
        path.canonicalize().ok()
    }
}

/// The tag `ldconfig -p` puts on this machine's own libraries, so a 32-bit
/// copy of the same name is never taken for the one a 64-bit binary loads.
fn abi_marker() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x86-64"),
        "aarch64" => Some("AArch64"),
        _ => None,
    }
}

/// Parse `ldconfig -p` lines (`\tlibssl.so.3 (libc6,x86-64) => /lib/…/libssl.so.3`)
/// into the first path for each name, keeping only lines whose tag holds
/// `abi` when one is given.
pub fn parse_ldconfig(text: &str, abi: Option<&str>) -> HashMap<String, PathBuf> {
    let mut cache = HashMap::new();
    for line in text.lines() {
        let Some((left, path)) = line.split_once(" => ") else {
            continue;
        };
        let Some((name, tag)) = left.trim().split_once(' ') else {
            continue;
        };
        if abi.is_some_and(|abi| !tag.contains(abi)) {
            continue;
        }
        cache
            .entry(name.to_string())
            .or_insert_with(|| PathBuf::from(path.trim()));
    }
    cache
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal 64-bit little-endian ELF file: one loaded segment holding a
    /// string table and a dynamic section that names `needs`.
    fn elf(needs: &[&str]) -> Vec<u8> {
        const BASE: u64 = 0x40_0000;
        let mut strtab = vec![0u8];
        let mut offsets = Vec::new();
        for name in needs {
            offsets.push(strtab.len() as u64);
            strtab.extend_from_slice(name.as_bytes());
            strtab.push(0);
        }
        let (phoff, phnum) = (64u64, 2u16);
        let str_off = phoff + 56 * u64::from(phnum);
        let dyn_off = str_off + strtab.len() as u64;
        let mut dynamic = Vec::new();
        let mut entry = |tag: u64, value: u64| {
            dynamic.extend_from_slice(&tag.to_le_bytes());
            dynamic.extend_from_slice(&value.to_le_bytes());
        };
        for at in &offsets {
            entry(1, *at);
        }
        entry(5, BASE + str_off);
        entry(10, strtab.len() as u64);
        entry(0, 0);
        let total = dyn_off + dynamic.len() as u64;

        let mut out = vec![0u8; 64];
        out[..4].copy_from_slice(b"\x7fELF");
        out[4] = 2;
        out[5] = 1;
        out[0x20..0x28].copy_from_slice(&phoff.to_le_bytes());
        out[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
        out[0x38..0x3a].copy_from_slice(&phnum.to_le_bytes());
        let mut header = |kind: u32, offset: u64, vaddr: u64, size: u64| {
            let mut h = vec![0u8; 56];
            h[..4].copy_from_slice(&kind.to_le_bytes());
            h[8..16].copy_from_slice(&offset.to_le_bytes());
            h[16..24].copy_from_slice(&vaddr.to_le_bytes());
            h[32..40].copy_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&h);
        };
        header(1, 0, BASE, total);
        header(2, dyn_off, BASE + dyn_off, dynamic.len() as u64);
        out.extend_from_slice(&strtab);
        out.extend_from_slice(&dynamic);
        out
    }

    fn read_from(bytes: &[u8]) -> impl FnMut(u64, usize) -> Option<Vec<u8>> + '_ {
        move |offset, len| {
            let start = usize::try_from(offset).ok()?;
            bytes
                .get(start..start.checked_add(len)?)
                .map(<[u8]>::to_vec)
        }
    }

    #[test]
    fn needed_libraries_come_out_of_the_dynamic_section() {
        let file = elf(&["libc.so.6", "libssl.so.3"]);
        assert_eq!(
            parse_needed(read_from(&file)),
            Some(vec!["libc.so.6".to_string(), "libssl.so.3".to_string()])
        );
    }

    #[test]
    fn a_corrupt_header_yields_nothing() {
        let mut file = elf(&["libc.so.6"]);
        // More program headers than any real binary has.
        file[0x38..0x3a].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(
            parse_needed(read_from(&file)),
            None,
            "an absurd header count"
        );

        let whole = elf(&["libc.so.6"]);
        assert_eq!(
            parse_needed(read_from(&whole[..whole.len() - 20])),
            None,
            "a file cut short"
        );
        assert_eq!(
            parse_needed(read_from(b"#!/bin/sh\n")),
            None,
            "not an ELF file"
        );
    }
}

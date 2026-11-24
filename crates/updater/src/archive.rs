//! Extract only bounded ordinary files into a fresh, private staging folder.
use crate::release::{Asset, MAX_DOWNLOAD, suffix};
use crate::{Result, require};
use flate2::read::MultiGzDecoder;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAX_EXPANDED: u64 = 4 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 10000;

struct Extractor<'a> {
    destination: &'a Path,
    root: &'a str,
    paths: HashMap<String, (String, bool)>,
    seen: HashSet<String>,
    total: u64,
}

fn safe_component(part: &str) -> bool {
    if part.is_empty()
        || matches!(part, "." | "..")
        || part.trim() != part
        || part.ends_with('.')
        || part
            .bytes()
            .any(|c| !(32..127).contains(&c) || b"\\<>:\"|?*~".contains(&c))
    {
        return false;
    }
    let stem = part
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    !matches!(
        stem.as_str(),
        "CON" | "CONIN$" | "CONOUT$" | "CLOCK$" | "PRN" | "AUX" | "NUL"
    ) && !((stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && stem.as_bytes()[3].is_ascii_digit())
}

impl Extractor<'_> {
    fn extract(
        &mut self,
        name: &str,
        directory: bool,
        size: u64,
        mode: u32,
        source: &mut dyn Read,
    ) -> Result<()> {
        require(name.len() <= 240, "archive path is too long")?;
        let name = if directory {
            name.strip_suffix('/').unwrap_or(name)
        } else {
            name
        };
        let parts: Vec<_> = name.split('/').collect();
        require(
            parts[0] == self.root && parts.len() <= 32,
            "invalid archive root or path length",
        )?;
        require(
            parts.iter().all(|part| safe_component(part)),
            "unsafe archive path",
        )?;
        require(
            parts.len() > 1 || directory,
            "archive root must be a directory",
        )?;
        require(
            self.seen.len() < MAX_FILES && self.seen.insert(name.to_ascii_lowercase()),
            "duplicate archive path or too many entries",
        )?;
        for end in 1..=parts.len() {
            let name = parts[..end].join("/");
            let identity = (name.clone(), end < parts.len() || directory);
            if let Some(previous) = self
                .paths
                .insert(name.to_ascii_lowercase(), identity.clone())
            {
                require(
                    previous == identity,
                    "archive case or file/directory collision",
                )?;
            }
        }
        self.total = self
            .total
            .checked_add(size)
            .ok_or("expanded archive size overflow")?;
        require(
            size <= MAX_DOWNLOAD && self.total <= MAX_EXPANDED,
            "expanded files exceeded their limit",
        )?;
        let target = self.destination.join(name);
        if directory {
            require(size == 0, "directory contains data")?;
            fs::create_dir_all(&target)?;
        } else {
            fs::create_dir_all(target.parent().ok_or("invalid archive path")?)?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            let mut copied = 0_u64;
            let mut buffer = [0_u8; 65536];
            loop {
                let capacity = buffer.len().min((size - copied + 1) as usize);
                let read = source.read(&mut buffer[..capacity])?;
                copied += read as u64;
                require(copied <= size, "oversized archive member")?;
                if read == 0 {
                    break;
                }
                output.write_all(&buffer[..read])?;
            }
            require(copied == size, "truncated archive member")?;
            output.sync_all()?;
            executable_permissions(&target, mode)?;
        }
        Ok(())
    }
}

fn executable_permissions(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if mode & 0o111 != 0 { 0o755 } else { 0o644 }),
        )?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

// Validate the directory's resource bounds before ZipArchive allocates it.
fn zip_count(file: &mut File) -> Result<usize> {
    let length = file.metadata()?.len();
    let start = length.saturating_sub(65557);
    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    let offset = tail
        .windows(4)
        .rposition(|value| value == b"PK\x05\x06")
        .ok_or("invalid zip directory")?;
    require(tail.len() - offset >= 22, "truncated zip directory")?;
    let word = |offset: usize| u16::from_le_bytes([tail[offset], tail[offset + 1]]);
    let dword = |offset: usize| {
        u32::from_le_bytes([
            tail[offset],
            tail[offset + 1],
            tail[offset + 2],
            tail[offset + 3],
        ])
    };
    let count = usize::from(word(offset + 10));
    let directory_size = u64::from(dword(offset + 12));
    let position = u64::from(dword(offset + 16));
    require(
        word(offset + 4) == 0
            && word(offset + 6) == 0
            && usize::from(word(offset + 8)) == count
            && count <= MAX_FILES
            && directory_size <= 4 * 1024 * 1024
            && position + directory_size == start + offset as u64
            && (offset < 20 || &tail[offset - 20..offset - 16] != b"PK\x06\x07")
            && tail.len() - offset == 22 + usize::from(word(offset + 20)),
        "oversized or unsupported zip directory",
    )?;
    file.seek(SeekFrom::Start(0))?;
    Ok(count)
}

fn unpack_zip(path: &Path, extractor: &mut Extractor<'_>) -> Result<()> {
    let mut file = File::open(path)?;
    let count = zip_count(&mut file)?;
    let mut packed = zip::ZipArchive::new(file)?;
    require(
        packed.len() == count && !packed.has_duplicate_names(),
        "zip directory count mismatch or duplicate names",
    )?;
    for index in 0..packed.len() {
        let mut entry = packed.by_index(index)?;
        let name = std::str::from_utf8(entry.name_raw())?.to_string();
        let directory = entry.is_dir();
        let mode = entry.unix_mode().unwrap_or(0);
        let kind = mode & 0o170000;
        require(
            !entry.encrypted()
                && (kind == 0 || kind == if directory { 0o040000 } else { 0o100000 })
                && matches!(
                    entry.compression(),
                    zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
                )
                && entry.external_attributes() & 0x400 == 0,
            "unsupported zip entry type, compression or encryption",
        )?;
        let size = entry.size();
        extractor.extract(&name, directory, size, mode, &mut entry)?;
    }
    Ok(())
}

struct TarReader<R> {
    source: R,
    position: u64,
    last_zero: bool,
}

impl<R: Read> Read for TarReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.source.read(buffer)?;
        self.position += read as u64;
        if self.position > MAX_EXPANDED {
            return Err(std::io::Error::other("expanded archive exceeded its limit"));
        }
        self.last_zero = read > 0 && buffer[..read].iter().all(|byte| *byte == 0);
        Ok(read)
    }
}

fn unpack_tar(path: &Path, extractor: &mut Extractor<'_>) -> Result<()> {
    let reader = TarReader {
        source: MultiGzDecoder::new(File::open(path)?),
        position: 0,
        last_zero: false,
    };
    let mut packed = tar::Archive::new(reader);
    let mut pending_name = None;
    let mut pending_metadata = false;
    // Raw iteration bounds extended metadata before the tar library allocates
    // it. Links and sparse descriptors never reach its automatic unpacker.
    for (index, entry) in packed.entries()?.raw(true).enumerate() {
        require(index < MAX_FILES, "too many tar entries")?;
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        let size = entry.size();
        if kind.is_gnu_longname() {
            require(!pending_metadata && size <= 241, "invalid GNU long name")?;
            let mut name = Vec::new();
            entry.read_to_end(&mut name)?;
            require(name.last() == Some(&0), "unterminated GNU long name")?;
            name.pop();
            pending_name = Some(String::from_utf8(name)?);
            pending_metadata = true;
            continue;
        }
        if kind.is_pax_local_extensions() {
            require(
                !pending_metadata && size <= 16384,
                "oversized or duplicate tar metadata",
            )?;
            let mut keys = HashSet::new();
            for item in entry.pax_extensions()?.ok_or("missing tar extensions")? {
                let item = item?;
                let key = item.key()?;
                require(keys.insert(key.to_string()), "duplicate tar metadata key")?;
                match key {
                    "path" => {
                        pending_name = Some(item.value()?.to_string());
                    }
                    // These are deliberately ignored: we install no ownership,
                    // timestamps, xattrs, ACLs or host-specific permissions.
                    "mtime" | "atime" | "ctime" | "uid" | "gid" | "uname" | "gname" => {}
                    _ => {
                        return Err(
                            "unsupported tar metadata (including sparse files and links)".into(),
                        );
                    }
                }
            }
            pending_metadata = true;
            continue;
        }
        require(
            kind.is_file() || kind.is_dir(),
            "tar links, devices, sparse files and global metadata are unsupported",
        )?;
        let name = match pending_name.take() {
            Some(name) => name,
            None => std::str::from_utf8(&entry.path_bytes())?.to_string(),
        };
        pending_metadata = false;
        let mode = entry.header().mode()?;
        extractor.extract(&name, kind.is_dir(), size, mode, &mut entry)?;
    }
    require(!pending_metadata, "orphaned tar metadata")?;
    let mut reader = packed.into_inner();
    require(reader.last_zero, "missing tar end marker")?;
    let mut padding = 0;
    let mut buffer = [0_u8; 65536];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        require(
            buffer[..read].iter().all(|byte| *byte == 0),
            "nonzero data after tar end",
        )?;
        padding += read;
    }
    require(
        padding >= 512 && reader.position.is_multiple_of(512),
        "truncated tar end",
    )?;
    Ok(())
}

pub(crate) fn unpack(archive: &Path, destination: &Path, asset: &Asset) -> Result<PathBuf> {
    let suffix = suffix(&asset.platform)?;
    let root = asset
        .name
        .strip_suffix(suffix)
        .ok_or("invalid archive suffix")?;
    let mut extractor = Extractor {
        destination,
        root,
        paths: HashMap::new(),
        seen: HashSet::new(),
        total: 0,
    };
    if suffix == ".zip" {
        unpack_zip(archive, &mut extractor)?;
    } else {
        unpack_tar(archive, &mut extractor)?;
    }
    Ok(destination.join(root))
}

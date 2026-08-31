//! Private, crash-tolerant evidence storage.
//!
//! The store never treats a path supplied by a caller as a filesystem path.
//! Session and capture IDs are validated opaque components and every object
//! path is resolved beneath a pre-created private session directory.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use surfacecheck_core::{
    from_json, EvidenceManifest, MAX_CAPTURE_COUNT, MAX_EVIDENCE_BUNDLE_BYTES, MAX_IMAGE_BYTES,
    MAX_STORED_SESSIONS,
};

const SESSION_DIR: &str = "sessions";
const MANIFEST_FILE: &str = "manifest.json";
const JOURNAL_FILE: &str = "journal.jsonl";
const CAPTURE_DIR: &str = "captures";
const TAR_BLOCK: usize = 512;

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub root: PathBuf,
    pub max_sessions: usize,
    pub max_captures: usize,
    pub max_image_bytes: u64,
    pub max_bundle_bytes: u64,
}

impl StoreConfig {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_sessions: MAX_STORED_SESSIONS,
            max_captures: MAX_CAPTURE_COUNT,
            max_image_bytes: MAX_IMAGE_BYTES,
            max_bundle_bytes: MAX_EVIDENCE_BUNDLE_BYTES,
        }
    }
}

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    InvalidPath(String),
    Exists(String),
    Quota(String),
    Journal(String),
    Archive(String),
    Manifest(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "storage I/O failed: {error}"),
            Self::InvalidPath(message) => write!(f, "invalid storage path: {message}"),
            Self::Exists(path) => write!(f, "evidence object already exists: {path}"),
            Self::Quota(message) => write!(f, "storage quota exceeded: {message}"),
            Self::Journal(message) => write!(f, "journal is invalid: {message}"),
            Self::Archive(message) => write!(f, "archive is invalid: {message}"),
            Self::Manifest(message) => write!(f, "manifest is invalid: {message}"),
        }
    }
}

impl Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JournalOperation {
    SessionCreated,
    CapturePublished,
    ManifestPublished,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JournalBody {
    sequence: u64,
    operation: JournalOperation,
    relative_path: String,
    content_sha256: String,
    previous_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JournalLine {
    body: JournalBody,
    entry_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalRecovery {
    pub entries: Vec<JournalEntry>,
    pub ignored_truncated_tail: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    pub sequence: u64,
    pub operation: JournalOperation,
    pub relative_path: String,
    pub content_sha256: String,
    pub entry_sha256: String,
}

#[derive(Debug, Clone)]
pub struct EvidenceStore {
    config: StoreConfig,
}

impl EvidenceStore {
    pub fn new(config: StoreConfig) -> Result<Self, StoreError> {
        create_private_dir(&config.root)?;
        let sessions = config.root.join(SESSION_DIR);
        create_private_dir(&sessions)?;
        Ok(Self { config })
    }

    pub fn config(&self) -> &StoreConfig {
        &self.config
    }

    pub fn create_session(&self, session_id: &str) -> Result<PathBuf, StoreError> {
        validate_component(session_id, "session_id")?;
        let sessions = self.config.root.join(SESSION_DIR);
        reject_symlink(&sessions)?;
        let count = fs::read_dir(&sessions)?.try_fold(0usize, |count, entry| {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            Ok::<_, io::Error>(
                count + usize::from(metadata.is_dir() && !metadata.file_type().is_symlink()),
            )
        })?;
        if count >= self.config.max_sessions {
            return Err(StoreError::Quota("stored session limit reached".into()));
        }
        let session = sessions.join(session_id);
        if session.exists() {
            return Err(StoreError::Exists(session_id.to_owned()));
        }
        fs::create_dir(&session)?;
        set_private_mode(&session, true)?;
        create_private_dir(&session.join(CAPTURE_DIR))?;
        create_private_file(&session.join(JOURNAL_FILE))?;
        append_journal(&session, JournalOperation::SessionCreated, "", b"")?;
        Ok(session)
    }

    pub fn session_path(&self, session_id: &str) -> Result<PathBuf, StoreError> {
        validate_component(session_id, "session_id")?;
        let session = self.config.root.join(SESSION_DIR).join(session_id);
        ensure_session(&session)?;
        Ok(session)
    }

    pub fn publish_capture(
        &self,
        session_id: &str,
        capture_id: &str,
        image: &[u8],
        manifest: &[u8],
    ) -> Result<(), StoreError> {
        validate_component(capture_id, "capture_id")?;
        if image.is_empty()
            || u64::try_from(image.len())
                .map_err(|_| StoreError::Quota("image length overflows".into()))?
                > self.config.max_image_bytes
        {
            return Err(StoreError::Quota(
                "image exceeds the configured limit".into(),
            ));
        }
        let session = self.session_path(session_id)?;
        let capture_path = session.join(CAPTURE_DIR).join(format!("{capture_id}.png"));
        ensure_no_symlink_path(&session, &capture_path)?;
        if capture_path.exists() {
            return Err(StoreError::Exists(format!("captures/{capture_id}.png")));
        }
        validate_manifest(manifest)?;
        let current = session_size(&session)?;
        let projected = current
            .checked_add(
                u64::try_from(image.len())
                    .map_err(|_| StoreError::Quota("image length overflows".into()))?,
            )
            .and_then(|value| {
                value.checked_add(
                    u64::try_from(manifest.len())
                        .map_err(|_| StoreError::Quota("manifest length overflows".into()))
                        .ok()?,
                )
            })
            .ok_or_else(|| StoreError::Quota("bundle size overflows".into()))?;
        if projected > self.config.max_bundle_bytes {
            return Err(StoreError::Quota(
                "bundle size limit reached; evidence was not evicted".into(),
            ));
        }
        atomic_create(&capture_path, image)?;
        append_journal(
            &session,
            JournalOperation::CapturePublished,
            &format!("{CAPTURE_DIR}/{capture_id}.png"),
            image,
        )?;
        let manifest_path = session.join(MANIFEST_FILE);
        atomic_replace(&manifest_path, manifest)?;
        append_journal(
            &session,
            JournalOperation::ManifestPublished,
            MANIFEST_FILE,
            manifest,
        )?;
        Ok(())
    }

    pub fn read_manifest(&self, session_id: &str) -> Result<Vec<u8>, StoreError> {
        let session = self.session_path(session_id)?;
        let path = session.join(MANIFEST_FILE);
        ensure_no_symlink_path(&session, &path)?;
        Ok(fs::read(path)?)
    }

    pub fn recover(&self, session_id: &str) -> Result<JournalRecovery, StoreError> {
        let session = self.session_path(session_id)?;
        recover_journal(&session)
    }

    pub fn export(&self, session_id: &str) -> Result<Vec<u8>, StoreError> {
        let session = self.session_path(session_id)?;
        let manifest_bytes = self.read_manifest(session_id)?;
        let manifest: EvidenceManifest =
            from_json(&manifest_bytes).map_err(|error| StoreError::Manifest(error.to_string()))?;
        let mut entries = BTreeMap::new();
        entries.insert(MANIFEST_FILE.to_owned(), manifest_bytes);
        for capture in &manifest.captures {
            let path = session.join(&capture.image.relative_path);
            ensure_no_symlink_path(&session, &path)?;
            let bytes = fs::read(&path)?;
            if u64::try_from(bytes.len())
                .map_err(|_| StoreError::Archive("image length overflows".into()))?
                != capture.image.bytes
                || sha256_hex(&bytes) != capture.image.sha256
            {
                return Err(StoreError::Archive(format!(
                    "image checksum mismatch for {}",
                    capture.capture_id
                )));
            }
            entries.insert(capture.image.relative_path.clone(), bytes);
        }
        let archive = build_tar(&entries)?;
        verify_archive(&archive)?;
        Ok(archive)
    }
}

fn validate_component(value: &str, field: &str) -> Result<(), StoreError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(StoreError::InvalidPath(format!(
            "{field} is not a safe opaque identifier"
        )));
    }
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<(), StoreError> {
    if path.exists() {
        reject_symlink(path)?;
        if !fs::metadata(path)?.is_dir() {
            return Err(StoreError::InvalidPath(format!(
                "{} is not a directory",
                path.display()
            )));
        }
    } else {
        fs::create_dir_all(path)?;
    }
    reject_symlink(path)?;
    set_private_mode(path, true)
}

fn create_private_file(path: &Path) -> Result<(), StoreError> {
    reject_symlink(path)?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(StoreError::Io)?;
    set_private_mode(path, false)?;
    drop(file);
    Ok(())
}

fn ensure_session(path: &Path) -> Result<(), StoreError> {
    reject_symlink(path)?;
    let metadata = fs::metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            StoreError::InvalidPath("session does not exist".into())
        } else {
            StoreError::Io(error)
        }
    })?;
    if !metadata.is_dir() {
        return Err(StoreError::InvalidPath("session is not a directory".into()));
    }
    ensure_no_symlink_path(path, &path.join(CAPTURE_DIR))?;
    ensure_no_symlink_path(path, &path.join(JOURNAL_FILE))?;
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), StoreError> {
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(StoreError::InvalidPath(format!(
            "symlink is not allowed: {}",
            path.display()
        )));
    }
    Ok(())
}

fn ensure_no_symlink_path(root: &Path, target: &Path) -> Result<(), StoreError> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| StoreError::InvalidPath("path escapes session".into()))?;
    let mut current = root.to_path_buf();
    reject_symlink(&current)?;
    for component in relative.components() {
        let Component::Normal(value) = component else {
            return Err(StoreError::InvalidPath(
                "path contains traversal or non-normal component".into(),
            ));
        };
        current.push(value);
        if current.exists() {
            reject_symlink(&current)?;
            let metadata = fs::symlink_metadata(&current)?;
            if !metadata.is_file() && !metadata.is_dir() {
                return Err(StoreError::InvalidPath(
                    "special file is not allowed".into(),
                ));
            }
        }
    }
    Ok(())
}

fn atomic_create(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    if path.exists() {
        return Err(StoreError::Exists(path.display().to_string()));
    }
    let parent = path
        .parent()
        .ok_or_else(|| StoreError::InvalidPath("object has no parent".into()))?;
    reject_symlink(parent)?;
    let temp = parent.join(format!(".tmp-{}", sha256_hex(bytes)));
    if temp.exists() {
        return Err(StoreError::Exists(temp.display().to_string()));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    set_private_mode(&temp, false)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if path.exists() {
        let _ = fs::remove_file(&temp);
        return Err(StoreError::Exists(path.display().to_string()));
    }
    fs::rename(&temp, path)?;
    sync_directory(parent)?;
    Ok(())
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| StoreError::InvalidPath("object has no parent".into()))?;
    reject_symlink(parent)?;
    let temp = parent.join(format!(".tmp-manifest-{}", sha256_hex(bytes)));
    if temp.exists() {
        let _ = fs::remove_file(&temp);
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    set_private_mode(&temp, false)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temp, path)?;
    sync_directory(parent)?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    Ok(())
}

fn set_private_mode(path: &Path, directory: bool) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if directory { 0o700 } else { 0o600 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

fn session_size(path: &Path) -> Result<u64, StoreError> {
    let mut total = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(StoreError::InvalidPath(
                "symlink or special file in evidence store".into(),
            ));
        }
        if metadata.is_dir() {
            total = total
                .checked_add(session_size(&entry.path())?)
                .ok_or_else(|| StoreError::Quota("bundle size overflows".into()))?;
        } else {
            total = total
                .checked_add(metadata.len())
                .ok_or_else(|| StoreError::Quota("bundle size overflows".into()))?;
        }
    }
    Ok(total)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_manifest(bytes: &[u8]) -> Result<(), StoreError> {
    let _: EvidenceManifest =
        from_json(bytes).map_err(|error| StoreError::Manifest(error.to_string()))?;
    Ok(())
}

fn append_journal(
    session: &Path,
    operation: JournalOperation,
    relative_path: &str,
    content: &[u8],
) -> Result<(), StoreError> {
    if relative_path.len() > 256
        || relative_path.starts_with('/')
        || relative_path.contains('\\')
        || relative_path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
            && !relative_path.is_empty()
    {
        return Err(StoreError::Journal("journal path is not normalized".into()));
    }
    let journal_path = session.join(JOURNAL_FILE);
    ensure_no_symlink_path(session, &journal_path)?;
    let recovery = recover_journal(session)?;
    let sequence = recovery
        .entries
        .last()
        .map_or(1, |entry| entry.sequence + 1);
    let previous_sha256 = recovery
        .entries
        .last()
        .map_or_else(|| "0".repeat(64), |entry| entry.entry_sha256.clone());
    let body = JournalBody {
        sequence,
        operation,
        relative_path: relative_path.to_owned(),
        content_sha256: sha256_hex(content),
        previous_sha256,
    };
    let body_bytes =
        serde_json::to_vec(&body).map_err(|error| StoreError::Journal(error.to_string()))?;
    let line = JournalLine {
        body,
        entry_sha256: sha256_hex(&body_bytes),
    };
    let mut bytes =
        serde_json::to_vec(&line).map_err(|error| StoreError::Journal(error.to_string()))?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new().append(true).open(&journal_path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn recover_journal(session: &Path) -> Result<JournalRecovery, StoreError> {
    let path = session.join(JOURNAL_FILE);
    ensure_no_symlink_path(session, &path)?;
    let mut bytes = Vec::new();
    File::open(&path)?.read_to_end(&mut bytes)?;
    let mut entries = Vec::new();
    let mut expected_sequence = 1u64;
    let mut ignored_truncated_tail = false;
    let mut offset = 0usize;
    let mut previous_sha256 = "0".repeat(64);
    while offset < bytes.len() {
        let Some(relative_end) = bytes[offset..].iter().position(|byte| *byte == b'\n') else {
            ignored_truncated_tail = true;
            break;
        };
        let end = offset + relative_end;
        let line = &bytes[offset..end];
        offset = end + 1;
        let parsed: JournalLine =
            serde_json::from_slice(line).map_err(|error| StoreError::Journal(error.to_string()))?;
        if parsed.body.sequence != expected_sequence
            || parsed.body.relative_path.len() > 256
            || parsed.body.relative_path.starts_with('/')
            || parsed.body.relative_path.contains('\\')
            || (parsed
                .body
                .relative_path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
                && !parsed.body.relative_path.is_empty())
        {
            return Err(StoreError::Journal(
                "sequence or path validation failed".into(),
            ));
        }
        let body_bytes = serde_json::to_vec(&parsed.body)
            .map_err(|error| StoreError::Journal(error.to_string()))?;
        if parsed.body.previous_sha256 != previous_sha256
            || parsed.entry_sha256 != sha256_hex(&body_bytes)
            || parsed.body.content_sha256.len() != 64
        {
            return Err(StoreError::Journal(
                "entry checksum validation failed".into(),
            ));
        }
        entries.push(JournalEntry {
            sequence: parsed.body.sequence,
            operation: parsed.body.operation,
            relative_path: parsed.body.relative_path,
            content_sha256: parsed.body.content_sha256,
            entry_sha256: parsed.entry_sha256.clone(),
        });
        previous_sha256 = parsed.entry_sha256;
        expected_sequence += 1;
    }
    Ok(JournalRecovery {
        entries,
        ignored_truncated_tail,
    })
}

fn build_tar(entries: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, StoreError> {
    let mut hashes = String::new();
    for (path, bytes) in entries {
        validate_archive_path(path)?;
        hashes.push_str(&sha256_hex(bytes));
        hashes.push_str("  ");
        hashes.push_str(path);
        hashes.push('\n');
    }
    let mut all = entries.clone();
    all.insert("SHA256SUMS".into(), hashes.into_bytes());
    let mut archive = Vec::new();
    for (path, bytes) in all {
        append_tar_entry(&mut archive, &path, &bytes)?;
    }
    archive.extend_from_slice(&[0u8; TAR_BLOCK * 2]);
    Ok(archive)
}

fn append_tar_entry(archive: &mut Vec<u8>, path: &str, bytes: &[u8]) -> Result<(), StoreError> {
    validate_archive_path(path)?;
    if path.len() > 255 {
        return Err(StoreError::Archive("archive path is too long".into()));
    }
    let mut header = [0u8; TAR_BLOCK];
    let (name, prefix) = if path.len() <= 100 {
        (path, "")
    } else {
        let split = path
            .rfind('/')
            .ok_or_else(|| StoreError::Archive("long path has no prefix".into()))?;
        (&path[split + 1..], &path[..split])
    };
    if name.len() > 100 || prefix.len() > 155 {
        return Err(StoreError::Archive(
            "archive path cannot be represented as USTAR".into(),
        ));
    }
    header[..name.len()].copy_from_slice(name.as_bytes());
    header[100..108].copy_from_slice(b"0000600\0");
    header[108..116].copy_from_slice(b"0000000\0");
    header[116..124].copy_from_slice(b"0000000\0");
    write_octal(
        &mut header[124..136],
        u64::try_from(bytes.len())
            .map_err(|_| StoreError::Archive("entry size overflows".into()))?,
    );
    header[136..148].copy_from_slice(b"00000000000\0");
    header[148..156].fill(b' ');
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    header[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
    let checksum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
    write_octal(&mut header[148..156], u64::from(checksum));
    archive.extend_from_slice(&header);
    archive.extend_from_slice(bytes);
    let padding = (TAR_BLOCK - bytes.len() % TAR_BLOCK) % TAR_BLOCK;
    archive.extend(std::iter::repeat_n(0, padding));
    Ok(())
}

fn write_octal(destination: &mut [u8], value: u64) {
    let width = destination.len() - 1;
    let text = format!("{value:0width$o}", width = width);
    destination[..width].copy_from_slice(text.as_bytes());
    destination[width] = 0;
}

pub fn verify_archive(archive: &[u8]) -> Result<(), StoreError> {
    let entries = parse_archive_entries(archive)?;
    verify_archive_entries(&entries)
}

/// Verify an archive before writing any of its entries to disk. Existing
/// destination files are never replaced and symlink ancestors are rejected.
pub fn extract_verified_archive(archive: &[u8], destination: &Path) -> Result<(), StoreError> {
    let entries = parse_archive_entries(archive)?;
    verify_archive_entries(&entries)?;
    create_private_dir(destination)?;
    for (path, bytes) in entries {
        if path == "SHA256SUMS" {
            continue;
        }
        let target = destination.join(&path);
        ensure_no_symlink_path(destination, &target)?;
        if let Some(parent) = target.parent() {
            create_private_dir(parent)?;
        }
        atomic_create(&target, &bytes)?;
    }
    sync_directory(destination)?;
    Ok(())
}

fn parse_archive_entries(archive: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, StoreError> {
    if !archive.len().is_multiple_of(TAR_BLOCK) {
        return Err(StoreError::Archive("archive is not block aligned".into()));
    }
    let mut entries = BTreeMap::new();
    let mut offset = 0usize;
    let mut zero_blocks = 0usize;
    while offset + TAR_BLOCK <= archive.len() {
        let header = &archive[offset..offset + TAR_BLOCK];
        offset += TAR_BLOCK;
        if header.iter().all(|byte| *byte == 0) {
            zero_blocks += 1;
            if zero_blocks == 2 {
                break;
            }
            continue;
        }
        zero_blocks = 0;
        let name_end = header[..100]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(100);
        let prefix_end = header[345..500]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(155);
        let name = std::str::from_utf8(&header[..name_end])
            .map_err(|_| StoreError::Archive("archive path is not UTF-8".into()))?;
        let prefix = std::str::from_utf8(&header[345..345 + prefix_end])
            .map_err(|_| StoreError::Archive("archive prefix is not UTF-8".into()))?;
        let path = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        validate_archive_path(&path)?;
        if header[156] != b'0' && header[156] != 0 {
            return Err(StoreError::Archive(
                "links and special entries are forbidden".into(),
            ));
        }
        let size = parse_octal(&header[124..136])?;
        let size = usize::try_from(size)
            .map_err(|_| StoreError::Archive("entry size overflows".into()))?;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| StoreError::Archive("entry range overflows".into()))?;
        if end > archive.len() {
            return Err(StoreError::Archive("entry extends beyond archive".into()));
        }
        if entries
            .insert(path, archive[offset..end].to_vec())
            .is_some()
        {
            return Err(StoreError::Archive("duplicate archive entry".into()));
        }
        offset = end
            .checked_add((TAR_BLOCK - size % TAR_BLOCK) % TAR_BLOCK)
            .ok_or_else(|| StoreError::Archive("archive offset overflows".into()))?;
    }
    if zero_blocks < 2 {
        return Err(StoreError::Archive(
            "archive is missing its terminator".into(),
        ));
    }
    Ok(entries)
}

fn verify_archive_entries(entries: &BTreeMap<String, Vec<u8>>) -> Result<(), StoreError> {
    let sums = entries
        .get("SHA256SUMS")
        .ok_or_else(|| StoreError::Archive("archive is missing SHA256SUMS".into()))?;
    let sums = std::str::from_utf8(sums)
        .map_err(|_| StoreError::Archive("SHA256SUMS is not UTF-8".into()))?;
    let mut referenced = BTreeMap::new();
    for line in sums.lines() {
        let (hash, path) = line
            .split_once("  ")
            .ok_or_else(|| StoreError::Archive("malformed SHA256SUMS entry".into()))?;
        validate_archive_path(path)?;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(StoreError::Archive("invalid SHA256SUMS hash".into()));
        }
        let data = entries
            .get(path)
            .ok_or_else(|| StoreError::Archive("SHA256SUMS references a missing entry".into()))?;
        if sha256_hex(data) != hash {
            return Err(StoreError::Archive(format!("checksum mismatch for {path}")));
        }
        referenced.insert(path, ());
    }
    if referenced.len() + 1 != entries.len() {
        return Err(StoreError::Archive(
            "archive contains undeclared evidence".into(),
        ));
    }
    let manifest = entries
        .get(MANIFEST_FILE)
        .ok_or_else(|| StoreError::Archive("archive is missing manifest.json".into()))?;
    let manifest: EvidenceManifest =
        from_json(manifest).map_err(|error| StoreError::Manifest(error.to_string()))?;
    for capture in &manifest.captures {
        let image = entries.get(&capture.image.relative_path).ok_or_else(|| {
            StoreError::Archive(format!(
                "manifest references missing {}",
                capture.image.relative_path
            ))
        })?;
        if u64::try_from(image.len())
            .map_err(|_| StoreError::Archive("image length overflows".into()))?
            != capture.image.bytes
            || sha256_hex(image) != capture.image.sha256
        {
            return Err(StoreError::Archive(format!(
                "manifest image checksum mismatch for {}",
                capture.capture_id
            )));
        }
    }
    Ok(())
}

fn validate_archive_path(path: &str) -> Result<(), StoreError> {
    if path.is_empty()
        || path.len() > 256
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(StoreError::Archive(
            "archive path is absolute, traversing, or empty".into(),
        ));
    }
    Ok(())
}

fn parse_octal(bytes: &[u8]) -> Result<u64, StoreError> {
    let text = bytes
        .iter()
        .copied()
        .take_while(|byte| *byte != 0 && *byte != b' ')
        .collect::<Vec<_>>();
    if text.is_empty() || !text.iter().all(u8::is_ascii_digit) {
        return Err(StoreError::Archive("invalid octal header field".into()));
    }
    let text = std::str::from_utf8(&text)
        .map_err(|_| StoreError::Archive("invalid octal header field".into()))?;
    u64::from_str_radix(text, 8)
        .map_err(|_| StoreError::Archive("invalid octal header field".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use surfacecheck_core::{from_json, to_canonical_json, EvidenceManifest};

    fn temp_root(label: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("surfacecheck-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn manifest_for(image: &[u8]) -> Vec<u8> {
        let mut manifest: EvidenceManifest = from_json(include_bytes!(
            "../../../tests/fixtures/valid_manifest.json"
        ))
        .expect("fixture");
        manifest.captures[0].image.bytes = u64::try_from(image.len()).expect("test image length");
        manifest.captures[0].image.sha256 = sha256_hex(image);
        let checksum = manifest.captures[0].image.sha256.clone();
        for finding in &mut manifest.deterministic_findings {
            for evidence in &mut finding.evidence {
                evidence.content_sha256 = checksum.clone();
            }
        }
        for finding in &mut manifest.agent_findings {
            for evidence in &mut finding.evidence {
                evidence.content_sha256 = checksum.clone();
            }
        }
        to_canonical_json(&manifest).expect("manifest validates")
    }

    #[test]
    fn creates_private_session_and_recovers_journal() {
        let root = temp_root("private");
        let store = EvidenceStore::new(StoreConfig::new(&root)).expect("store");
        let session = store.create_session("session-1").expect("session");
        assert!(session.join(CAPTURE_DIR).is_dir());
        let recovery = store.recover("session-1").expect("journal");
        assert_eq!(recovery.entries.len(), 1);
        assert!(!recovery.ignored_truncated_tail);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&session)
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(session.join(JOURNAL_FILE))
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publishes_no_replace_capture_and_deterministic_export() {
        let image = b"synthetic-png-object";
        let manifest = manifest_for(image);
        let root_a = temp_root("export-a");
        let root_b = temp_root("export-b");
        let store_a = EvidenceStore::new(StoreConfig::new(&root_a)).expect("store");
        let store_b = EvidenceStore::new(StoreConfig::new(&root_b)).expect("store");
        store_a.create_session("session-1").expect("session");
        store_b.create_session("session-1").expect("session");
        store_a
            .publish_capture("session-1", "capture-1", image, &manifest)
            .expect("capture");
        store_b
            .publish_capture("session-1", "capture-1", image, &manifest)
            .expect("capture");
        assert!(matches!(
            store_a.publish_capture("session-1", "capture-1", image, &manifest),
            Err(StoreError::Exists(_))
        ));
        assert_eq!(
            store_a.export("session-1").expect("archive"),
            store_b.export("session-1").expect("archive")
        );
        let _ = fs::remove_dir_all(root_a);
        let _ = fs::remove_dir_all(root_b);
    }

    #[test]
    fn quota_and_symlink_components_fail_closed() {
        let image = b"abcd";
        let manifest = manifest_for(image);
        let root = temp_root("attacks");
        let mut config = StoreConfig::new(&root);
        config.max_image_bytes = 3;
        let store = EvidenceStore::new(config).expect("store");
        store.create_session("session-1").expect("session");
        assert!(matches!(
            store.publish_capture("session-1", "capture-1", image, &manifest),
            Err(StoreError::Quota(_))
        ));

        #[cfg(unix)]
        {
            let captures = root.join(SESSION_DIR).join("session-1").join(CAPTURE_DIR);
            let outside = root.join("outside");
            fs::create_dir(&outside).expect("outside");
            fs::remove_dir(&captures).expect("remove captures");
            std::os::unix::fs::symlink(&outside, &captures).expect("symlink");
            let mut permissive = StoreConfig::new(&root);
            permissive.max_image_bytes = 64;
            let store = EvidenceStore::new(permissive).expect("store");
            assert!(matches!(
                store.publish_capture("session-1", "capture-1", image, &manifest),
                Err(StoreError::InvalidPath(_))
            ));
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn truncated_journal_tail_is_reported_without_replaying_it() {
        let root = temp_root("journal-tail");
        let store = EvidenceStore::new(StoreConfig::new(&root)).expect("store");
        let session = store.create_session("session-1").expect("session");
        let mut journal = OpenOptions::new()
            .append(true)
            .open(session.join(JOURNAL_FILE))
            .expect("journal");
        journal.write_all(b"{\"body\":").expect("tail");
        journal.sync_all().expect("sync");
        let recovery = store.recover("session-1").expect("recovery");
        assert_eq!(recovery.entries.len(), 1);
        assert!(recovery.ignored_truncated_tail);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn archive_path_validation_rejects_traversal() {
        let mut entries = BTreeMap::new();
        entries.insert("../escape".to_owned(), b"bad".to_vec());
        assert!(matches!(build_tar(&entries), Err(StoreError::Archive(_))));
    }

    #[test]
    fn archive_verification_rejects_mutated_path() {
        let image = b"synthetic-png-object";
        let manifest = manifest_for(image);
        let root = temp_root("archive-path");
        let store = EvidenceStore::new(StoreConfig::new(&root)).expect("store");
        store.create_session("session-1").expect("session");
        store
            .publish_capture("session-1", "capture-1", image, &manifest)
            .expect("capture");
        let mut archive = store.export("session-1").expect("archive");
        archive[0] = b'.';
        archive[1] = b'.';
        assert!(verify_archive(&archive).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn verified_archive_extracts_without_overwriting() {
        let image = b"synthetic-png-object";
        let manifest = manifest_for(image);
        let root = temp_root("archive-extract");
        let store = EvidenceStore::new(StoreConfig::new(&root)).expect("store");
        store.create_session("session-1").expect("session");
        store
            .publish_capture("session-1", "capture-1", image, &manifest)
            .expect("capture");
        let archive = store.export("session-1").expect("archive");
        let destination = root.join("extracted");
        extract_verified_archive(&archive, &destination).expect("extract");
        assert_eq!(
            fs::read(destination.join("captures/capture-1.png")).expect("image"),
            image
        );
        assert!(matches!(
            extract_verified_archive(&archive, &destination),
            Err(StoreError::Exists(_))
        ));
        let _ = fs::remove_dir_all(root);
    }
}

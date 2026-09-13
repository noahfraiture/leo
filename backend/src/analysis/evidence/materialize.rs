use super::{CandidateFrame, EvidenceError, EvidenceManifest, SentImage};
use crate::profiles::ImageSizePolicy;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path, time::Duration};

/// Hashes without retaining the recording in RAM.
pub fn file_hash(path: &Path) -> Result<String, EvidenceError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Checks a resumed run still refers to the same recording bytes before more model work.
pub fn verify_sources(directory: &Path, manifest: &EvidenceManifest) -> Result<(), EvidenceError> {
    for source in &manifest.sources {
        let path = source_path(directory, &source.relative_path)?;
        if path.metadata()?.len() != source.size_bytes || file_hash(&path)? != source.sha256 {
            return Err(EvidenceError::Invalid(
                "A recording changed since this run was created".into(),
            ));
        }
    }
    Ok(())
}

/// Extracts candidate bytes off the UI thread and records their actual sent-byte identity.
pub fn materialize_frame(
    directory: &Path,
    manifest: &EvidenceManifest,
    frame: &CandidateFrame,
    size: ImageSizePolicy,
) -> Result<(Vec<u8>, SentImage), EvidenceError> {
    if manifest.frame(&frame.id) != Some(frame) {
        return Err(EvidenceError::Invalid("Unknown candidate frame".into()));
    }
    let source = manifest
        .sources
        .get(frame.source_index)
        .ok_or_else(|| EvidenceError::Invalid("Unknown source".into()))?;
    if source.camera_id != frame.camera_id
        || frame.recording_offset_ms >= (source.end_utc_ms - source.start_utc_ms) as u64
    {
        return Err(EvidenceError::Invalid(
            "Candidate frame is outside its segment".into(),
        ));
    }
    let path = source_path(directory, &source.relative_path)?;
    let bytes = super::super::video::extract_jpeg(
        &path,
        Duration::from_millis(frame.recording_offset_ms),
        size,
    )?;
    let sent = SentImage {
        candidate_id: frame.id.clone(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        size_bytes: bytes.len() as u64,
    };
    Ok((bytes, sent))
}

pub fn source_path(directory: &Path, relative: &Path) -> Result<std::path::PathBuf, EvidenceError> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(EvidenceError::Invalid(
            "Invalid relative recording path".into(),
        ));
    }
    let path = directory.join(relative).canonicalize()?;
    if !path.starts_with(directory.canonicalize()?) {
        return Err(EvidenceError::Invalid(
            "Recording is outside the session".into(),
        ));
    }
    Ok(path)
}

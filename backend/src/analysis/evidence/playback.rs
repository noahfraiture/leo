//! Converts a bounded source interval to a webview-playable derivative without changing recordings.
use super::{EvidenceError, EvidenceManifest};
use ffmpeg_sidecar::command::FfmpegCommand;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// A short MP4 derivative and the mapping from its playback time to the session timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlaybackClip {
    pub path: PathBuf,
    pub camera_id: u32,
    pub session_start_ms: u64,
    pub duration_ms: u64,
    pub citation_offset_ms: u64,
}

/// Builds up to one minute of playable context around a verified candidate citation.
/// Decode/transcode and file I/O are blocking; call from a worker, never a component render.
pub fn prepare_playback(
    directory: &Path,
    manifest: &EvidenceManifest,
    frame_id: &str,
    before_ms: u64,
    after_ms: u64,
    cache: &Path,
) -> Result<PlaybackClip, EvidenceError> {
    let frame = manifest
        .frame(frame_id)
        .ok_or_else(|| EvidenceError::Invalid("Unknown candidate citation".into()))?;
    let source = manifest
        .sources
        .get(frame.source_index)
        .ok_or_else(|| EvidenceError::Invalid("Unknown recording segment".into()))?;
    if before_ms.saturating_add(after_ms) == 0 || before_ms.saturating_add(after_ms) > 60_000 {
        return Err(EvidenceError::Invalid(
            "Playback context must be between 1 millisecond and 60 seconds".into(),
        ));
    }
    let segment_length = source
        .end_utc_ms
        .checked_sub(source.start_utc_ms)
        .and_then(|duration| u64::try_from(duration).ok())
        .filter(|duration| *duration > 0)
        .ok_or_else(|| EvidenceError::Invalid("Invalid segment time".into()))?;
    let expected_session_offset = i64::try_from(frame.recording_offset_ms)
        .ok()
        .and_then(|offset| source.start_utc_ms.checked_add(offset))
        .and_then(|utc| utc.checked_sub(manifest.session_start_utc_ms))
        .and_then(|offset| u64::try_from(offset).ok());
    if frame.recording_offset_ms >= segment_length
        || frame.camera_id != source.camera_id
        || expected_session_offset != Some(frame.session_offset_ms)
        || frame.session_offset_ms >= manifest.duration_ms
    {
        return Err(EvidenceError::Invalid(
            "Citation is outside the segment".into(),
        ));
    }
    let session_begin_in_segment = manifest
        .session_start_utc_ms
        .saturating_sub(source.start_utc_ms)
        .max(0) as u64;
    let start = frame
        .recording_offset_ms
        .saturating_sub(before_ms)
        .max(session_begin_in_segment);
    let session_end_utc_ms = manifest.session_start_utc_ms.saturating_add(
        i64::try_from(manifest.duration_ms)
            .map_err(|_| EvidenceError::Invalid("Timeline overflow".into()))?,
    );
    let session_end_in_segment = session_end_utc_ms
        .saturating_sub(source.start_utc_ms)
        .max(0) as u64;
    let end = frame
        .recording_offset_ms
        .saturating_add(after_ms)
        .min(segment_length)
        .min(session_end_in_segment);
    let duration = end
        .checked_sub(start)
        .filter(|duration| *duration > 0)
        .ok_or_else(|| EvidenceError::Invalid("Playback interval is empty".into()))?;
    let session_start_ms = frame
        .session_offset_ms
        .checked_sub(frame.recording_offset_ms - start)
        .ok_or_else(|| EvidenceError::Invalid("Playback begins outside the session".into()))?;
    let path = super::materialize::source_path(directory, &source.relative_path)?;
    if path.metadata()?.len() != source.size_bytes
        || super::materialize::file_hash(&path)? != source.sha256
    {
        return Err(EvidenceError::Invalid("Recording has changed".into()));
    }
    let identity = format!("playback-v1:{}:{start}:{end}", source.sha256);
    let name = format!("{:x}.mp4", Sha256::digest(identity));
    fs::create_dir_all(cache)?;
    let output = cache.join(name);
    if !output.is_file() {
        let temporary = tempfile::Builder::new().suffix(".mp4").tempfile_in(cache)?;
        let mut command = FfmpegCommand::new();
        command.hide_banner().args(["-loglevel","error","-nostdin","-y","-ss"])
            .arg(format!("{start}ms")).arg("-i").arg(&path)
            .args(["-t",&format!("{duration}ms"),"-map","0:v:0","-an","-vf","scale=w='min(iw,1280)':h='min(ih,720)':force_original_aspect_ratio=decrease:force_divisible_by=2,setsar=1","-c:v","libx264","-preset","veryfast","-crf","23","-maxrate","4M","-bufsize","8M","-pix_fmt","yuv420p","-movflags","+faststart","-f","mp4"])
            .arg(temporary.path());
        let result = command.as_inner_mut().output()?;
        if !result.status.success() {
            return Err(EvidenceError::Invalid(
                "Could not prepare recording playback".into(),
            ));
        }
        let size = temporary.as_file().metadata()?.len();
        if size == 0 || size > 64 * 1024 * 1024 {
            return Err(EvidenceError::Invalid(
                "Playback output is empty or too large".into(),
            ));
        }
        temporary.as_file().sync_all()?;
        match temporary.persist_noclobber(&output) {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.error.into()),
        }
    }
    Ok(PlaybackClip {
        path: output,
        camera_id: frame.camera_id,
        session_start_ms,
        duration_ms: duration,
        citation_offset_ms: frame.recording_offset_ms - start,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{CandidateFrame, RecordingSource};
    #[test]
    fn inconsistent_citation_timelines_fail_before_opening_media() {
        let source = RecordingSource {
            relative_path: "missing.mkv".into(),
            camera_id: 1,
            start_utc_ms: 1000,
            end_utc_ms: 6000,
            size_bytes: 1,
            sha256: "hash".into(),
        };
        let mut manifest = EvidenceManifest {
            session_id: uuid::Uuid::new_v4(),
            session_start_utc_ms: 2000,
            duration_ms: 4000,
            sources: vec![source],
            frames: vec![],
            frame_sets: vec![],
            warnings: vec![],
            eligible_periods: vec![],
        };
        for (camera, recording, session) in [
            (1, 0, 0),
            (2, 2000, 1000),
            (1, 2000, u64::MAX),
            (1, 9000, 8000),
        ] {
            manifest.frames = vec![CandidateFrame {
                id: "frame".into(),
                camera_id: camera,
                source_index: 0,
                session_offset_ms: session,
                recording_offset_ms: recording,
            }];
            let error = prepare_playback(
                Path::new("/missing"),
                &manifest,
                "frame",
                3000,
                3000,
                Path::new("/missing/cache"),
            )
            .unwrap_err();
            assert!(matches!(error, EvidenceError::Invalid(_)), "{error}");
        }
    }
}

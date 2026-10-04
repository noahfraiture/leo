use std::fs;

use super::super::{SentImage, materialize_frame};
use super::{JobRecord, RunError, RunResult, RunSnapshot, RunStore, store};
use crate::{
    exercise::preview_reference_bytes,
    model::{ImageFormat, ImageRole, StructuredImage},
    profiles::ImageSizePolicy,
};

/// Images and their identities for a single bounded request; never the complete recording.
pub struct PreparedImages {
    pub images: Vec<StructuredImage>,
    pub sent_images: Vec<SentImage>,
    pub reference_hashes: Vec<(uuid::Uuid, String)>,
}

/// Prepares frozen references and cached candidate images on a blocking worker.
pub fn prepare_images(
    store: &RunStore,
    snapshot: &RunSnapshot,
    job: &JobRecord,
) -> RunResult<PreparedImages> {
    let (mut images, reference_hashes) = references(store, snapshot, job)?;
    let (candidates, sent_images) = candidates(store, snapshot, job)?;
    images.extend(candidates);
    Ok(PreparedImages {
        images,
        sent_images,
        reference_hashes,
    })
}

fn references(
    store: &RunStore,
    snapshot: &RunSnapshot,
    job: &JobRecord,
) -> RunResult<(Vec<StructuredImage>, Vec<(uuid::Uuid, String)>)> {
    let directory = store.directory(snapshot.inputs.id)?;
    let mut images = Vec::new();
    let mut reference_hashes = Vec::new();
    for reference in snapshot
        .inputs
        .exercise
        .references
        .iter()
        .filter(|reference| {
            snapshot.inputs.evidence_policy.reference_images
                && job
                    .criterion_id
                    .is_none_or(|id| reference.criterion_ids.contains(&id))
        })
    {
        let asset = &reference.image;
        let original = fs::read(directory.join("references").join(format!(
            "{}.{}",
            asset.id,
            asset.media_type.extension()
        )))?;
        if original.len() as u64 != asset.byte_length || store::hash(&original) != asset.sha256 {
            return Err(RunError::Invalid(
                "A copied reference original changed".into(),
            ));
        }
        let maximum = match snapshot.inputs.evidence_policy.image_size {
            ImageSizePolicy::Original => asset.width.max(asset.height),
            ImageSizePolicy::MaximumLongEdge(limit) => limit,
        };
        let preview = preview_reference_bytes(&original, maximum)?;
        reference_hashes.push((asset.id, store::hash(&preview.png)));
        images.push(StructuredImage {
            id: format!("R-{}", asset.id),
            role: ImageRole::Reference,
            description: format!(
                "Object: {}\nState: {}\nPurpose: {}",
                reference.object_label, reference.state_label, reference.purpose
            ),
            bytes: preview.png,
            format: ImageFormat::Png,
            detail: snapshot.inputs.evidence_policy.image_detail,
        });
    }
    Ok((images, reference_hashes))
}

fn candidates(
    store: &RunStore,
    snapshot: &RunSnapshot,
    job: &JobRecord,
) -> RunResult<(Vec<StructuredImage>, Vec<SentImage>)> {
    let directory = store.directory(snapshot.inputs.id)?;
    let mut images = Vec::new();
    let mut sent_images = Vec::new();
    for id in &job.frame_ids {
        let frame = snapshot
            .checkpoint
            .evidence
            .frame(id)
            .ok_or_else(|| RunError::Invalid("Unknown candidate image".into()))?;
        let cache_key = store::hash(&serde_json::to_vec(&(
            frame,
            snapshot.inputs.evidence_policy.image_size,
        ))?);
        let image_path = directory.join("cache").join(format!("{cache_key}.jpg"));
        let identity_path = directory.join("cache").join(format!("{cache_key}.json"));
        let (bytes, sent) = if identity_path.try_exists()? && image_path.try_exists()? {
            let sent: SentImage = store::read_json(&identity_path)?;
            let bytes = fs::read(&image_path)?;
            if sent.candidate_id != *id
                || sent.size_bytes != bytes.len() as u64
                || sent.sha256 != store::hash(&bytes)
            {
                return Err(RunError::Invalid("Candidate image cache changed".into()));
            }
            (bytes, sent)
        } else {
            let (bytes, sent) = materialize_frame(
                &store.session_directory,
                &snapshot.checkpoint.evidence,
                frame,
                snapshot.inputs.evidence_policy.image_size,
            )?;
            if identity_path.try_exists()? && store::read_json::<SentImage>(&identity_path)? != sent
            {
                return Err(RunError::Invalid(
                    "Recreated candidate image differs from its saved identity".into(),
                ));
            }
            store::write_bytes(&image_path, &bytes, true)?;
            store::write_json(&identity_path, &sent, true)?;
            (bytes, sent)
        };
        images.push(StructuredImage {
            id: id.clone(),
            role: ImageRole::Candidate,
            description: format!(
                "Camera {}, session offset {} ms",
                frame.camera_id, frame.session_offset_ms
            ),
            bytes,
            format: ImageFormat::Jpeg,
            detail: snapshot.inputs.evidence_policy.image_detail,
        });
        sent_images.push(sent);
    }
    Ok((images, sent_images))
}

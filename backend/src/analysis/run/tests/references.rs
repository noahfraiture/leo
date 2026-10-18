use super::*;
use crate::{
    analysis::run::{jobs, store},
    exercise::{ImageAsset, ImageMediaType, ReferenceAsset},
    model::ImageRole,
};
use std::{fs, io::Cursor};
use uuid::Uuid;

/// Real encoded pixels exercise reference delivery, independent of the scripted model.
fn attach_references(fixture: &Fixture, snapshot: &mut RunSnapshot) {
    let directory = fixture
        .store
        .directory(fixture.run_id)
        .unwrap()
        .join("references");
    fs::create_dir_all(&directory).unwrap();
    for (index, criterion) in snapshot
        .inputs
        .exercise
        .checklist
        .criteria
        .iter()
        .enumerate()
    {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            3,
            2,
            image::Rgb([40 + index as u8, 80, 120]),
        ))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
        let bytes = bytes.into_inner();
        let id = Uuid::new_v4();
        fs::write(directory.join(format!("{id}.png")), &bytes).unwrap();
        snapshot.inputs.exercise.references.push(ReferenceAsset {
            image: ImageAsset {
                id,
                sha256: store::hash(&bytes),
                byte_length: bytes.len() as u64,
                media_type: ImageMediaType::Png,
                width: 3,
                height: 2,
                orientation: 1,
            },
            object_label: format!("Object {index}"),
            state_label: "Resting".into(),
            purpose: "Identify the object".into(),
            criterion_ids: vec![criterion.id],
        });
    }
}

#[test]
fn reference_pixels_are_opt_in_and_the_closer_look_uses_only_linked_images() {
    let fixture = Fixture::new("http://127.0.0.1:1/v1");
    let mut snapshot = fixture.snapshot();
    attach_references(&fixture, &mut snapshot);
    let scan = &snapshot.checkpoint.jobs[0];
    let disabled = jobs::prepare(&fixture.store, &snapshot, scan).unwrap();
    assert!(disabled.reference_hashes.is_empty());
    assert!(
        disabled
            .request
            .images
            .iter()
            .all(|image| image.role == ImageRole::Candidate)
    );

    snapshot.inputs.evidence_policy.reference_images = true;
    let enabled = jobs::prepare(&fixture.store, &snapshot, scan).unwrap();
    assert_eq!(enabled.reference_hashes.len(), 2);
    let criterion = snapshot.inputs.exercise.checklist.criteria[0].id;
    let recheck = jobs::record(
        &snapshot.inputs,
        JobKind::Recheck,
        Some(criterion),
        scan.frame_ids.clone(),
    )
    .unwrap();
    let focused = jobs::prepare(&fixture.store, &snapshot, &recheck).unwrap();
    assert_eq!(focused.reference_hashes, enabled.reference_hashes[..1]);
    assert_eq!(
        focused.sent_images, enabled.sent_images,
        "references are separate from candidate provenance"
    );
    let reference = &focused.request.images[0];
    assert_eq!(reference.role, ImageRole::Reference);
    assert_eq!(
        image::load_from_memory(&reference.bytes).unwrap().width(),
        3
    );
}

#[test]
fn modified_reference_originals_are_rejected_before_a_model_request() {
    let fixture = Fixture::new("http://127.0.0.1:1/v1");
    let mut snapshot = fixture.snapshot();
    attach_references(&fixture, &mut snapshot);
    snapshot.inputs.evidence_policy.reference_images = true;
    let id = snapshot.inputs.exercise.references[0].image.id;
    let path = fixture
        .store
        .directory(fixture.run_id)
        .unwrap()
        .join("references")
        .join(format!("{id}.png"));
    fs::write(path, b"changed pixels").unwrap();
    assert!(jobs::prepare(&fixture.store, &snapshot, &snapshot.checkpoint.jobs[0]).is_err());
}

#[cfg(unix)]
#[test]
fn frozen_reference_symlinks_are_rejected_by_model_preparation() {
    let fixture = Fixture::new("http://127.0.0.1:1/v1");
    let mut snapshot = fixture.snapshot();
    attach_references(&fixture, &mut snapshot);
    snapshot.inputs.evidence_policy.reference_images = true;
    let asset = &snapshot.inputs.exercise.references[0].image;
    let directory = fixture.store.directory(fixture.run_id).unwrap();
    let original = directory
        .join("references")
        .join(format!("{}.png", asset.id));
    let moved = directory.join("moved-original.png");
    fs::rename(&original, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &original).unwrap();

    assert!(fixture.store.read_reference(&snapshot, asset.id).is_err());
    assert!(jobs::prepare(&fixture.store, &snapshot, &snapshot.checkpoint.jobs[0]).is_err());
}

#[test]
fn frozen_reference_reads_enforce_the_import_size_limit() {
    let fixture = Fixture::new("http://127.0.0.1:1/v1");
    let mut snapshot = fixture.snapshot();
    attach_references(&fixture, &mut snapshot);
    let asset = &mut snapshot.inputs.exercise.references[0].image;
    let path = fixture
        .store
        .directory(fixture.run_id)
        .unwrap()
        .join("references")
        .join(format!("{}.png", asset.id));
    let bytes = vec![0; crate::exercise::MAX_IMAGE_BYTES as usize + 1];
    fs::write(path, &bytes).unwrap();
    asset.byte_length = bytes.len() as u64;
    asset.sha256 = store::hash(&bytes);
    let id = asset.id;
    drop(bytes);

    assert!(fixture.store.read_reference(&snapshot, id).is_err());
}

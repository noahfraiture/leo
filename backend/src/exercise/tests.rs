use std::{fs, io::Cursor, path::Path};

use ::image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use tempfile::TempDir;
use uuid::Uuid;

use super::*;

fn store() -> (TempDir, ExerciseStore) {
    let temporary = tempfile::tempdir().unwrap();
    let store = ExerciseStore::new(temporary.path().join("exercises"));
    (temporary, store)
}

fn criterion(text: &str) -> Criterion {
    Criterion {
        id: Uuid::new_v4(),
        text: text.into(),
    }
}

fn ready_draft(store: &ExerciseStore) -> ExerciseDraft {
    let mut draft = store.create_draft("Storage exercise").unwrap();
    draft.checklist.criteria = vec![criterion("Place the unit in the case.")];
    store.save_draft(&draft).unwrap()
}

fn encoded_image(format: ImageFormat) -> Vec<u8> {
    let pixels = RgbImage::from_fn(2, 3, |x, y| Rgb([x as u8 * 100, y as u8 * 70, 90]));
    let mut output = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(pixels)
        .write_to(&mut output, format)
        .unwrap();
    output.into_inner()
}

fn attach_reference(store: &ExerciseStore, draft: &mut ExerciseDraft, source: &Path) -> ImageAsset {
    fs::write(source, encoded_image(ImageFormat::Png)).unwrap();
    let image = store.import_reference(draft.id, source).unwrap();
    draft.references.push(ReferenceAsset {
        image: image.clone(),
        object_label: "Case".into(),
        state_label: "Open".into(),
        purpose: "Identify the storage location".into(),
        criterion_ids: vec![draft.checklist.criteria[0].id],
    });
    image
}

#[test]
fn publishing_requires_the_saved_operator_text() {
    let (_temporary, store) = store();
    let mut draft = ready_draft(&store);
    draft.checklist.criteria[0].text = "Close the case.".into();
    assert!(matches!(
        store.publish_revision(&draft),
        Err(Error::StaleDraft)
    ));
    assert!(store.revisions(draft.id).unwrap().is_empty());
}

#[test]
fn later_edits_do_not_change_a_published_checklist() {
    let (_temporary, store) = store();
    let mut draft = ready_draft(&store);
    let revision = store.publish_revision(&draft).unwrap();
    draft.checklist.criteria[0].text = "Close the case.".into();
    let draft = store.save_draft(&draft).unwrap();
    let newer = store.publish_revision(&draft).unwrap();
    assert_ne!(revision.id, newer.id);
    assert_eq!(
        store.load_revision(draft.id, revision.id).unwrap(),
        revision
    );
}

#[test]
fn stale_writers_cannot_replace_newer_edits() {
    let (_temporary, store) = store();
    let original = ready_draft(&store);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let writers = ["Left", "Right"].map(|name| {
        let store = store.clone();
        let barrier = barrier.clone();
        let mut edited = original.clone();
        edited.name = name.into();
        std::thread::spawn(move || {
            barrier.wait();
            store.save_draft(&edited)
        })
    });
    let results = writers.map(|writer| writer.join().unwrap());
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(Error::StaleDraft)))
            .count(),
        1
    );
    let winner = results.into_iter().find_map(Result::ok).unwrap();
    assert_eq!(store.load_draft(original.id).unwrap(), winner);
}

#[test]
fn reference_links_follow_criterion_identity_when_reordered() {
    let (temporary, store) = store();
    let mut draft = ready_draft(&store);
    attach_reference(&store, &mut draft, &temporary.path().join("case.png"));
    let linked_id = draft.references[0].criterion_ids[0];
    draft.checklist.criteria.push(criterion("Close the case."));
    draft.checklist.criteria.reverse();
    let saved = store.save_draft(&draft).unwrap();
    assert_eq!(saved.references[0].criterion_ids, vec![linked_id]);
    assert_eq!(saved.checklist.criteria[1].id, linked_id);
}

#[test]
fn removing_linked_criteria_requires_explicit_link_edits() {
    let (temporary, store) = store();
    let mut draft = ready_draft(&store);
    attach_reference(&store, &mut draft, &temporary.path().join("case.png"));
    let mut draft = store.save_draft(&draft).unwrap();
    draft.checklist.criteria.clear();
    assert!(matches!(store.save_draft(&draft), Err(Error::Invalid(_))));
    draft.references[0].criterion_ids.clear();
    assert!(store.save_draft(&draft).is_ok());
}

#[test]
fn import_preserves_original_bytes_and_detects_format_from_content() {
    let (temporary, store) = store();
    let draft = ready_draft(&store);
    let source = temporary.path().join("misleading.jpg");
    let original = encoded_image(ImageFormat::Png);
    fs::write(&source, &original).unwrap();
    let image = store.import_reference(draft.id, &source).unwrap();
    assert_eq!(image.media_type, ImageMediaType::Png);
    assert_eq!(store.read_reference(draft.id, &image).unwrap(), original);
    assert!(store.load_draft(draft.id).unwrap().references.is_empty());
}

#[test]
fn corrupt_image_is_rejected_without_publishing_an_asset() {
    let (temporary, store) = store();
    let draft = ready_draft(&store);
    let source = temporary.path().join("broken.png");
    let mut bytes = encoded_image(ImageFormat::Png);
    bytes.truncate(bytes.len() / 2);
    fs::write(&source, bytes).unwrap();
    assert!(store.import_reference(draft.id, &source).is_err());
    assert!(
        !store
            .root
            .join(draft.id.to_string())
            .join("assets")
            .exists()
    );
}

#[test]
fn oversized_upload_is_rejected_before_decode() {
    let (temporary, store) = store();
    let draft = ready_draft(&store);
    let source = temporary.path().join("oversized.png");
    fs::File::create(&source)
        .unwrap()
        .set_len(MAX_IMAGE_BYTES + 1)
        .unwrap();
    assert!(matches!(
        store.import_reference(draft.id, &source),
        Err(Error::ImageTooLarge)
    ));
}

#[test]
fn changed_asset_bytes_block_publication() {
    let (temporary, store) = store();
    let mut draft = ready_draft(&store);
    let asset = attach_reference(&store, &mut draft, &temporary.path().join("case.png"));
    let draft = store.save_draft(&draft).unwrap();
    let path = store
        .root
        .join(draft.id.to_string())
        .join("assets")
        .join(format!("{}.png", asset.id));
    fs::write(path, b"altered after import").unwrap();
    assert!(matches!(
        store.publish_revision(&draft),
        Err(Error::ChangedAsset(_))
    ));
}

#[test]
fn removed_draft_references_remain_available_to_saved_revisions() {
    let (temporary, store) = store();
    let mut draft = ready_draft(&store);
    let asset = attach_reference(&store, &mut draft, &temporary.path().join("case.png"));
    let mut draft = store.save_draft(&draft).unwrap();
    let revision = store.publish_revision(&draft).unwrap();
    draft.references.clear();
    store.save_draft(&draft).unwrap();
    assert_eq!(
        store
            .load_revision(draft.id, revision.id)
            .unwrap()
            .references[0]
            .image,
        asset
    );
    assert!(store.read_reference(draft.id, &asset).is_ok());
}

#[test]
fn preview_applies_exif_orientation_without_modifying_original() {
    let (temporary, store) = store();
    let draft = ready_draft(&store);
    let jpeg = encoded_image(ImageFormat::Jpeg);
    // One little-endian EXIF entry: Orientation=6 (90 degrees clockwise).
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut original = vec![0xff, 0xd8, 0xff, 0xe1];
    original.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    original.extend_from_slice(exif);
    original.extend_from_slice(&jpeg[2..]);
    let source = temporary.path().join("rotated.jpg");
    fs::write(&source, &original).unwrap();
    let asset = store.import_reference(draft.id, &source).unwrap();
    let preview = store.preview_reference(draft.id, &asset, 100).unwrap();
    assert_eq!((asset.width, asset.height, asset.orientation), (2, 3, 6));
    assert_eq!((preview.width, preview.height), (3, 2));
    assert_eq!(store.read_reference(draft.id, &asset).unwrap(), original);
}

fn proposal(draft: &ExerciseDraft) -> ChecklistProposal {
    ChecklistProposal::for_draft(
        draft,
        vec![ProposedCriterion {
            text: "Place the unit inside the storage case.".into(),
            source_ranges: vec![SourceRange {
                start: 0,
                end: draft.checklist.source_text().len(),
            }],
        }],
        Vec::new(),
    )
    .unwrap()
}

#[test]
fn proposal_becomes_stale_after_unsaved_edits() {
    let (_temporary, store) = store();
    let mut draft = ready_draft(&store);
    let proposal = proposal(&draft);
    draft.checklist.criteria[0].text.push_str(" Then close it.");
    assert!(matches!(
        proposal.validate_against(&draft),
        Err(Error::StaleProposal)
    ));
}

#[test]
fn proposal_source_ranges_must_respect_utf8_boundaries() {
    let (_temporary, store) = store();
    let mut draft = ready_draft(&store);
    draft.checklist.criteria[0].text = "Étape: fermer.".into();
    let mut proposal = proposal(&draft);
    proposal.criteria[0].source_ranges = vec![SourceRange { start: 1, end: 3 }];
    assert!(matches!(
        proposal.validate_against(&draft),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn source_discrepancy_comments_must_cite_the_source() {
    let (_temporary, store) = store();
    let draft = ready_draft(&store);
    let mut proposal = proposal(&draft);
    proposal.comments.push(ProposalComment {
        kind: ProposalCommentKind::SourceDiscrepancy,
        text: "The quantity changed.".into(),
        source_ranges: Vec::new(),
        proposed_item: Some(0),
    });
    assert!(matches!(
        proposal.validate_against(&draft),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn proposal_does_not_publish_or_change_the_operator_checklist() {
    let (_temporary, store) = store();
    let draft = ready_draft(&store);
    let proposal = proposal(&draft);
    assert_ne!(proposal.criteria[0].text, draft.checklist.criteria[0].text);
    assert_eq!(store.load_draft(draft.id).unwrap(), draft);
    assert!(store.revisions(draft.id).unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn symlink_uploads_are_rejected() {
    let (temporary, store) = store();
    let draft = ready_draft(&store);
    let original = temporary.path().join("original.png");
    fs::write(&original, encoded_image(ImageFormat::Png)).unwrap();
    let link = temporary.path().join("linked.png");
    std::os::unix::fs::symlink(original, &link).unwrap();
    assert!(matches!(
        store.import_reference(draft.id, &link),
        Err(Error::InvalidStorageEntry)
    ));
}

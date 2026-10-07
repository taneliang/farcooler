//! A compose's staged images (ov-393): uploaded in chunks and kept, typed
//! nowhere; read once by the compose that names them and deleted; capped;
//! and swept, the staged within the hour and the composed within a day.

use std::time::SystemTime;

use farcooler_protocol::v1::ImageBlock;

use super::*;

fn png(len: usize) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    b.resize(len, 0x5a);
    b
}

/// Upload `bytes` under `id` as a client does, chunk by chunk.
async fn stage(root: &Path, id: &[u8], bytes: &[u8]) -> PathBuf {
    let mut offset = 0u64;
    for chunk in bytes.chunks(farcooler_protocol::PASTE_CHUNK_BYTES) {
        match put_chunk(root, id, bytes.len() as u64, offset, chunk).await.expect("a chunk") {
            Stored::Partial { stored } => offset = stored,
            Stored::Complete { path, .. } => return path,
        }
    }
    panic!("never complete")
}

fn staged_block(id: &[u8]) -> AgentPromptBlock {
    AgentPromptBlock { content: Some(Content::StagedImage(bytes::Bytes::copy_from_slice(id))) }
}

/// Files directly in `dir`.
fn files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file()).collect()).unwrap_or_default()
}

/// Ten megabytes in chunks: kept under its id, whole, and nothing in the
/// paste directory a person or a sweep of pastes would see as a paste.
#[tokio::test]
async fn a_ten_megabyte_image_is_staged_whole_under_its_id() {
    let tmp = tempfile::tempdir().unwrap();
    let image = png(10 * 1024 * 1024);
    let path = stage(tmp.path(), &[7, 1], &image).await;
    assert_eq!(path, staged_dir_in(tmp.path()).unwrap().join("0701"));
    assert_eq!(std::fs::read(&path).unwrap(), image);
    assert!(files(&crate::paths::pastes_dir_in(tmp.path()).unwrap()).is_empty());
}

/// Named by a compose, read in the blocks' order beside a carried one, and
/// deleted; named again, it's gone: `image`.
#[tokio::test]
async fn a_compose_reads_its_staged_images_once() {
    let tmp = tempfile::tempdir().unwrap();
    let big = png(2 * 1024 * 1024);
    let path = stage(tmp.path(), &[1], &big).await;
    let carried = AgentPromptBlock {
        content: Some(Content::Image(ImageBlock { mime_type: "image/png".into(), data: png(10).into() })),
    };
    let blocks = [staged_block(&[1]), carried];
    let got = images(tmp.path(), &blocks).expect("read");
    assert_eq!(got.iter().map(|(_, b)| b.len()).collect::<Vec<_>>(), [big.len(), 10]);
    assert!(!path.exists(), "used once, and deleted");
    assert_eq!(images(tmp.path(), &blocks).unwrap_err().what(), "image");
}

/// Past 50 MB together, or 900 KB carried: `images_too_large`, and
/// the staged ones are deleted all the same.
#[tokio::test]
async fn images_past_the_caps_are_refused_and_still_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    // Each no more than a paste takes, 16 MB: three of them and the rest.
    let (cap, file) = (farcooler_protocol::MAX_COMPOSE_UPLOAD_BYTES, farcooler_protocol::MAX_PASTE_FILE_BYTES as usize);
    let named: Vec<_> = (1..=4u8).map(|i| staged_block(&[i])).collect();
    let mut sent = Vec::new();
    for i in 1..=3u8 {
        sent.push(stage(tmp.path(), &[i], &png(file)).await);
    }
    sent.push(stage(tmp.path(), &[4], &png(cap - 3 * file)).await);
    assert_eq!(images(tmp.path(), &named).expect("at the cap").len(), 4);
    for i in 1..=3u8 {
        sent.push(stage(tmp.path(), &[i], &png(file)).await);
    }
    sent.push(stage(tmp.path(), &[4], &png(cap - 3 * file + 1)).await);
    assert_eq!(images(tmp.path(), &named).unwrap_err().what(), "images_too_large");
    assert!(!sent.iter().any(|p| p.exists()), "every staged image named is gone");

    let carried = |n: usize| AgentPromptBlock {
        content: Some(Content::Image(ImageBlock { mime_type: "image/png".into(), data: png(n).into() })),
    };
    let inline = farcooler_protocol::MAX_COMPOSE_IMAGE_BYTES;
    assert!(images(tmp.path(), &[carried(inline)]).is_ok());
    assert_eq!(images(tmp.path(), &[carried(inline + 1)]).unwrap_err().what(), "images_too_large");
}

/// An id that isn't one names nothing: refused before anything is read.
#[test]
fn an_empty_or_long_id_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(images(tmp.path(), &[staged_block(&[])]).unwrap_err().what(), "transfer id");
    assert_eq!(images(tmp.path(), &[staged_block(&[0; 65])]).unwrap_err().what(), "transfer id");
}

/// The sweep: a staged image after an hour, a composed copy after 23 hours,
/// and an ordinary paste only after its week.
#[tokio::test]
async fn the_sweep_takes_staged_within_the_hour_and_composed_within_a_day() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let staged = stage(root, &[3], &png(64)).await;
    let pastes = crate::paths::pastes_dir_in(root).unwrap();
    let composed = pastes.join(format!("{COMPOSED_PREFIX}0190.png"));
    let pasted = pastes.join("2026-10-07-101010Z-shot.png");
    std::fs::write(&composed, png(64)).unwrap();
    std::fs::write(&pasted, png(64)).unwrap();
    let now = SystemTime::now();
    let hours = |h: u64| now + Duration::from_secs(h * 60 * 60 + 60);

    super::super::sweep_at(root, now).await;
    assert!(staged.exists() && composed.exists() && pasted.exists(), "nothing is old yet");
    super::super::sweep_at(root, hours(1)).await;
    assert!(!staged.exists(), "staged, unused for an hour");
    assert!(composed.exists() && pasted.exists());
    super::super::sweep_at(root, hours(23)).await;
    assert!(!composed.exists(), "a composed copy, within a day");
    assert!(pasted.exists(), "a paste keeps its week");
}

//! The game mounts; its index holds the files a second, independent reader finds (`gz-corpus`
//! reads the archives' directories and the loose tree itself); every `READ_STRIDE`th archived
//! file by key is read and unpacks to its stated size, every archived file lies inside its
//! archive, and 100 files spread over the index read the same bytes through both readers.

use std::collections::BTreeSet;
use std::time::Instant;

use gz_corpus::{check_all, finish, stride};
use gz_vfs::{GameFiles, MountOptions, Source};

const READ_STRIDE: usize = 8;

#[test]
fn the_game_mounts_and_reads_as_a_second_reader_reads_it() {
    let Some(game) = gz_corpus::game() else { return };
    let started = Instant::now();
    let other = game.unwrap();
    let files = GameFiles::mount(&other.root, &MountOptions { cache_dir: None }).unwrap();
    let mut failed = Vec::new();

    // the same files, by key
    let mine: BTreeSet<&str> = files.iter().map(|f| f.path).collect();
    let theirs: BTreeSet<&str> = other.files().iter().map(|e| e.key.as_str()).collect();
    for k in theirs.difference(&mine).take(5) {
        failed.push(format!("{k}: found by the second reader, not in the index"));
    }
    for k in mine.difference(&theirs).take(5) {
        failed.push(format!("{k}: in the index, not found by the second reader"));
    }

    // every archived entry lies inside its archive
    let lens: Vec<u64> =
        files.archives().iter().map(|a| std::fs::metadata(&a.path).map_or(0, |m| m.len())).collect();
    let mut archived = 0usize;
    for f in files.iter() {
        if let Source::Archive { archive, offset, packed } = f.source {
            archived += 1;
            let len = lens.get(archive as usize).copied().unwrap_or(0);
            if u64::from(offset) + u64::from(packed) > len {
                failed.push(format!("{}: runs past the end of its archive", f.path));
            }
        }
    }
    assert!(archived > 10_000, "{archived}");

    // a stride of the archived files read and unpacked
    let refs: Vec<_> = files.iter().filter(|f| matches!(f.source, Source::Archive { .. })).collect();
    let sample = stride(refs, READ_STRIDE);
    failed.extend(check_all(
        &sample,
        |f| f.path.to_owned(),
        |f| {
            let b = files.read_ref(*f).map_err(|e| e.to_string())?;
            if b.len() as u64 != f.size {
                Err(format!("{} bytes read, {} stated", b.len(), f.size))
            } else {
                Ok(())
            }
        },
    ));

    // 100 files spread over the whole index, through both readers
    let all: Vec<_> = other.files().iter().collect();
    let spread = stride(all, (other.files().len() / 100).max(1));
    failed.extend(check_all(
        &spread,
        |e| e.key.clone(),
        |e| {
            let a = files.read(&e.key).map_err(|e| e.to_string())?;
            let b = other.read(e).map_err(|e| e.to_string())?;
            if a == b { Ok(()) } else { Err("the two readers read different bytes".into()) }
        },
    ));
    println!(
        "vfs: {} files, {archived} archived in {} archives; every {READ_STRIDE}th archived read ({}), {} compared",
        files.len(),
        files.archives().len(),
        sample.len(),
        spread.len()
    );
    finish("vfs", files.len(), started, &failed);
}

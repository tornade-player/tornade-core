//! Whole audio files kept in memory for tracks that live on a network mount.
//!
//! The player reads a file entirely before decoding it. On a NAS that read is the wait between two songs, and it is
//! repeated at every seek and every resume. This cache keeps the files of the previous, the current and the next track
//! (at most [`MAX_ENTRIES`] files and [`MAX_TOTAL`] bytes), and the next one is read in the background while the current
//! one plays. Files on a local disk are never kept: reading them is instant, so they would only cost memory.
//!
//! A mount is "network" when `/proc/self/mountinfo` gives it a network or FUSE file system (NFS, SMB, SSHFS, GVFS, rclone...).

use log::{info, warn};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Instant, SystemTime};

/// Files kept at most: previous, current, next.
const MAX_ENTRIES: usize = 3;
/// Bytes kept at most; a file bigger than this is not kept at all.
const MAX_TOTAL: usize = 200 * 1024 * 1024;

struct Entry {
    path: PathBuf,
    /// Size and modification time when read: a changed file (tags rewritten) is read again.
    stamp: (u64, Option<SystemTime>),
    data: Arc<[u8]>,
}

#[derive(Default)]
struct Inner {
    /// Oldest first.
    entries: Vec<Entry>,
    /// Files being read right now: whoever wants one of them waits for it instead of reading it a second time.
    loading: HashSet<PathBuf>,
}

pub struct FileCache {
    inner: Mutex<Inner>,
    done: Condvar,
    /// Whether a path is worth keeping (a network mount); replaced in tests.
    keeps: fn(&Path) -> bool,
}

impl Default for FileCache {
    fn default() -> Self {
        Self::with_policy(is_network)
    }
}

impl FileCache {
    pub fn with_policy(keeps: fn(&Path) -> bool) -> Self {
        FileCache {
            inner: Mutex::new(Inner::default()),
            done: Condvar::new(),
            keeps,
        }
    }

    fn stamp(path: &Path) -> io::Result<(u64, Option<SystemTime>)> {
        let m = std::fs::metadata(path)?;
        Ok((m.len(), m.modified().ok()))
    }

    /// The whole file: from memory when it is kept, else read (and kept when it is on a network mount).
    pub fn read(&self, path: &Path) -> io::Result<Arc<[u8]>> {
        let stamp = Self::stamp(path)?;
        let mut inner = self.inner.lock().unwrap();
        loop {
            if let Some(i) = inner.entries.iter().position(|e| e.path == path) {
                if inner.entries[i].stamp == stamp {
                    return Ok(Arc::clone(&inner.entries[i].data));
                }
                inner.entries.remove(i);
            }
            if inner.loading.contains(path) {
                inner = self.done.wait(inner).unwrap();
                continue;
            }
            break;
        }
        inner.loading.insert(path.to_path_buf());
        drop(inner);

        let started = Instant::now();
        let read = std::fs::read(path).map(Arc::<[u8]>::from);

        let mut inner = self.inner.lock().unwrap();
        inner.loading.remove(path);
        if let Ok(data) = &read
            && data.len() <= MAX_TOTAL
            && (self.keeps)(path)
        {
            info!(
                "File cache: {} kept ({} KiB read in {:?})",
                path.display(),
                data.len() / 1024,
                started.elapsed()
            );
            inner.entries.push(Entry {
                path: path.to_path_buf(),
                stamp,
                data: Arc::clone(data),
            });
            Self::trim(&mut inner);
        }
        self.done.notify_all();
        read
    }

    /// Reads a file in advance (the next track). Only for a network mount; local files are not worth it.
    pub fn prefetch(&self, path: &Path) {
        if (self.keeps)(path)
            && let Err(e) = self.read(path)
        {
            warn!("File cache: prefetch of {} failed: {e}", path.display());
        }
    }

    /// Forgets every file that is not in `wanted` (the tracks around the current one).
    pub fn keep_only(&self, wanted: &[PathBuf]) {
        self.inner
            .lock()
            .unwrap()
            .entries
            .retain(|e| wanted.contains(&e.path));
    }

    fn trim(inner: &mut Inner) {
        while inner.entries.len() > MAX_ENTRIES
            || inner.entries.iter().map(|e| e.data.len()).sum::<usize>() > MAX_TOTAL
        {
            inner.entries.remove(0);
        }
    }

    #[cfg(test)]
    fn kept(&self) -> Vec<PathBuf> {
        self.inner
            .lock()
            .unwrap()
            .entries
            .iter()
            .map(|e| e.path.clone())
            .collect()
    }
}

/// Whether `path` is on a network (or FUSE) file system.
pub fn is_network(path: &Path) -> bool {
    std::fs::read_to_string("/proc/self/mountinfo")
        .ok()
        .and_then(|text| fstype_of(&text, path))
        .is_some_and(|t| is_network_fstype(&t))
}

fn is_network_fstype(t: &str) -> bool {
    match t {
        "nfs" | "nfs4" | "cifs" | "smb3" | "smbfs" | "ncpfs" | "9p" | "ceph" | "glusterfs" | "lustre" | "afs"
        | "davfs" | "fuse" => true,
        // fuse.sshfs, fuse.rclone, fuse.gvfsd-fuse...; the portals only expose local files
        _ => t.starts_with("fuse.") && !t.contains("portal"),
    }
}

/// File system type of the mount that holds `path` in the text of /proc/self/mountinfo (the longest mount point wins).
fn fstype_of(mountinfo: &str, path: &Path) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for line in mountinfo.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let (Some(point), Some(fstype)) = (left.split(' ').nth(4), right.split(' ').next()) else {
            continue;
        };
        let point = point
            .replace("\\040", " ")
            .replace("\\011", "\t")
            .replace("\\012", "\n")
            .replace("\\134", "\\");
        let depth = Path::new(&point).components().count();
        if path.starts_with(&point) && best.as_ref().is_none_or(|(d, _)| depth >= *d) {
            best = Some((depth, fstype.to_string()));
        }
    }
    best.map(|(_, t)| t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const MOUNTINFO: &str = "\
24 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
97 24 0:51 / /run/user/1000/gvfs rw,nosuid,relatime shared:60 - fuse.gvfsd-fuse gvfsd-fuse rw,user_id=1000
98 24 0:52 / /run/user/1000/doc rw,nosuid shared:61 - fuse.portal portal rw
99 24 0:53 / /mnt/nas\\040share rw shared:62 - nfs4 nas:/music rw
100 24 0:54 / /boot rw shared:63 - fuseblk /dev/sda1 rw";

    fn always(_: &Path) -> bool {
        true
    }

    fn never(_: &Path) -> bool {
        false
    }

    fn temp(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tornade-file-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::File::create(&path).unwrap().write_all(bytes).unwrap();
        path
    }

    #[test]
    fn finds_the_mount_of_a_path() {
        let t = |p: &str| fstype_of(MOUNTINFO, Path::new(p));
        assert_eq!(t("/home/me/a.flac").as_deref(), Some("ext4"));
        assert_eq!(t("/run/user/1000/gvfs/smb-share:server=nas/a.flac").as_deref(), Some("fuse.gvfsd-fuse"));
        assert_eq!(t("/mnt/nas share/a.flac").as_deref(), Some("nfs4"));
        // A mount point is matched by whole components: /boot2 is not on /boot
        assert_eq!(t("/boot2/a.flac").as_deref(), Some("ext4"));
    }

    #[test]
    fn network_file_systems_are_the_slow_ones() {
        for t in ["nfs", "nfs4", "cifs", "smb3", "fuse", "fuse.sshfs", "fuse.rclone", "fuse.gvfsd-fuse"] {
            assert!(is_network_fstype(t), "{t}");
        }
        for t in ["ext4", "btrfs", "xfs", "tmpfs", "fuseblk", "fuse.portal", "fuse.xdg-document-portal"] {
            assert!(!is_network_fstype(t), "{t}");
        }
    }

    #[test]
    fn a_local_file_is_read_but_not_kept() {
        let cache = FileCache::with_policy(never);
        let p = temp("local.bin", b"abc");
        assert_eq!(&*cache.read(&p).unwrap(), b"abc");
        assert!(cache.kept().is_empty());
    }

    #[test]
    fn a_network_file_is_kept_and_served_from_memory() {
        let cache = FileCache::with_policy(always);
        let p = temp("net.bin", b"abc");
        let first = cache.read(&p).unwrap();
        let second = cache.read(&p).unwrap();
        // The same allocation: the second read did not touch the file
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(cache.kept(), vec![p]);
    }

    #[test]
    fn a_changed_file_is_read_again() {
        let cache = FileCache::with_policy(always);
        let p = temp("changed.bin", b"abc");
        assert_eq!(&*cache.read(&p).unwrap(), b"abc");
        std::fs::write(&p, b"abcdef").unwrap();
        assert_eq!(&*cache.read(&p).unwrap(), b"abcdef");
        assert_eq!(cache.kept().len(), 1);
    }

    #[test]
    fn only_three_files_stay_and_the_oldest_goes_first() {
        let cache = FileCache::with_policy(always);
        let paths: Vec<PathBuf> = (0..4).map(|i| temp(&format!("n{i}.bin"), &[i as u8; 8])).collect();
        for p in &paths {
            cache.read(p).unwrap();
        }
        assert_eq!(cache.kept(), paths[1..].to_vec());
    }

    #[test]
    fn keep_only_forgets_the_others() {
        let cache = FileCache::with_policy(always);
        let a = temp("ka.bin", b"a");
        let b = temp("kb.bin", b"b");
        cache.read(&a).unwrap();
        cache.read(&b).unwrap();
        cache.keep_only(std::slice::from_ref(&b));
        assert_eq!(cache.kept(), vec![b]);
    }

    #[test]
    fn a_missing_file_is_an_error_and_nothing_stays_loading() {
        let cache = FileCache::with_policy(always);
        let p = std::env::temp_dir().join("tornade-file-cache-missing.bin");
        assert!(cache.read(&p).is_err());
        assert!(cache.inner.lock().unwrap().loading.is_empty());
    }
}

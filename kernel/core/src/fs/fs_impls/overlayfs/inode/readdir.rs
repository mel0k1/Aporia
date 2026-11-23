use hashbrown::HashMap;

use super::{OverlayInode, xattr::OverlayXattrType};
use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::real::read_all_dirents,
        utils::DirentVisitor,
        vfs::path::{Dentry, is_dot_or_dotdot},
    },
    prelude::*,
};

pub(super) struct ReaddirEntry {
    name: String,
    type_: InodeType,
    layer_index: usize,
    overlay_ino: u64,
    is_whiteout: bool,
    is_origin_preserved: bool,
}

impl ReaddirEntry {
    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn type_(&self) -> InodeType {
        self.type_
    }

    pub(super) fn overlay_ino(&self) -> u64 {
        self.overlay_ino
    }

    pub(super) fn is_whiteout(&self) -> bool {
        self.is_whiteout
    }

    fn new(name: &str, type_: InodeType, layer_index: usize, overlay_ino: u64) -> Self {
        Self {
            name: String::from(name),
            type_,
            layer_index,
            overlay_ino,
            is_whiteout: false,
            is_origin_preserved: false,
        }
    }
}

pub(super) struct ReaddirCache {
    entries: Box<[ReaddirEntry]>,
}

impl ReaddirCache {
    pub(super) fn entries(&self) -> &[ReaddirEntry] {
        &self.entries
    }
}

pub(super) type OverlayInodeLockPayload = Option<Arc<ReaddirCache>>;

pub(super) type OverlayInodeLockGuard<'a> = MutexGuard<'a, OverlayInodeLockPayload>;

impl OverlayInode {
    pub(super) fn current_readdir_cache(&self) -> Result<Arc<ReaddirCache>> {
        let mut slot = self.lock();
        if let Some(current) = slot.as_ref() {
            return Ok(current.clone());
        }
        let entries = self.build_merged_entries()?;
        let has_origin_preserved = entries.iter().any(|entry| entry.is_origin_preserved);
        let snapshot = Arc::new(ReaddirCache {
            entries: entries.into_boxed_slice(),
        });
        *slot = Some(snapshot.clone());
        if !has_origin_preserved && let Some(upper_real) = self.upper.get() {
            let namespace = self.fs_arc().policy().xattr_namespace();
            let _ = OverlayXattrType::Impure.remove_from(upper_real.dentry(), namespace);
        }
        Ok(snapshot)
    }

    pub(super) fn is_empty_dir(&self, payload: &OverlayInodeLockPayload) -> Result<bool> {
        let rebuilt;
        let entries: &[ReaddirEntry] = match payload {
            Some(snapshot) => snapshot.entries(),
            None => {
                rebuilt = self.build_merged_entries()?;
                &rebuilt
            }
        };
        Ok(!entries.iter().any(|entry| !entry.is_whiteout))
    }

    fn build_merged_entries(&self) -> Result<Vec<ReaddirEntry>> {
        let fs = self.fs_arc();
        let namespace = fs.policy().xattr_namespace();
        let defer_all = !fs.identity().is_same_fs_passthrough();
        let dir_is_impure = match self.upper.get() {
            Some(upper) => OverlayXattrType::Impure.is_positive_on(upper.dentry(), namespace)?,
            None => false,
        };

        let mut entries: Vec<ReaddirEntry> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        let layers = self.lowers.iter().rev().chain(self.upper.get());
        for layer in layers {
            let layer_index = layer.layer_index();
            let mut layer_entries = read_layer_entries(layer.dentry(), layer_index)?;
            for entry in &mut layer_entries {
                if defer_all || (layer_index == 0 && dir_is_impure) {
                    entry.overlay_ino = 0;
                }
            }
            fold_layer_entries(&mut entries, &mut index, layer_entries);
        }
        drop(index);
        self.resolve_entry_whiteouts(&mut entries)?;
        if dir_is_impure {
            self.resolve_entry_origins(&mut entries)?;
        }
        Ok(entries)
    }

    fn resolve_entry_whiteouts(&self, entries: &mut [ReaddirEntry]) -> Result<()> {
        let fs = self.fs_arc();
        for entry in entries {
            if !matches!(entry.type_, InodeType::CharDevice | InodeType::File) {
                continue;
            }
            let layer_dir = self.layer_dir(entry.layer_index)?;
            let child = match layer_dir.as_dir_dentry_or_err()?.lookup_child(&entry.name) {
                Ok(child) => child,
                Err(err) if err.error() == Errno::ENOENT => continue,
                Err(err) => return Err(err),
            };
            entry.is_whiteout = fs.is_whiteout(&child)?;
        }
        Ok(())
    }

    fn resolve_entry_origins(&self, entries: &mut [ReaddirEntry]) -> Result<()> {
        let fs = self.fs_arc();
        for entry in entries {
            if entry.layer_index != 0 {
                entry.is_origin_preserved = true;
                continue;
            }
            let layer_dir = self.layer_dir(entry.layer_index)?;
            let child = match layer_dir.as_dir_dentry_or_err()?.lookup_child(&entry.name) {
                Ok(child) => child,
                Err(err) if err.error() == Errno::ENOENT => continue,
                Err(err) => return Err(err),
            };
            entry.is_origin_preserved = fs.origin_of(&child).is_some();
        }
        Ok(())
    }

    fn layer_dir(&self, layer_index: usize) -> Result<&Arc<Dentry>> {
        let layer_dentry = if layer_index == 0 {
            self.upper.get().map(|upper| upper.dentry())
        } else {
            self.lowers
                .iter()
                .find(|layer| layer.layer_index() == layer_index)
                .map(|lower| lower.dentry())
        };
        layer_dentry.ok_or_else(|| {
            errno!(
                EIO,
                "the merged entry names a layer this directory does not have",
            )
        })
    }
}

struct LayerEntryVisitor {
    layer_index: usize,
    entries: Vec<ReaddirEntry>,
}

impl DirentVisitor for LayerEntryVisitor {
    fn visit(&mut self, name: &str, ino: u64, type_: InodeType, _offset: usize) -> Result<()> {
        self.entries
            .push(ReaddirEntry::new(name, type_, self.layer_index, ino));
        Ok(())
    }
}

fn read_layer_entries(layer_dir: &Arc<Dentry>, layer_index: usize) -> Result<Vec<ReaddirEntry>> {
    let dir = layer_dir.as_dir_dentry_or_err()?;
    let mut visitor = LayerEntryVisitor {
        layer_index,
        entries: Vec::new(),
    };
    read_all_dirents(dir.inode(), &mut visitor)?;
    let mut entries = visitor.entries;
    entries.retain(|entry| !is_dot_or_dotdot(&entry.name));
    Ok(entries)
}

fn fold_layer_entries(
    entries: &mut Vec<ReaddirEntry>,
    index: &mut HashMap<String, usize>,
    layer: Vec<ReaddirEntry>,
) {
    for source in layer {
        let position = if let Some(position) = index.get(source.name()).copied() {
            position
        } else {
            let position = entries.len();
            entries.push(ReaddirEntry::new(
                source.name(),
                InodeType::Unknown,
                usize::MAX,
                0,
            ));
            index.insert(String::from(source.name()), position);
            position
        };
        entries[position] = source;
    }
}

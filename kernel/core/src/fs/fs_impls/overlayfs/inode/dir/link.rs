use crate::{
    fs::{
        fs_impls::overlayfs::{
            fs::OverlayFs,
            inode::{Lookup, NegativeLookup, OverlayInode, OverlayXattrType},
        },
        vfs::{inode::RenameMode, path::Dentry},
    },
    prelude::*,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LinkTarget {
    Whiteout,
    Vacant,
}

impl OverlayInode {
    pub(in super::super) fn link_impl(
        &self,
        self_dentry: &Dentry,
        old_dentry: &Dentry,
        name: &str,
    ) -> Result<()> {
        let fs = self.fs_arc();
        self.writable_real_object(self_dentry)?;
        let source = self.promote_link_source(old_dentry)?;
        let source_upper = source.writable_upper().dentry();
        let mut dir_guard = self.lock();
        let target = self.classify_link_target(&fs, name)?;
        self.set_link_impure_marker(&fs, source_upper)?;
        self.link_name(&fs, source_upper, name, target)?;
        *dir_guard = None;
        Ok(())
    }

    fn promote_link_source(&self, old_dentry: &Dentry) -> Result<Arc<OverlayInode>> {
        let source = Arc::downcast::<OverlayInode>(old_dentry.inode().clone())
            .expect("the link source is not an overlay inode");
        source.writable_real_object(old_dentry)?;
        Ok(source)
    }

    fn classify_link_target(&self, fs: &OverlayFs, name: &str) -> Result<LinkTarget> {
        let target_lookup = fs.lookup(self, name)?;
        if matches!(target_lookup, Lookup::Positive(_)) {
            return Err(errno!(ESTALE));
        }
        if matches!(
            target_lookup,
            Lookup::Negative(NegativeLookup::HiddenByWhiteout)
        ) {
            return Ok(LinkTarget::Whiteout);
        }
        Ok(LinkTarget::Vacant)
    }

    fn set_link_impure_marker(&self, fs: &OverlayFs, source_upper: &Arc<Dentry>) -> Result<()> {
        let source_is_origin_backed = fs.origin_of(source_upper).is_some();
        if source_is_origin_backed {
            let upper_parent = self.writable_upper();
            if !fs.policy().can_store_private_xattr() {
                return Err(errno!(
                    EOPNOTSUPP,
                    "the upper filesystem cannot store the impure marker required for a link"
                ));
            }
            if !OverlayXattrType::Impure
                .is_positive_on(upper_parent.dentry(), fs.policy().xattr_namespace())?
            {
                OverlayXattrType::Impure.set_value_on(
                    upper_parent.dentry(),
                    fs.policy().xattr_namespace(),
                    None,
                )?;
            }
        }
        Ok(())
    }

    fn link_name(
        &self,
        fs: &OverlayFs,
        source_upper: &Arc<Dentry>,
        name: &str,
        target: LinkTarget,
    ) -> Result<()> {
        match target {
            LinkTarget::Whiteout => self.link_over_whiteout(fs, name, source_upper),
            LinkTarget::Vacant => self
                .writable_upper()
                .dentry()
                .as_dir_dentry_or_err()?
                .link(source_upper, name),
        }
    }

    fn link_over_whiteout(&self, fs: &OverlayFs, name: &str, source: &Arc<Dentry>) -> Result<()> {
        let upper_parent = self.writable_upper();
        let upper_workdir = fs.upper_workdir_inuse();
        let temp = upper_workdir.create_workdir_link_temp(name, source)?;
        temp.publish(upper_parent.dentry(), name, RenameMode::Replace)
    }
}

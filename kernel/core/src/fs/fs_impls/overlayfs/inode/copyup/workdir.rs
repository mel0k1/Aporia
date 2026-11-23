use crate::{
    fs::{
        fs_impls::overlayfs::{fs::mount::inuse::UpperWorkdirInuse, inode::CreateOp},
        vfs::{
            inode::{Inode, RenameMode},
            path::Dentry,
        },
    },
    prelude::*,
};

pub(in super::super) struct WorkdirTemp {
    name: String,
    dentry: Arc<Dentry>,
    workspace: Arc<Dentry>,
    consumed: bool,
}

impl WorkdirTemp {
    pub(in super::super) fn name(&self) -> &str {
        &self.name
    }

    pub(in super::super) fn inode(&self) -> &Arc<dyn Inode> {
        self.dentry.inode()
    }

    pub(in super::super) fn dentry(&self) -> &Arc<Dentry> {
        &self.dentry
    }

    pub(in super::super) fn mark_consumed(&mut self) {
        self.consumed = true;
    }

    pub(in super::super) fn publish(
        mut self,
        upper_parent: &Arc<Dentry>,
        name: &str,
        mode: RenameMode,
    ) -> Result<()> {
        self.workspace.as_dir_dentry_or_err()?.rename(
            &self.name,
            &upper_parent.as_dir_dentry_or_err()?,
            name,
            mode,
        )?;
        self.consumed = true;
        Ok(())
    }
}

impl Drop for WorkdirTemp {
    fn drop(&mut self) {
        if self.consumed {
            return;
        }
        self.consumed = true;
        let dir = match self.workspace.as_dir_dentry_or_err() {
            Ok(dir) => dir,
            Err(_) => return,
        };
        let _ = if self.dentry.inode().type_().is_directory() {
            dir.rmdir(&self.name)
        } else {
            dir.unlink(&self.name)
        };
    }
}

impl UpperWorkdirInuse {
    pub(in super::super) fn create_workdir_temp(
        &self,
        target_name: &str,
        op: CreateOp<'_>,
    ) -> Result<WorkdirTemp> {
        let workspace = self.workdir_workspace()?;
        let name = self.next_temp_name(target_name);
        let dentry = op.create_child_in(workspace, &name)?;
        Ok(WorkdirTemp {
            name,
            dentry,
            workspace: Arc::clone(workspace),
            consumed: false,
        })
    }

    pub(in super::super) fn create_workdir_link_temp(
        &self,
        target_name: &str,
        source: &Arc<Dentry>,
    ) -> Result<WorkdirTemp> {
        let workspace = self.workdir_workspace()?;
        let name = self.next_temp_name(target_name);
        let dir = workspace.as_dir_dentry_or_err()?;
        dir.link(source, &name)?;
        let dentry = dir.lookup_child(&name)?;
        Ok(WorkdirTemp {
            name,
            dentry,
            workspace: Arc::clone(workspace),
            consumed: false,
        })
    }
}

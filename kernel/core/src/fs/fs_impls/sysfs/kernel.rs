use alloc::sync::Arc;

use aster_systree::{
    BranchNodeFields, Error, Result, SysAttrSetBuilder, SysBranchNode, SysNode, SysPerms, SysStr,
    inherit_sys_branch_node,
};
use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};
use spin::Once;

pub(super) fn register(config_obj: Arc<dyn SysNode>) -> crate::prelude::Result<()> {
    KERNEL_SYS_NODE_ROOT.get().unwrap().add_child(config_obj)?;
    Ok(())
}

pub(super) fn unregister(name: &str) -> crate::prelude::Result<()> {
    let _ = KERNEL_SYS_NODE_ROOT.get().unwrap().remove_child(name)?;
    Ok(())
}

pub(super) fn init() {
    KERNEL_SYS_NODE_ROOT.call_once(|| {
        let singleton = KernelSysNodeRoot::new();
        super::systree_singleton()
            .root()
            .add_child(singleton.clone())
            .unwrap();

        singleton
    });
}

static KERNEL_SYS_NODE_ROOT: Once<Arc<KernelSysNodeRoot>> = Once::new();

#[derive(Debug)]
pub(crate) struct KernelSysNodeRoot {
    fields: BranchNodeFields<dyn SysNode, Self>,
}

#[inherit_methods(from = "self.fields")]
impl KernelSysNodeRoot {
    fn new() -> Arc<Self> {
        let name = SysStr::from("kernel");

        let mut builder = SysAttrSetBuilder::new();
        builder.add(
            SysStr::from("cpu_byteorder"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("address_bits"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("uevent_seqnum"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        let attrs = builder
            .build()
            .expect("Failed to build kernel attribute set");

        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            KernelSysNodeRoot { fields }
        })
    }

    fn add_child(&self, new_child: Arc<dyn SysNode>) -> Result<()>;
}

inherit_sys_branch_node!(KernelSysNodeRoot, fields, {
    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        match name {
            "cpu_byteorder" => {
                let value = if cfg!(target_endian = "little") {
                    "little"
                } else {
                    "big"
                };
                let mut printer = VmPrinter::new_skip(writer, offset);
                writeln!(printer, "{}", value)?;
                Ok(printer.bytes_written())
            }
            "address_bits" => {
                let mut printer = VmPrinter::new_skip(writer, offset);
                writeln!(printer, "{}", usize::BITS)?;
                Ok(printer.bytes_written())
            }
            "uevent_seqnum" => {
                let mut printer = VmPrinter::new_skip(writer, offset);
                writeln!(printer, "{}", aster_device::uevent::current_seqnum())?;
                Ok(printer.bytes_written())
            }
            _ => Err(Error::AttributeError),
        }
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

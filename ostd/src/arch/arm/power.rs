use core::arch::asm;

use fdt::node::FdtNode;
use spin::Once;

use crate::{arch::boot::DEVICE_TREE, power::ExitCode};

#[derive(Debug)]
enum PsciMethod {
    Smc,
    Hvc,
}

impl PsciMethod {
    fn parse(node: &FdtNode) -> Option<Self> {
        let method = node.property("method")?.as_str()?;
        match method {
            "smc" => Some(Self::Smc),
            "hvc" => Some(Self::Hvc),
            _ => None,
        }
    }
}

static PSCI_METHOD: Once<PsciMethod> = Once::new();

#[repr(u32)]
enum PsciFunc {
    SystemOff = 0x8400_0008,
    SystemReset = 0x8400_0009,
}

pub fn try_poweroff(_code: ExitCode) {
    psci_call(PsciFunc::SystemOff);
}

pub fn try_restart(_code: ExitCode) {
    psci_call(PsciFunc::SystemReset);
}

fn psci_call(func_id: PsciFunc) {
    let Some(psci_method) = PSCI_METHOD.get() else {
        return;
    };

    macro_rules! asm_smccc {
        ($insn:literal, $func_id:expr) => {
            asm!(
                $insn,
                in("w0") $func_id,
                lateout("x0") _, out("x1") _, out("x2") _, out("x3") _, out("x4") _, out("x5") _,
                out("x6") _, out("x7") _, out("x8") _, out("x9") _, out("x10") _, out("x11") _,
                out("x12") _, out("x13") _, out("x14") _, out("x15") _, out("x16") _, out("x17") _,
            )
        }
    }

    unsafe {
        match psci_method {
            PsciMethod::Smc => asm_smccc!("smc #0", func_id as u32),
            PsciMethod::Hvc => asm_smccc!("hvc #0", func_id as u32),
        }
    }
}

pub(super) fn init() {
    const FDT_COMPATIBLE: &[&str] = &["arm,psci-0.2", "arm,psci-1.0"];

    let device_tree = DEVICE_TREE.get().unwrap();

    let Some(psci_node) = device_tree.find_compatible(FDT_COMPATIBLE) else {
        return;
    };
    let Some(psci_method) = PsciMethod::parse(&psci_node) else {
        crate::warn!("PSCI node has an invalid method");
        return;
    };

    crate::info!("PSCI detected: {:?}", psci_method);

    PSCI_METHOD.call_once(|| psci_method);
}

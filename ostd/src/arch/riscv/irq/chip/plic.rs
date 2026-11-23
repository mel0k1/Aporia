use alloc::{boxed::Box, collections::btree_map::BTreeMap, vec::Vec};

use bit_field::BitField;
use fdt::Fdt;

use crate::{
    Error, Result,
    io::{IoMem, IoMemAllocatorBuilder, Sensitive},
    irq::IrqLine,
};

pub(super) struct Plic {
    phandle: u32,
    io_mem: IoMem<Sensitive>,
    hart_to_target_mapping: BTreeMap<u32, u32>,
    interrupt_number_mappings: Box<[Option<u8>]>,
}

impl Plic {
    pub(super) fn phandle(&self) -> u32 {
        self.phandle
    }

    fn num_interrupt_sources(&self) -> u32 {
        self.interrupt_number_mappings.len() as u32
    }

    pub(super) fn set_priority(&mut self, interrupt_source: u32, priority: u32) {
        assert!(interrupt_source > 0 && interrupt_source < self.num_interrupt_sources());

        let offset = Self::PRIORITY_OFFSET + 4 * interrupt_source as usize;
        unsafe { self.io_mem.write_once(offset, &priority) };
    }

    pub(super) fn is_pending(&self, interrupt_source: u32) -> bool {
        assert!(interrupt_source > 0 && interrupt_source < self.num_interrupt_sources());

        let word_index = interrupt_source as usize / 32;
        let bit_index = interrupt_source as usize % 32;
        let offset = Self::PENDING_OFFSET + 4 * word_index;
        let value = unsafe { self.io_mem.read_once::<u32>(offset) };
        (value >> bit_index) & 1 != 0
    }

    pub(super) fn set_interrupt_enabled(&self, hart: u32, interrupt_source: u32, enabled: bool) {
        assert!(interrupt_source > 0 && interrupt_source < self.num_interrupt_sources());

        let target = *self.hart_to_target_mapping.get(&hart).unwrap();
        let word_index = interrupt_source as usize / 32;
        let bit_index = interrupt_source as usize % 32;
        let offset = Self::ENABLE_OFFSET + Self::ENABLE_STRIDE * target as usize + 4 * word_index;

        let mut value = unsafe { self.io_mem.read_once::<u32>(offset) };
        value.set_bit_to(bit_index, enabled);
        unsafe { self.io_mem.write_once(offset, &value) };
    }

    pub(super) fn set_threshold(&self, hart: u32, threshold: u32) {
        let target = *self.hart_to_target_mapping.get(&hart).unwrap();
        let offset = Self::THRESHOLD_OFFSET + Self::THRESHOLD_STRIDE * target as usize;

        unsafe { self.io_mem.write_once(offset, &threshold) };
    }

    pub(super) fn claim_interrupt(&self, hart: u32) -> u32 {
        let target = *self.hart_to_target_mapping.get(&hart).unwrap();
        let offset = Self::CLAIM_COMPLETE_OFFSET + Self::CLAIM_COMPLETE_STRIDE * target as usize;

        unsafe { self.io_mem.read_once::<u32>(offset) }
    }

    pub(super) fn complete_interrupt(&self, hart: u32, interrupt_source: u32) {
        assert!(interrupt_source > 0 && interrupt_source < self.num_interrupt_sources());

        let target = *self.hart_to_target_mapping.get(&hart).unwrap();
        let offset = Self::CLAIM_COMPLETE_OFFSET + Self::CLAIM_COMPLETE_STRIDE * target as usize;

        unsafe { self.io_mem.write_once(offset, &interrupt_source) };
    }

    pub(super) fn managed_harts(&self) -> impl Iterator<Item = u32> + use<'_> {
        self.hart_to_target_mapping.keys().copied()
    }

    pub(super) fn init(&mut self) {
        for interrupt_source in 1..self.num_interrupt_sources() {
            self.set_priority(interrupt_source, 0);
        }

        for hart in self.managed_harts() {
            for interrupt_source in 1..self.num_interrupt_sources() {
                self.set_interrupt_enabled(hart, interrupt_source, false);
            }

            self.set_threshold(hart, 0);

            while let irq_num = self.claim_interrupt(hart)
                && irq_num != 0
            {
                self.complete_interrupt(hart, irq_num);
            }
        }
    }
}

impl Plic {
    const PRIORITY_OFFSET: usize = 0x0;
    const PENDING_OFFSET: usize = 0x1000;
    const ENABLE_OFFSET: usize = 0x2000;
    const THRESHOLD_OFFSET: usize = 0x200000;
    const CLAIM_COMPLETE_OFFSET: usize = 0x200004;

    const ENABLE_STRIDE: usize = 0x80;
    const THRESHOLD_STRIDE: usize = 0x1000;
    const CLAIM_COMPLETE_STRIDE: usize = 0x1000;

    const POSSIBLE_DT_COMPATIBLES: [&str; 4] = [
        "andestech,nceplic100",
        "sifive,plic-1.0.0",
        "thead,c900-plic",
        "riscv,plic0",
    ];
}

impl Plic {
    pub(super) fn from_fdt(fdt: &Fdt<'_>, io_mem_builder: &IoMemAllocatorBuilder) -> Vec<Self> {
        fdt.all_nodes()
            .filter(|node| {
                node.compatible().is_some_and(|compatibles| {
                    compatibles
                        .all()
                        .any(|compatible| Self::POSSIBLE_DT_COMPATIBLES.contains(&compatible))
                })
            })
            .map(|plic_node| {
                let phandle = plic_node
                    .property("phandle")
                    .and_then(|prop| prop.as_usize())
                    .expect("Failed to read 'phandle' property from PLIC node")
                    as u32;

                let range = {
                    let region = plic_node
                        .reg()
                        .expect("Failed to read 'reg' property from PLIC node")
                        .next()
                        .expect("Empty 'reg' property found in PLIC node");

                    let base = region.starting_address as usize;
                    let size = region
                        .size
                        .expect("Incomplete 'reg' property found in PLIC node");

                    base..(base + size)
                };

                let num_interrupt_sources = plic_node
                    .property("riscv,ndev")
                    .and_then(|prop| prop.as_usize())
                    .expect("Failed to read 'riscv,ndev' property from PLIC node")
                    as u32;

                let hart_to_target_mapping = {
                    let cpu_nodes = fdt
                        .find_node("/cpus")
                        .expect("Failed to find '/cpus' node")
                        .children()
                        .filter(|node| node.name.split('@').next().unwrap() == "cpu")
                        .collect::<Vec<_>>();

                    plic_node
                        .property("interrupts-extended")
                        .expect("Failed to read 'interrupts-extended' property from PLIC node")
                        .value
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .enumerate()
                        .filter_map(|(idx, chunk)| {
                            let target = [
                                u32::from_be_bytes(chunk[0..4].try_into().unwrap()),
                                u32::from_be_bytes(chunk[4..8].try_into().unwrap()),
                            ];

                            if target[1] != 0x09 {
                                return None;
                            }

                            let hart_id = cpu_nodes.iter().find_map(|node| {
                                node.children()
                                    .find(|child| {
                                        child
                                            .compatible()
                                            .is_some_and(|c| c.all().any(|s| s == "riscv,cpu-intc"))
                                            && child.property("phandle").is_some_and(|ph| {
                                                ph.as_usize().unwrap() as u32 == target[0]
                                            })
                                    })
                                    .and_then(|_| node.property("reg").and_then(|p| p.as_usize()))
                            })?;

                            Some((hart_id as u32, idx as u32))
                        })
                        .collect()
                };

                Self {
                    phandle,
                    io_mem: io_mem_builder.reserve(range, crate::mm::CachePolicy::Uncacheable),
                    hart_to_target_mapping,
                    interrupt_number_mappings: (0..num_interrupt_sources)
                        .map(|_| None)
                        .collect::<Vec<Option<u8>>>()
                        .into_boxed_slice(),
                }
            })
            .collect()
    }

    pub(super) fn interrupt_number_mapping(&self, interrupt_source: u32) -> Option<u8> {
        self.interrupt_number_mappings[interrupt_source as usize]
    }

    pub(super) fn map_interrupt_source_to(
        &mut self,
        interrupt_source: u32,
        irq_line: &IrqLine,
    ) -> Result<()> {
        assert_ne!(interrupt_source, 0);

        match self.interrupt_number_mappings[interrupt_source as usize] {
            None => {
                self.interrupt_number_mappings[interrupt_source as usize] = Some(irq_line.num());
                Ok(())
            }
            Some(_) => Err(Error::AccessDenied),
        }
    }

    pub(super) fn unmap_interrupt_source(&mut self, interrupt_source: u32) {
        assert_ne!(interrupt_source, 0);

        self.interrupt_number_mappings[interrupt_source as usize] = None;
    }
}
